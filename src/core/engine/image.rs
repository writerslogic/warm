// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Journal checkpoints (`documents.ckpt`, `docs/file-format/CHECKPOINT.md`). A compaction of a
//! documents-only store writes the document store as one file inside the new journal
//! generation instead of document frames, so the generation is the checkpoint plus the frames
//! appended after it. An engine opening a generation with a checkpoint and no frames maps it
//! and answers unfiltered lexical queries from it. Document writes in every encode mode,
//! deferred encodings and deletes go to a store over the checkpoint that holds the documents
//! written since and marks the checkpoint documents they replace or delete dead
//! (`documents::overlay`); frames after the checkpoint are replayed the same way at open, which
//! rebuilds that dead set. A checkpoint holds no vector state, so chunk-vector queries,
//! embedding counts, index maintenance and deferred encoding use the live overlay. Checked document reads also
//! preserve the checkpoint. Operations requiring the complete lexical store rebuild it from
//! the checkpoint and overlay ([`WarmCore::hydrate_documents`]).

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::core::sync::RwLock;
use anyhow::{bail, ensure, Context, Result};

use super::WarmCore;
use crate::core::documents::{
    self, Backing, CheckpointBase, DocumentStore, ServingImage, TextSegment,
};

pub(super) const FILE: &str = "documents.ckpt";
/// An encrypted checkpoint: this magic, the binding words in the clear (generation-name CRC32
/// and vector dimensions, at the plain header's word positions), then the checkpoint sealed
/// with the store key over those 40 bytes as associated data.
const SEALED_MAGIC: &[u8; 8] = b"HMSCKPE\x01";
const SEALED_HEADER: usize = 40;

/// How [`WarmCore::open_checkpoint`] left the document store.
pub(super) enum Opened {
    /// No checkpoint: the caller attaches a text segment and replays the journal.
    Absent,
    /// Served from the mapped checkpoint; the generation holds no frames.
    Served,
    /// Served from the mapped checkpoint; the caller replays the frames after it.
    Overlaid,
}

#[derive(Default)]
pub(super) struct ImageState {
    /// The checkpoint the store is served from while it is not rebuilt (the store holds it
    /// too; this is the lock-free test).
    serving: RwLock<Option<Arc<ServingImage>>>,
    /// Whether a mutation other than a document write, encoding or delete was applied: a
    /// checkpoint holds document state only.
    pub(super) other_state: AtomicBool,
    /// Set once the engine opened completely; only then may closing write a checkpoint.
    opened: AtomicBool,
    /// Set when a fold failed: the engine stops folding and keeps the index in memory.
    fold_failed: AtomicBool,
}

fn word(header: &[u8], index: usize) -> u64 {
    let at = 8 + 8 * index;
    u64::from_le_bytes(header[at..at + 8].try_into().expect("eight bytes"))
}

impl WarmCore {
    /// Runs `capture` on the store while it is served from a checkpoint, between transactions
    /// and excluding a rebuild meanwhile; `None` otherwise or when `capture` declines.
    pub(super) fn capture_from_checkpoint<T>(
        &self,
        capture: impl FnOnce(&DocumentStore) -> Option<T>,
    ) -> Option<T> {
        self.image.serving.read().as_ref()?;
        let _transaction = self.mutation_gate.read();
        let store = self.documents.read();
        if !store.has_base() {
            return None;
        }
        capture(&store)
    }

    /// Whether every mutation of a batch can be applied to a store served from a checkpoint
    /// without rebuilding it: document writes in any encode mode, deferred encodings and
    /// document deletes. A checkpoint holds no vector state (`documents::checkpoint_representable`
    /// and no vector shards), so the chunk vectors, embeddings and pending encodings of the
    /// documents written since are the store's whole vector side, and the store over the
    /// checkpoint keeps them as any store does.
    pub(super) fn checkpoint_accepts(&self, mutations: &[super::Mutation]) -> bool {
        self.image.serving.read().is_some()
            && mutations.iter().all(|mutation| {
                matches!(
                    mutation,
                    super::Mutation::Document(_)
                        | super::Mutation::DeleteDocument { .. }
                        | super::Mutation::EncodeDocument { .. }
                        | super::Mutation::AuditPending(_)
                        | super::Mutation::AuditAcknowledged { .. }
                )
            })
    }

    /// Whether this store may hold checkpoints: schema 4 and later (an older release refuses
    /// the store instead of opening it without its checkpointed documents).
    fn checkpoint_schema(&self) -> bool {
        self.schema >= crate::core::format::CHECKPOINT_SCHEMA_VERSION
    }

    fn fresh_text_segment(&self) -> Result<Arc<TextSegment>> {
        Ok(Arc::new(TextSegment::create(
            &self.storage_path.join("documents.text"),
            #[cfg(feature = "security")]
            self.encryption.clone(),
        )?))
    }

    /// Builds the live store from the checkpoint being served and the store over it (the
    /// checkpoint's live documents, then the ones written since) and stops serving it. The
    /// result holds the same documents, so a writer that installs before or after it reaches
    /// the same state.
    pub(super) fn hydrate_documents(&self) -> Result<()> {
        if self.image.serving.read().is_none() {
            return Ok(());
        }
        let transaction = self.mutation_gate.begin_write();
        if self.image.serving.read().is_none() {
            return Ok(());
        }
        let (image, dead, written) = self
            .documents
            .read()
            .overlay()
            .context("store served from a checkpoint without it")?;
        // The dead documents' postings are left out of the rebuilt lists rather than removed
        // from them one document at a time (each removal re-encodes every list it touches).
        let mut store = image.hydrate_excluding(Some(self.fresh_text_segment()?), &dead)?;
        for id in &dead {
            store.remove_unindexed_scrubbed(id)?;
        }
        store.insert_batch(written)?;
        if let Some(segment) = store.text_segment() {
            segment.drain_scrubs()?;
        }
        let _publication = transaction.publish();
        *self.documents.write() = store;
        *self.image.serving.write() = None;
        Ok(())
    }

    /// Opens the checkpoint of the live generation, if it has one (see [`Opened`]). A
    /// checkpoint is the store's data, so one that fails its checks fails the open.
    pub(super) fn open_checkpoint(&self) -> Result<Opened> {
        let path = self.arena.data_dir().join(FILE);
        if path.symlink_metadata().is_err() {
            return Ok(Opened::Absent);
        }
        ensure!(
            self.checkpoint_schema(),
            "journal generation holds a checkpoint but the store schema predates checkpoints"
        );
        let image = Arc::new(
            self.load_checkpoint(&path)
                .with_context(|| format!("document checkpoint {}", path.display()))?,
        );
        *self.documents.write() = DocumentStore::over(CheckpointBase::new(Arc::clone(&image)));
        *self.image.serving.write() = Some(image);
        Ok(if self.arena.stats().used_bytes == 0 {
            Opened::Served
        } else {
            Opened::Overlaid
        })
    }

    fn generation_crc(name: &str) -> u32 {
        crc32fast::hash(name.as_bytes())
    }

    fn load_checkpoint(&self, path: &Path) -> Result<ServingImage> {
        let mut file = std::fs::File::open(path)?;
        let mut outer = [0u8; SEALED_HEADER];
        file.read_exact(&mut outer)?;
        let generation = Self::generation_crc(&self.arena.checkpoint().0);
        ensure!(
            (word(&outer, 1) as u32, word(&outer, 2)) == (generation, self.dimensions as u64),
            "checkpoint belongs to another generation or dimension count"
        );
        #[cfg(feature = "security")]
        let sealed_key = self.encryption.clone();
        #[cfg(not(feature = "security"))]
        let sealed_key: Option<()> = None;
        let bytes = if &outer[..8] == SEALED_MAGIC {
            #[cfg(feature = "security")]
            {
                let Some(cipher) = &sealed_key else {
                    bail!("sealed checkpoint in a store opened without encryption");
                };
                let mut sealed = Vec::new();
                file.read_to_end(&mut sealed)?;
                Backing::Owned(cipher.decrypt_with_aad(&sealed, &outer)?)
            }
            #[cfg(not(feature = "security"))]
            bail!("sealed checkpoint needs the security feature");
        } else {
            ensure!(
                documents::checkpoint_magic_known(&outer[..8]),
                "not a document checkpoint of this version"
            );
            ensure!(
                sealed_key.is_none(),
                "plaintext checkpoint in an encrypted store"
            );
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(&outer[documents::CHECKPOINT_CRC_START..]);
            let mut buffer = vec![0u8; 1 << 16];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
            file.seek(SeekFrom::Start(0))?;
            let mut header = vec![0u8; documents::CHECKPOINT_HEADER_LEN];
            file.read_exact(&mut header)?;
            ensure!(
                hasher.finalize() == documents::checkpoint_crc(&header),
                "checkpoint checksum mismatch"
            );
            // SAFETY: the store lock keeps other engines out of the store directory, and no
            // engine writes a published checkpoint: compactions write new ones in new
            // generations and remove an old generation only after the switch.
            Backing::Mapped(unsafe { memmap2::Mmap::map(&file)? })
        };
        ensure!(
            bytes.len() >= documents::CHECKPOINT_HEADER_LEN
                && documents::checkpoint_crc(&bytes)
                    == crc32fast::hash(&bytes[documents::CHECKPOINT_CRC_START..])
                && documents::checkpoint_binding(&bytes) == (generation, self.dimensions as u64),
            "checkpoint checksum mismatch"
        );
        Ok(ServingImage::new(bytes)?
            .with_text_cache(self.config.document_checkpoint.text_cache_bytes))
    }

    /// The sealed checkpoint of `store` for the generation directory `generation`, when this
    /// store takes checkpoints and holds only lexical-only documents; called under the
    /// maintenance lease of a compaction snapshot, which checked the vector shards are empty
    /// (encoded chunks also fill them, and a checkpoint does not hold them).
    pub(super) fn encode_checkpoint(
        &self,
        store: &DocumentStore,
        generation: &str,
    ) -> Result<Option<Vec<u8>>> {
        if !self.checkpoint_schema()
            || !self.config.document_checkpoint.enabled
            || self.image.other_state.load(Ordering::Acquire)
        {
            return Ok(None);
        }
        let Some(image) = documents::encode_checkpoint(
            store,
            Self::generation_crc(generation),
            self.dimensions,
            self.config.document_checkpoint.compress_text,
        )?
        else {
            return Ok(None);
        };
        self.seal_checkpoint(image).map(Some)
    }

    /// Whether a compaction writes this store's checkpoint by streaming it
    /// ([`documents::merge_checkpoint`]) instead of encoding it in memory: a plaintext store
    /// that takes checkpoints. Sealed checkpoints are one AEAD message and stay in memory until
    /// the sealed block container carries them.
    #[cfg(feature = "security")]
    pub(super) fn streams_checkpoints(&self) -> bool {
        self.encryption.is_none() && self.streams_plain()
    }

    #[cfg(not(feature = "security"))]
    pub(super) fn streams_checkpoints(&self) -> bool {
        self.streams_plain()
    }

    /// A streamed merge writes inline text; compressed text needs the whole stream to train
    /// its dictionary, so a store that compresses text encodes its checkpoint in memory.
    fn streams_plain(&self) -> bool {
        self.checkpoint_schema()
            && self.config.document_checkpoint.enabled
            && !self.image.other_state.load(Ordering::Acquire)
    }

    /// Streams the checkpoint of the live store into the staging generation directory.
    pub(super) fn stream_checkpoint(
        &self,
        store: &DocumentStore,
        staging: &Path,
        generation: &str,
    ) -> Result<Option<u64>> {
        documents::merge_checkpoint(
            store,
            Self::generation_crc(generation),
            self.dimensions,
            &staging.join(FILE),
        )
    }

    /// After a streamed compaction published a checkpoint holding the whole document store:
    /// replaces the live store (and with it the index in memory) by the store over that
    /// checkpoint, the state a reopen would reach.
    pub(super) fn serve_published_checkpoint(&self) -> Result<()> {
        let path = self.arena.data_dir().join(FILE);
        let image = Arc::new(
            self.load_checkpoint(&path)
                .with_context(|| format!("document checkpoint {}", path.display()))?,
        );
        image.release_pages();
        *self.documents.write() = DocumentStore::over(CheckpointBase::new(Arc::clone(&image)));
        *self.image.serving.write() = Some(image);
        Ok(())
    }

    /// Folds the live index into the checkpoint when its estimated size reaches
    /// `document_checkpoint.index_memory_budget`; called after each committed document batch.
    /// A fold that fails leaves the store as it was (everything is committed) and ends folding
    /// for this engine.
    pub(super) fn fold_if_over_budget(&self) {
        let budget = self.config.document_checkpoint.index_memory_budget;
        if budget == 0
            || !self.image.opened.load(Ordering::Acquire)
            || self.image.fold_failed.load(Ordering::Acquire)
            || !self.streams_checkpoints()
        {
            return;
        }
        let fraction = self.config.document_checkpoint.index_fold_fraction;
        let (resident, base_bytes) = {
            let documents = self.documents.read();
            (documents.resident_estimate(), documents.base_bytes())
        };
        // Rounding the fractional threshold down is harmless: it only folds a byte early.
        let threshold = budget.max((fraction.max(0.0) * base_bytes as f64) as usize);
        if resident < threshold {
            return;
        }
        if let Err(error) = self.compact() {
            self.image.fold_failed.store(true, Ordering::Release);
            tracing::warn!(%error, "folding the document index into the checkpoint failed");
        }
    }

    /// Whether a compaction now would write a checkpoint: checkpoints enabled, only document
    /// state, no vector shards, and every chunk lexical-only.
    pub(super) fn takes_checkpoint(&self) -> bool {
        if !self.checkpoint_schema()
            || !self.config.document_checkpoint.enabled
            || self.image.other_state.load(Ordering::Acquire)
        {
            return false;
        }
        // One lock at a time: commits take the shards before the store.
        self.shards.read().count() == 0
            && documents::checkpoint_representable(&self.documents.read())
    }

    /// Writes a checkpoint when it is due: the engine fully opened, a compaction would write
    /// one ([`Self::takes_checkpoint`]), and at least `document_checkpoint.min_journal_bytes` of
    /// frames in the live generation (the frames a checkpoint would fold in).
    pub(super) fn checkpoint_if_due(&self) -> Result<()> {
        if !self.image.opened.load(Ordering::Acquire) {
            return Ok(());
        }
        let used = self.arena.stats().used_bytes;
        if used == 0 || used < self.config.document_checkpoint.min_journal_bytes {
            return Ok(());
        }
        if !self.takes_checkpoint() {
            return Ok(());
        }
        self.compact()
    }

    /// At close: when the live generation is a checkpoint with no frames after it, every
    /// document's text is in the checkpoint, so the text segment cache is cut to its header.
    pub(super) fn release_text_cache(&self) {
        if !self.config.document_checkpoint.compress_text
            || !self.image.opened.load(Ordering::Acquire)
            || self.arena.stats().used_bytes != 0
            || self.arena.data_dir().join(FILE).symlink_metadata().is_err()
        {
            return;
        }
        match documents::TextSegment::release(&self.storage_path.join("documents.text")) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => tracing::warn!(%error, "text cache not released at close"),
        }
    }

    #[cfg(feature = "security")]
    fn seal_checkpoint(&self, mut image: Vec<u8>) -> Result<Vec<u8>> {
        use zeroize::Zeroize;
        let Some(cipher) = &self.encryption else {
            return Ok(image);
        };
        let mut outer = image[..SEALED_HEADER].to_vec();
        outer[..8].copy_from_slice(SEALED_MAGIC);
        outer[8..16].fill(0);
        let sealed = cipher.encrypt_with_aad(&image, &outer);
        image.zeroize();
        outer.extend_from_slice(&sealed?);
        Ok(outer)
    }

    #[cfg(not(feature = "security"))]
    fn seal_checkpoint(&self, image: Vec<u8>) -> Result<Vec<u8>> {
        Ok(image)
    }

    /// Whether unfiltered lexical queries are being answered from a mapped checkpoint (the
    /// store opened from one and nothing has needed the rebuilt store since).
    #[doc(hidden)]
    pub fn serves_from_checkpoint(&self) -> bool {
        self.image.serving.read().is_some()
    }

    pub(super) fn mark_opened(&self) {
        self.image.opened.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::documents::SearchOptions;
    use crate::core::WarmConfig;

    fn checkpoint_files(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.extend(checkpoint_files(&path));
            } else if path.file_name().is_some_and(|name| name == FILE) {
                found.push(path);
            }
        }
        found
    }

    /// A store whose checkpoint is version 1 (flat postings) opens from it, answers exactly
    /// as it did from version 2, and its next compaction writes version 2.
    #[test]
    fn version_1_checkpoint_serves_unchanged_and_compacts_to_version_3() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let mut config = WarmConfig::default();
        config.document_checkpoint.min_journal_bytes = 0;
        let options = documents::IngestOptions {
            encode: documents::EncodeMode::Lexical,
            ..Default::default()
        };
        let words = ["alpha", "beta", "gamma", "delta", "evidence"];
        let queries = ["evidence", "alpha beta", "gamma evidence delta"];
        let answers = |core: &WarmCore| -> Result<Vec<String>> {
            queries
                .iter()
                .map(|q| {
                    let hits = core.search_documents(q, &SearchOptions::default())?;
                    Ok(serde_json::to_string(&hits)?)
                })
                .collect()
        };
        let expected = {
            let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
            let documents = (0..300)
                .map(|i| {
                    let text: Vec<&str> = (0..1 + i % 7).map(|j| words[(i * 3 + j) % 5]).collect();
                    serde_json::from_value(serde_json::json!({
                        "id": format!("d{i:03}"), "text": text.join(" ")
                    }))
                    .unwrap()
                })
                .collect();
            core.memorize_documents_with(documents, &options)?;
            core.compact()?;
            drop(core);
            let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
            assert!(core.serves_from_checkpoint());
            answers(&core)?
        };
        let [file] = checkpoint_files(directory.path()).try_into().unwrap();
        let bytes = std::fs::read(&file)?;
        assert_eq!(&bytes[..8], b"HMSCKP\x00\x03");
        let generation = file
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let dimensions = documents::checkpoint_binding(&bytes).1 as usize;
        let store = ServingImage::new(Backing::Owned(bytes))?.hydrate(None)?;
        let flat = documents::encode_checkpoint_v1(
            &store,
            WarmCore::generation_crc(generation),
            dimensions,
        )?
        .expect("lexical-only store");
        std::fs::write(&file, flat)?;

        let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
        assert!(core.serves_from_checkpoint());
        assert_eq!(answers(&core)?, expected);
        core.compact()?;
        drop(core);
        let [file] = checkpoint_files(directory.path()).try_into().unwrap();
        assert_eq!(&std::fs::read(&file)?[..8], b"HMSCKP\x00\x03");
        let core = WarmCore::new(256, Some(path), Some(config))?;
        assert!(core.serves_from_checkpoint());
        assert_eq!(answers(&core)?, expected);
        Ok(())
    }

    /// Chunk text is in compressed blocks of the checkpoint: it reads back byte for byte (hits,
    /// records, text of a document written since), the text segment cache does not keep a
    /// second copy at close, and a checkpoint that holds the text inline still opens, serves
    /// the same answers and compacts to the blocks.
    #[test]
    fn compressed_text_round_trips_and_inline_text_checkpoints_still_open() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let mut config = WarmConfig::default();
        config.document_checkpoint.min_journal_bytes = 0;
        config.document_checkpoint.compress_text = true;
        let options = documents::IngestOptions {
            encode: documents::EncodeMode::Lexical,
            ..Default::default()
        };
        let words = ["alpha", "beta", "gamma", "delta", "evidence", "caf\u{e9}"];
        let text_of = |i: usize| -> String {
            (0..2 + i % 40)
                .map(|j| words[(i * 3 + j * j) % words.len()])
                .collect::<Vec<_>>()
                .join(" ")
        };
        let queries = [
            "evidence",
            "alpha beta",
            "gamma evidence delta",
            "caf\u{e9}",
        ];
        let answers = |core: &WarmCore| -> Result<Vec<String>> {
            queries
                .iter()
                .map(|q| {
                    let hits = core.search_documents(q, &SearchOptions::default())?;
                    Ok(serde_json::to_string(&hits)?)
                })
                .collect()
        };
        let feature_word = |bytes: &[u8]| u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let expected = {
            let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
            let documents = (0..600)
                .map(|i| {
                    serde_json::from_value(serde_json::json!({
                        "id": format!("d{i:03}"), "text": text_of(i)
                    }))
                    .unwrap()
                })
                .collect();
            core.memorize_documents_with(documents, &options)?;
            drop(core);
            let text_cache = std::fs::metadata(directory.path().join("documents.text"))?.len();
            assert_eq!(
                text_cache, 8,
                "a closed checkpointed store keeps no text cache"
            );
            let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
            assert!(core.serves_from_checkpoint());
            let mut checked = 0;
            for q in queries {
                for hit in core.search_documents(q, &SearchOptions::default())? {
                    let i: usize = hit.document_id[1..].parse()?;
                    assert_eq!(hit.text.as_deref(), Some(text_of(i).as_str()), "{q}");
                    checked += 1;
                }
            }
            assert!(checked >= 20, "{checked} hits read back");
            answers(&core)?
        };
        let [file] = checkpoint_files(directory.path()).try_into().unwrap();
        let bytes = std::fs::read(&file)?;
        assert_eq!(feature_word(&bytes), 1, "text blocks feature");
        let generation = file
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let dimensions = documents::checkpoint_binding(&bytes).1 as usize;
        let store = ServingImage::new(Backing::Owned(bytes.clone()))?.hydrate(None)?;
        let inline = documents::encode_checkpoint(
            &store,
            WarmCore::generation_crc(generation),
            dimensions,
            false,
        )?
        .expect("lexical-only store");
        assert_eq!(feature_word(&inline), 0);
        assert!(
            inline.len() > bytes.len(),
            "the blocks are smaller than inline text"
        );
        std::fs::write(&file, &inline)?;

        let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
        assert!(core.serves_from_checkpoint());
        assert_eq!(answers(&core)?, expected);
        core.compact()?;
        drop(core);
        let [file] = checkpoint_files(directory.path()).try_into().unwrap();
        assert_eq!(feature_word(&std::fs::read(&file)?), 1);
        for budget in [0, 1, 64 << 20] {
            let mut config = config.clone();
            config.document_checkpoint.text_cache_bytes = budget;
            let core = WarmCore::new(256, Some(path.clone()), Some(config))?;
            assert!(core.serves_from_checkpoint());
            assert_eq!(answers(&core)?, expected, "text cache of {budget} bytes");
            assert_eq!(
                answers(&core)?,
                expected,
                "second read, cache of {budget} bytes"
            );
        }
        Ok(())
    }

    /// Erased text does not survive in a compressed block or in the dictionary sampled from
    /// the stream: a scrubbing compaction rebuilds the text blocks from the live documents.
    #[test]
    fn purged_text_is_absent_from_blocks_and_dictionary() -> Result<()> {
        const MARKER: &str = "quokkapurgemarker";
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let mut config = WarmConfig::default();
        config.document_checkpoint.min_journal_bytes = 0;
        config.document_checkpoint.compress_text = true;
        let options = documents::IngestOptions {
            encode: documents::EncodeMode::Lexical,
            ..Default::default()
        };
        let filler = "common words repeat across the corpus so the dictionary samples them ";
        let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
        let documents = (0..400)
            .map(|i| {
                let text = if i % 20 == 3 {
                    format!("{MARKER} {}", filler.repeat(1 + i % 5))
                } else {
                    format!("document {i} {}", filler.repeat(1 + i % 5))
                };
                serde_json::from_value(serde_json::json!({"id": format!("d{i:03}"), "text": text}))
                    .unwrap()
            })
            .collect();
        core.memorize_documents_with(documents, &options)?;
        core.compact()?;
        let stream = |dir: &Path| -> Result<(Vec<u8>, Vec<u8>)> {
            let [file] = checkpoint_files(dir).try_into().unwrap();
            ServingImage::new(Backing::Owned(std::fs::read(file)?))?.text_contents()
        };
        let holds = |bytes: &[u8]| bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes());
        let (dict, text) = stream(directory.path())?;
        assert!(holds(&text), "the marked documents are in the stream");
        assert!(!dict.is_empty());
        for i in (3..400).step_by(20) {
            core.delete_document(&format!("d{i:03}"))?;
        }
        core.compact_scrubbing()?;
        let (dict, text) = stream(directory.path())?;
        assert!(
            !holds(&dict) && !holds(&text),
            "erased text left in the blocks"
        );
        Ok(())
    }

    /// `search_lexical` of a bare word disjunction ranks as `search_documents` of the same
    /// text (ids, scores to the bit, including words that share a stem), is served from the
    /// checkpoint and its overlay without rebuilding the store, and a query it cannot rank that
    /// way (a required clause) still hydrates and agrees afterwards.
    #[test]
    fn search_lexical_ranks_as_search_documents_without_hydrating() -> Result<()> {
        use crate::core::documents::{LexicalOptions, LexicalQuery};
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let mut config = WarmConfig::default();
        config.document_checkpoint.min_journal_bytes = 0;
        let options = documents::IngestOptions {
            encode: documents::EncodeMode::Lexical,
            ..Default::default()
        };
        let words = [
            "alpha", "beta", "gamma", "delta", "evidence", "run", "running", "runs", "runner",
            "sparse",
        ];
        let text = |i: usize| -> String {
            let text: Vec<&str> = (0..1 + i % 9)
                .map(|j| words[(i * 7 + j * j + j) % words.len()])
                .collect();
            text.join(" ")
        };
        {
            let core = WarmCore::new(256, Some(path.clone()), Some(config.clone()))?;
            let documents = (0..400)
                .map(|i| {
                    serde_json::from_value(serde_json::json!({
                        "id": format!("d{i:03}"), "text": text(i)
                    }))
                    .unwrap()
                })
                .collect();
            core.memorize_documents_with(documents, &options)?;
            core.compact()?;
        }
        let core = WarmCore::new(256, Some(path), Some(config))?;
        assert!(core.serves_from_checkpoint());
        let queries = [
            "evidence",
            "alpha beta",
            "running run runs",
            "runs runner evidence",
            "gamma evidence delta beta",
            "absent alpha",
            "absent",
        ];
        let agree = |core: &WarmCore| -> Result<usize> {
            let mut matched = 0;
            for query in queries {
                for k in [1usize, 7, 20, 100] {
                    let lexical = core.search_lexical(
                        &LexicalQuery::parse(query)?,
                        &LexicalOptions {
                            k,
                            ..Default::default()
                        },
                    )?;
                    let documents = core.search_documents(
                        query,
                        &SearchOptions {
                            k: Some(k as u32),
                            candidate_limit: Some(k as u32),
                            semantic_weight: Some(0.0),
                            ..Default::default()
                        },
                    )?;
                    let a: Vec<_> = lexical
                        .hits
                        .iter()
                        .map(|h| (h.document_id.as_str(), h.score.to_bits()))
                        .collect();
                    let b: Vec<_> = documents
                        .iter()
                        .map(|h| (h.document_id.as_str(), h.lexical_score.to_bits()))
                        .collect();
                    assert_eq!(a, b, "query {query:?} k {k}");
                    matched += a.len();
                }
            }
            Ok(matched)
        };
        assert!(agree(&core)? > 100);
        assert!(core.serves_from_checkpoint(), "ranking hydrated the store");
        // Writes since the checkpoint are ranked from the overlay.
        assert!(core.delete_document("d003")?);
        for (id, body) in [
            ("d010", "alpha running runs"),
            ("zz", "evidence evidence alpha"),
        ] {
            core.memorize_document_with(
                serde_json::from_value(serde_json::json!({"id": id, "text": body}))?,
                &options,
            )?;
        }
        assert!(agree(&core)? > 100);
        assert!(
            core.serves_from_checkpoint(),
            "overlay ranking hydrated the store"
        );
        // A required clause is outside the shared path: it hydrates, and the plain path then
        // ranks the hydrated store as it ranked the overlay.
        let required = core.search_lexical(
            &LexicalQuery::parse("+alpha beta")?,
            &LexicalOptions::default(),
        )?;
        assert!(!required.hits.is_empty());
        assert!(!core.serves_from_checkpoint());
        assert!(agree(&core)? > 100);
        Ok(())
    }
}
