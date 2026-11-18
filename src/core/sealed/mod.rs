// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Block AEAD container with per-segment data keys and crypto-erase.
//!
//! A store is a directory holding `keys.tbl` (the key table) and, per segment, a data file
//! `seg-<id>-g<gen>.dat` (concatenated sealed blocks) and a header `seg-<id>-g<gen>.hdr`
//! (block index plus one authentication tag). The header is replaced atomically, so the
//! data file may carry an uncommitted tail after a crash; that tail is ignored and truncated
//! when the segment is next opened for writing.
//!
//! Nonce invariant: under one data key every seal operation (block, rewritten block, header
//! tag) takes a fresh counter from a per-segment sequence. The key table persists a
//! high-water mark `nonce_hwm`; a writer reserves a chunk by raising the mark durably before
//! it uses any counter in the chunk, and a reopened segment resumes at the mark. A counter
//! is therefore never reused, including after a crash that lost uncommitted blocks, and a
//! rewritten block is a new record with a new counter.
//!
//! Plaintext mode uses the same container with a null cipher whose "tag" is a CRC32. It
//! detects corruption and offers no authenticity or confidentiality; it is an explicit
//! opt-out (`WARM_ENCRYPTION=off` or `StoreOptions::mode`).

mod keys;
mod segment;
mod store;

pub use keys::KeySource;
pub use segment::{SegmentReader, SegmentWriter};
pub use store::{default_mode, Mode, SealedStore, StoreOptions};

pub const FORMAT_VERSION: u32 = 1;
pub const DEFAULT_BLOCK_BYTES: u32 = 64 * 1024;
pub const MAX_BLOCK_BYTES: u32 = 16 * 1024 * 1024;
pub const TAG_LEN: usize = 16;
/// Header bytes of one block index entry: offset, length, counter, tag.
pub const INDEX_ENTRY_BYTES: usize = 8 + 4 + 8 + TAG_LEN;
/// Counters reserved per durable high-water update.
pub(crate) const NONCE_CHUNK: u64 = 1024;
pub(crate) const MAX_SEGMENTS: usize = 1 << 20;
pub(crate) const MAX_BLOCKS: u64 = 1 << 24;

pub(crate) fn random_bytes(out: &mut [u8]) -> anyhow::Result<()> {
    use rand::TryRng;
    rand::rngs::SysRng
        .try_fill_bytes(out)
        .map_err(|_| anyhow::anyhow!("operating system randomness unavailable"))
}

#[cfg(test)]
thread_local! {
    static FAIL_AT: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
}

/// Crash-injection point: a test arms one name and the matching call fails.
#[inline]
pub(crate) fn failpoint(_name: &'static str) -> anyhow::Result<()> {
    #[cfg(test)]
    if FAIL_AT.with(|f| *f.borrow()) == Some(_name) {
        anyhow::bail!("injected failure at {_name}");
    }
    Ok(())
}

#[cfg(all(test, feature = "security"))]
pub(crate) fn arm_failpoint(name: Option<&'static str>) {
    FAIL_AT.with(|f| *f.borrow_mut() = name);
}

/// Durable write of `bytes` to `path`: temp file, fsync, rename, fsync of the directory.
pub(crate) fn write_atomic(
    path: &std::path::Path,
    bytes: &[u8],
    private: bool,
    points: [&'static str; 2],
) -> anyhow::Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let tmp = path.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    failpoint(points[0])?;
    std::fs::rename(&tmp, path)?;
    failpoint(points[1])?;
    super::durable_file::sync_directory(dir)
}

#[cfg(all(test, feature = "security"))]
mod tests;
