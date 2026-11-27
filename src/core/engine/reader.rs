// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Read-only opens, refresh and close. See `docs/reference/concurrency.md`.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What a reader has applied from the writer's files.
#[derive(Default)]
pub(super) struct ReaderState {
    /// Journal offset where the first unapplied frame starts.
    replayed: AtomicUsize,
    /// The experience store's files as last read.
    experience: crate::core::sync::Mutex<super::super::experience::store::DiskStamp>,
}

/// The outcome of [`WarmCore::refresh_in_place`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Nothing was committed since the last read.
    Unchanged,
    /// New commits were applied to this engine.
    Advanced,
    /// The writer compacted: this engine reads a generation that is gone, and the caller must
    /// open a new one (see [`WarmCore::refresh`], which does).
    Reopen,
}

impl WarmCore {
    /// Opens an existing store for reading while another engine may hold it for writing.
    ///
    /// Takes no lock and writes nothing: the engine reads the writer's files where they are.
    /// Queries see the journal up to its last complete, checksum-valid frame; [`Self::refresh`]
    /// moves to a newer one by reading only what was appended. Every mutation returns
    /// [`crate::ReadOnlyError`]. The store must already exist. Provenance, signing and audit
    /// are the writer's: a reader opens none of them.
    pub fn open_read_only(
        dimensions: u32,
        storage_path: Option<String>,
        config: Option<WarmConfig>,
    ) -> Result<Self> {
        // A compaction that publishes a generation between this open reading `CURRENT` and
        // reading the generation it names leaves a file missing for an instant; opening again
        // finds the new one. A real fault fails the same way every time and is returned.
        const ATTEMPTS: u32 = 4;
        let mut attempt = 0;
        loop {
            match Self::open_engine(dimensions, storage_path.clone(), config.clone(), true) {
                Err(error) if attempt + 1 < ATTEMPTS => {
                    tracing::debug!(error = %format!("{error:#}"), "read-only open retried");
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20 * u64::from(attempt)));
                }
                other => return other,
            }
        }
    }

    /// True for an engine opened with [`Self::open_read_only`].
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub(super) fn record_reader_position(&self) {
        if let Some(reader) = &self.reader {
            reader
                .replayed
                .store(self.arena.stats().used_bytes, Ordering::Release);
            *reader.experience.lock() =
                super::super::experience::store::disk_stamp(&self.storage_path);
        }
    }

    /// Whether the writer has committed since this read-only engine last read: a complete new
    /// journal frame, a compaction, or a change to the dense, sparse or experience stores.
    /// Always false for a writer. It reads the new frames to prove them whole, so its cost is
    /// the size of what is new.
    pub fn has_newer_commits(&self) -> Result<bool> {
        let Some(reader) = &self.reader else {
            return Ok(false);
        };
        Ok(self.arena.generation_replaced()?
            // A walk that fails is the writer replacing files under it, or damage: a refresh
            // finds out which.
            || self.arena.has_more().unwrap_or(true)
            || self.dense.reader_is_stale()
            || self.sparse.changed_on_disk()
            || *reader.experience.lock()
                != super::super::experience::store::disk_stamp(&self.storage_path))
    }

    /// Brings this reader up to the writer's latest commit in place, reading only what was
    /// appended since. A compaction replaced the generation this reader holds, which cannot
    /// be followed incrementally: that is [`Refresh::Reopen`], and nothing was changed.
    pub fn refresh_in_place(&self) -> Result<Refresh> {
        let Some(reader) = &self.reader else {
            return Err(anyhow::anyhow!("refresh applies to a read-only engine"));
        };
        let reopen = self.arena.generation_replaced()?;
        if reopen {
            return Ok(Refresh::Reopen);
        }
        let mut changed = false;
        let moved = match self.arena.advance() {
            Ok(moved) => moved,
            // The generation was replaced and its files removed between the check and now.
            Err(error) => {
                return if self.arena.generation_replaced()? {
                    Ok(Refresh::Reopen)
                } else {
                    Err(error)
                };
            }
        };
        if moved {
            let transaction = self.mutation_gate.begin_write();
            let _publication = transaction.publish();
            let from = reader.replayed.load(Ordering::Acquire);
            let shards = self.shards.write();
            // Frames are applied one at a time; an error leaves `replayed` at the last
            // applied frame's end only if it is recorded per frame, so record the end after
            // success and let a retry re-read from the last success.
            let end = self.replay_from(from, &shards)?;
            reader.replayed.store(end, Ordering::Release);
            drop(shards);
            self.rebuild_meaning_indices();
            self.revision.fetch_add(1, Ordering::Release);
            changed = true;
        }
        if self.dense.reader_is_stale() {
            changed |= self.dense.refresh()?;
        }
        if self.sparse.changed_on_disk() {
            self.sparse.reload()?;
            changed = true;
        }
        let stamp = super::super::experience::store::disk_stamp(&self.storage_path);
        if *reader.experience.lock() != stamp {
            self.open_experience()?;
            *reader.experience.lock() = stamp;
            changed = true;
        }
        Ok(if changed {
            Refresh::Advanced
        } else {
            Refresh::Unchanged
        })
    }

    /// Moves a read-only engine to the store's current committed state. Returns false when
    /// nothing was committed since. After a compaction the engine is opened afresh, which
    /// reads the compacted store once; otherwise only the new frames are read. A failure
    /// leaves the engine as it was. A passphrase passed directly in the config is not
    /// retained: reopen with [`Self::open_read_only`] instead.
    pub fn refresh(&mut self) -> Result<bool> {
        match self.refresh_in_place()? {
            Refresh::Unchanged => Ok(false),
            Refresh::Advanced => Ok(true),
            Refresh::Reopen => {
                let fresh = Self::open_read_only(
                    u32::try_from(self.dimensions)?,
                    Some(self.storage_path.display().to_string()),
                    Some(self.config.clone()),
                )?;
                *self = fresh;
                Ok(true)
            }
        }
    }

    /// Flushes a writer's state and releases its lock, so another process can open the store
    /// for writing. A read-only engine just releases its file handles. The engine is consumed:
    /// use after close is a compile error here, and an error in the bindings. The lock is
    /// released even if the flush fails, and the error says what was not persisted.
    pub fn close(self) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        self.flush()
    }

    pub(super) fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            return Err(crate::core::error::ReadOnlyError.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::documents::DocumentInput;

    const DIM: u32 = 256;

    fn doc(id: &str, text: &str) -> DocumentInput {
        serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap()
    }

    fn open(dir: &Path, read_only: bool) -> WarmCore {
        let path = Some(dir.display().to_string());
        let config = Some(WarmConfig::unencrypted());
        if read_only {
            WarmCore::open_read_only(DIM, path, config).unwrap()
        } else {
            WarmCore::new(DIM, path, config).unwrap()
        }
    }

    fn fill(core: &WarmCore, from: usize, count: usize) {
        for n in from..from + count {
            core.memorize_document(doc(
                &format!("d-{n}"),
                &format!("document {n} about {} and {}", n * 7, n * 13),
            ))
            .unwrap();
        }
    }

    /// A reader holds open handles to the journal. A compaction publishes a new generation and
    /// deletes the old one under it: the writer is not refused, and the reader goes on reading
    /// the generation it opened until it refreshes.
    #[test]
    fn a_reader_keeps_its_generation_while_the_writer_compacts_and_deletes_it() {
        let dir = tempfile::tempdir().unwrap();
        let writer = open(dir.path(), false);
        fill(&writer, 0, 30);
        let mut reader = open(dir.path(), true);
        assert_eq!(reader.document_count(), 30);
        let (first_frame, _) = reader.arena.read_frame(0).unwrap();
        let stale_generation = reader.arena.data_dir();

        writer.compact().unwrap();
        assert!(
            reader.arena.generation_replaced().unwrap(),
            "the compaction did not publish a generation"
        );
        let segment = stale_generation.join("seg_0.bin");
        // On Windows the file can stay as a delete-pending name until the reader lets go.
        #[cfg(unix)]
        assert!(!segment.exists(), "the writer left the old segment behind");
        assert_eq!(
            reader.arena.read_frame(0).unwrap().0,
            first_frame,
            "the reader lost the generation it holds"
        );
        assert_eq!(reader.document_count(), 30);
        assert_eq!(reader.refresh_in_place().unwrap(), Refresh::Reopen);

        assert!(reader.refresh().unwrap());
        assert_eq!(reader.document_count(), 30);
        assert!(reader.get_document("d-7").unwrap().is_some());
        assert!(!reader.has_newer_commits().unwrap());

        fill(&writer, 30, 2);
        assert_eq!(reader.refresh_in_place().unwrap(), Refresh::Advanced);
        assert_eq!(reader.document_count(), 32);
        // Once the reader holds nothing of the stale generation, a later compaction clears it.
        drop(reader);
        writer.compact().unwrap();
        assert!(!segment.exists(), "a stale segment outlived every reader");
    }

    /// A scrubbing compaction overwrites the old generation with zeros before unlinking it,
    /// and that reaches a reader's handle: erased bytes are gone from the disk at once, not
    /// when the reader refreshes. The reader's memory holds what it applied until it refreshes.
    #[cfg(unix)]
    #[test]
    fn a_scrubbing_compaction_zeroes_what_a_reader_still_holds_open() {
        let dir = tempfile::tempdir().unwrap();
        let writer = open(dir.path(), false);
        fill(&writer, 0, 10);
        let mut reader = open(dir.path(), true);
        assert!(reader.arena.read_frame(0).is_ok());

        writer.delete_document("d-3").unwrap();
        writer.compact_scrubbing().unwrap();
        assert!(
            reader.arena.read_frame(0).is_err(),
            "scrubbed journal bytes were still readable through a reader's handle"
        );
        assert_eq!(reader.refresh_in_place().unwrap(), Refresh::Reopen);
        assert!(reader.refresh().unwrap());
        assert!(reader.get_document("d-3").unwrap().is_none());
        assert_eq!(reader.document_count(), 9);
    }

    /// The writer must stay free to rename and delete a journal file a reader has open: the
    /// reader holds plain handles opened with every share mode, nothing mapped. On Windows
    /// this is the share-mode contract (`FILE_SHARE_DELETE | READ | WRITE`) the reader relies
    /// on; the same calls succeed on Unix, where an unlinked file stays readable.
    #[test]
    fn a_segment_can_be_renamed_and_deleted_under_a_reader_handle() {
        let dir = tempfile::tempdir().unwrap();
        {
            let writer = open(dir.path(), false);
            fill(&writer, 0, 5);
            writer.flush().unwrap();
        }
        let reader = open(dir.path(), true);
        let (first_frame, _) = reader.arena.read_frame(0).unwrap();
        let segment = reader.arena.data_dir().join("seg_0.bin");
        let moved = segment.with_extension("moved");
        std::fs::rename(&segment, &moved).expect("rename under a reader handle");
        std::fs::remove_file(&moved).expect("delete under a reader handle");
        assert_eq!(reader.arena.read_frame(0).unwrap().0, first_frame);
    }

    fn refresh_cost(existing: usize) -> (u64, u64) {
        let dir = tempfile::tempdir().unwrap();
        let writer = open(dir.path(), false);
        // One transaction per batch keeps the setup quick for the large store.
        for batch in 0..existing / 50 {
            let docs = (0..50)
                .map(|i| {
                    let n = batch * 50 + i;
                    doc(
                        &format!("d-{n}"),
                        &format!("filler {n} {}", "word ".repeat(200)),
                    )
                })
                .collect();
            writer.memorize_documents(docs).unwrap();
        }
        let reader = open(dir.path(), true);
        let journal = reader.arena.stats().used_bytes as u64;
        writer
            .memorize_document(doc("tail", "one new commit"))
            .unwrap();
        let before = reader.arena.bytes_read();
        assert_eq!(reader.refresh_in_place().unwrap(), Refresh::Advanced);
        let cost = reader.arena.bytes_read() - before;
        assert!(reader.get_document("tail").unwrap().is_some());
        (journal, cost)
    }

    /// A refresh reads the frames committed since the last one and nothing else, so its cost
    /// does not grow with the store: the same new commit costs the same bytes beside a store
    /// of 100 documents and one forty times that size.
    #[test]
    fn a_refresh_reads_only_what_was_committed_since() {
        let (small_journal, small_cost) = refresh_cost(100);
        let (large_journal, large_cost) = refresh_cost(4000);
        assert!(
            large_journal > 20 * small_journal,
            "{small_journal} {large_journal}"
        );
        assert!(small_cost > 0);
        assert_eq!(
            small_cost, large_cost,
            "refresh read {small_cost} bytes beside {small_journal} of journal and \
             {large_cost} beside {large_journal}"
        );
    }
}
