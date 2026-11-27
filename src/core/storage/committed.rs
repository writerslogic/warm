// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The committed-end word: how far into the journal a reader may trust bytes.
//!
//! A writer copies a frame into the shared mapping before it syncs the frame, so for a moment
//! the file holds a complete, checksum-valid frame the writer may still disown (a failed sync
//! rolls it back). A reader that scanned it would apply a frame that never committed. The
//! writer therefore records `(segment, end offset)` in a small file next to the segments, only
//! after the frame's sync succeeded, and readers never scan past it.
//!
//! The word is advisory and needs no sync of its own: it only ever names bytes that are
//! already durable, so after a crash it is behind the log or equal to it, and a writer
//! recomputes it at open from the log itself. A store without the file (written before it
//! existed) is read by scanning to the last whole frame, as before. A writer that predates the
//! word does not maintain it: do not open a store with an older writer while newer readers
//! follow it.

use super::*;

pub(super) const COMMITTED_FILE: &str = "committed_end";
const WORD_LEN: usize = 20;

/// What a reader found at the committed-end file.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Committed {
    /// No file: a store from before the word. Scan to the last whole frame.
    Absent,
    /// The file exists but holds no valid word yet (being created, or torn): no progress.
    Unreadable,
    /// The writer has committed the journal up to this segment and offset.
    At(usize, usize),
}

fn encode(segment: usize, offset: usize) -> [u8; WORD_LEN] {
    let mut word = [0u8; WORD_LEN];
    word[0..8].copy_from_slice(&(segment as u64).to_le_bytes());
    word[8..16].copy_from_slice(&(offset as u64).to_le_bytes());
    let mut hasher = Hasher::new();
    hasher.update(&word[..16]);
    word[16..].copy_from_slice(&hasher.finalize().to_le_bytes());
    word
}

pub(super) fn publish(dir: &Path, segment: usize, offset: usize) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(COMMITTED_FILE))
        .context("opening the committed-end word")?;
    super::super::durable_file::positioned_write(&file, &encode(segment, offset), 0)
        .context("writing the committed-end word")
}

pub(super) fn read(dir: &Path) -> Result<Committed> {
    let file = match File::open(dir.join(COMMITTED_FILE)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Committed::Absent),
        Err(error) => return Err(error).context("opening the committed-end word"),
    };
    let mut word = [0u8; WORD_LEN];
    if super::super::durable_file::positioned_read(&file, &mut word, 0).is_err() {
        return Ok(Committed::Unreadable);
    }
    let mut hasher = Hasher::new();
    hasher.update(&word[..16]);
    if hasher.finalize().to_le_bytes() != word[16..] {
        return Ok(Committed::Unreadable);
    }
    let segment = u64::from_le_bytes(word[0..8].try_into().expect("8 bytes"));
    let offset = u64::from_le_bytes(word[8..16].try_into().expect("8 bytes"));
    match (usize::try_from(segment), usize::try_from(offset)) {
        (Ok(segment), Ok(offset)) => Ok(Committed::At(segment, offset)),
        _ => Ok(Committed::Unreadable),
    }
}
