// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! A reader's view of a live journal. The segment files stay where the writer keeps them; the
//! reader holds one open handle per segment and reads frames by position. Nothing is mapped:
//! a mapped file cannot be deleted on Windows, and the writer must stay free to delete and
//! rename. Every handle is opened with all three share modes (Rust's `File::open` does so on
//! Windows), so the writer's rename and delete are never refused for the handle itself.
//!
//! The committed end is the writer's committed-end word (`committed.rs`): a reader never
//! scans past it, because a frame the writer copied but has not yet synced may still be rolled
//! back. Frames are walked from the last known end up to that word. A store from before the
//! word is walked to the first byte that is not a whole, checksum-valid frame instead.

use super::*;

/// Handles to the segments of the generation the reader opened, by segment id.
pub(super) struct ReadView {
    files: RwLock<Vec<Arc<File>>>,
    /// Bytes read from the segments: what a refresh costs, for tests that pin it.
    bytes: std::sync::atomic::AtomicU64,
}

impl ReadView {
    pub(super) fn read(
        &self,
        segment_size: usize,
        global_offset: usize,
        len: usize,
    ) -> Result<Vec<u8>> {
        let segment = global_offset / segment_size;
        let local = global_offset % segment_size;
        ensure!(
            local
                .checked_add(len)
                .is_some_and(|end| end <= segment_size),
            "Read out of bounds: offset {global_offset}, len {len}"
        );
        let file = self
            .files
            .read()
            .get(segment)
            .cloned()
            .ok_or_else(|| anyhow!("read beyond final arena segment"))?;
        self.bytes.fetch_add(len as u64, Ordering::Relaxed);
        let mut bytes = vec![0u8; len];
        super::super::durable_file::positioned_read(&file, &mut bytes, local as u64)
            .with_context(|| format!("reading arena offset {global_offset}"))?;
        Ok(bytes)
    }
}

/// The reader's recorded end does not lie on a frame boundary at or before the writer's
/// committed end (the log was truncated or rewritten under it): it cannot follow in place, and
/// a fresh open reads the log as it now is.
#[derive(Debug)]
pub(crate) struct ReaderDiverged(pub(crate) String);

impl std::fmt::Display for ReaderDiverged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "reader diverged from the journal: {}", self.0)
    }
}

impl std::error::Error for ReaderDiverged {}

enum Scan {
    /// A whole frame: the offset after it.
    Frame(usize),
    /// An end marker, an unwritten tail, or no room for a header.
    End,
    /// Bytes that are neither: a frame still being written, or damage.
    Bad,
}

impl PersistentArena {
    /// Opens the journal at `base` (the store's `vectors_data.bin`) for reading without taking
    /// a lock, creating or writing anything.
    pub(crate) fn open_read_only(base: &Path) -> Result<Self> {
        let data_path = super::super::durable_file::arena_generation(base)?;
        let source = [base.join(FORMAT_MANIFEST), data_path.join(FORMAT_MANIFEST)]
            .into_iter()
            .find(|path| path.exists());
        let (version, segment_size, derived_tables) = match source {
            Some(source) => {
                let (manifest, _) = Self::parse_manifest(&source)?;
                (
                    manifest.version,
                    manifest.segment_size,
                    manifest.derived_tables,
                )
            }
            // A store from before manifests is format 2 (see `validate_or_create_manifest`).
            None => (2, LEGACY_SEGMENT_SIZE, false),
        };
        if let Some(store) = base.parent() {
            super::super::durable_file::verify_manifests(store, base)?;
        }
        let placeholder = ActiveSegment::new(tempfile::tempfile()?)?;
        let arena = Self {
            segment_size,
            format_version: version,
            derived_tables,
            base_path: base.to_path_buf(),
            data_path: RwLock::new(data_path),
            read_segments: RwLock::new(Vec::new()),
            active_segment: RwLock::new(placeholder),
            active_id: AtomicUsize::new(0),
            write_offset: AtomicUsize::new(0),
            active_capacity: AtomicUsize::new(0),
            version_counter: AtomicUsize::new(0),
            rotation_pending: AtomicBool::new(false),
            commit_uncertain: AtomicBool::new(false),
            read_only: AtomicBool::new(true),
            view: Some(ReadView {
                files: RwLock::new(Vec::new()),
                bytes: std::sync::atomic::AtomicU64::new(0),
            }),
        };
        arena.advance()?;
        Ok(arena)
    }

    /// Whether `CURRENT` now names another generation (a compaction published one).
    pub(crate) fn generation_replaced(&self) -> Result<bool> {
        let live = super::super::durable_file::arena_generation(&self.base_path)?;
        Ok(live != *self.data_path.read())
    }

    /// Whether the writer has completed a frame since the last [`Self::advance`].
    pub(crate) fn has_more(&self) -> Result<bool> {
        let (end, _) = self.walk()?;
        Ok(end
            != (
                self.active_id.load(Ordering::Acquire),
                self.write_offset.load(Ordering::Acquire),
            ))
    }

    /// Moves the committed end forward over every whole frame the writer has completed since
    /// the last call; returns whether it moved. The old end stays valid when this fails.
    pub(crate) fn advance(&self) -> Result<bool> {
        let (end, before) = self.walk()?;
        self.active_id.store(end.0, Ordering::Release);
        self.write_offset.store(end.1, Ordering::Release);
        Ok(end != before)
    }

    /// The committed end as the files stand now, and the one last recorded.
    fn walk(&self) -> Result<((usize, usize), (usize, usize))> {
        let view = self.view.as_ref().context("arena is not a reader")?;
        let data_path = self.data_path.read().clone();
        let ids = list_segments(&data_path)?;
        ensure!(
            ids.iter().enumerate().all(|(expected, &id)| expected == id),
            "arena segments are not contiguous from 0: {ids:?}"
        );
        // Reads of applied frames are not held up by a long walk over a new tail.
        let files: Vec<Arc<File>> = {
            let mut files = view.files.write();
            while files.len() < ids.len() {
                let path = data_path.join(format!("seg_{}.bin", files.len()));
                let file = File::open(&path)
                    .with_context(|| format!("opening arena segment {}", path.display()))?;
                files.push(Arc::new(file));
            }
            files.clone()
        };
        let before = (
            self.active_id.load(Ordering::Acquire),
            self.write_offset.load(Ordering::Acquire),
        );
        let (mut segment, mut offset) = before;
        let limit = match committed::read(&data_path)? {
            committed::Committed::Absent => None,
            committed::Committed::Unreadable => Some(before),
            committed::Committed::At(end_segment, end_offset) => {
                if before > (end_segment, end_offset) {
                    return Err(ReaderDiverged(format!(
                        "recorded end {before:?} is past the committed end \
                         {:?}",
                        (end_segment, end_offset)
                    ))
                    .into());
                }
                Some((end_segment, end_offset))
            }
        };
        let mut scratch = Vec::new();
        while let Some(file) = files.get(segment) {
            if limit.is_some_and(|limit| (segment, offset) >= limit) {
                break;
            }
            let sealed = segment + 1 < files.len();
            match self.scan_frame(file, offset, &mut scratch)? {
                Scan::Frame(next) => offset = next,
                Scan::End if sealed => {
                    segment += 1;
                    offset = 0;
                }
                Scan::Bad if limit.is_some() => {
                    // Everything below the committed end is whole frames: a reader that
                    // cannot parse one is not where it thinks it is.
                    return Err(ReaderDiverged(format!(
                        "no whole frame at segment {segment} offset {offset} below the \
                         committed end"
                    ))
                    .into());
                }
                Scan::Bad if sealed => {
                    return Err(anyhow!(
                        "arena segment {segment} has damaged bytes at offset {offset} and a \
                         later segment follows"
                    ));
                }
                Scan::End | Scan::Bad => break,
            }
        }
        Ok(((segment, offset), before))
    }

    fn scan_frame(&self, file: &File, offset: usize, scratch: &mut Vec<u8>) -> Result<Scan> {
        if self.segment_size - offset < HEADER_SIZE {
            return Ok(Scan::End);
        }
        let length = usize::try_from(file.metadata()?.len())?.min(self.segment_size);
        if length.saturating_sub(offset) < HEADER_SIZE {
            return Ok(Scan::End);
        }
        let counter = self.view.as_ref().map(|view| &view.bytes);
        if let Some(counter) = counter {
            counter.fetch_add(HEADER_SIZE as u64, Ordering::Relaxed);
        }
        let mut header = [0u8; HEADER_SIZE];
        super::super::durable_file::positioned_read(file, &mut header, offset as u64)?;
        let Some([crc, raw_len, comp_len, _]) = header_words(&header) else {
            return Ok(Scan::End);
        };
        if raw_len == 0 && comp_len == 0 {
            return Ok(Scan::End);
        }
        let (raw_len, comp_len) = (raw_len as usize, comp_len as usize);
        if comp_len == 0 || comp_len > raw_len || raw_len > MAX_RAW_FRAME_SIZE {
            return Ok(Scan::Bad);
        }
        let end = offset + HEADER_SIZE + comp_len;
        if end > length {
            return Ok(Scan::Bad);
        }
        if let Some(counter) = counter {
            counter.fetch_add(comp_len as u64, Ordering::Relaxed);
        }
        let mut payload = vec![0u8; comp_len];
        super::super::durable_file::positioned_read(
            file,
            &mut payload,
            (offset + HEADER_SIZE) as u64,
        )?;
        let mut hasher = Hasher::new();
        if comp_len < raw_len {
            scratch.resize(raw_len, 0);
            match lz4_flex::block::decompress_into(&payload, &mut scratch[..raw_len]) {
                Ok(written) if written == raw_len => hasher.update(&scratch[..written]),
                _ => return Ok(Scan::Bad),
            }
        } else {
            hasher.update(&payload);
        }
        Ok(if hasher.finalize() == crc {
            Scan::Frame(end)
        } else {
            Scan::Bad
        })
    }
}

#[cfg(test)]
impl PersistentArena {
    /// Bytes this reader has read from the journal segments so far.
    pub(crate) fn bytes_read(&self) -> u64 {
        self.view
            .as_ref()
            .map_or(0, |view| view.bytes.load(Ordering::Relaxed))
    }
}
