// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Private snapshots for read-only engines.
//!
//! A read-only engine never opens the shared store's files for writing and never takes its
//! lock. It copies a consistent cut of the store into `<store>/.hms.readers/<id>/` and runs a
//! normal engine over that copy, so nothing the engine does on open (tail repair, caches,
//! checkpoints) can reach the writer's files.
//!
//! The cut is consistent because of how the store commits:
//!
//! - The journal is append-only. A frame is complete only if its CRC32 matches, and the
//!   writer writes frames in order, so the longest prefix of valid frames in the active
//!   segment is a state the writer committed (or is about to acknowledge). Sealed segments
//!   never change. The copy keeps that prefix only; a frame being written during the copy
//!   is dropped, never half-copied.
//! - Every other file is replaced atomically (temp file, sync, rename) or belongs to a
//!   sub-store with its own recovery. Those files are copied first and listed (path, size,
//!   mtime, inode) before and after; any difference means a writer changed one mid-copy and
//!   the whole snapshot is retried. They are copied before the journal, so they are never
//!   ahead of it; a journal ahead of a derived file is the state recovery after a crash
//!   already handles.
//! - A compaction publishes a new generation by replacing `CURRENT`. The pointer is read
//!   before and after the copy; a change means the copy may mix generations and is retried.

use anyhow::{ensure, Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::storage::{header_words, list_segments, FORMAT_MANIFEST, HEADER_SIZE};
use super::store_lock::{StoreLock, LOCK_FILE};
use super::MAX_RAW_FRAME_SIZE;

/// Directory inside a store that holds reader snapshots; not part of the store's data.
pub(crate) const READERS_DIR: &str = ".hms.readers";
const ARENA_DIR: &str = "vectors_data.bin";
/// Default key files of a store: the engine reads each from the shared store through its
/// config path instead of from the snapshot.
pub(crate) const KEY_FILES: [&str; 3] = ["master.key", "hms_signing.key", "hms_provenance.key"];
const CURRENT: &str = "CURRENT";
const ATTEMPTS: u32 = 8;
/// A snapshot directory younger than this is never reaped: its engine may not have locked it
/// yet.
const REAP_GRACE: Duration = Duration::from_secs(60);

/// The store changed while it was being copied.
#[derive(Debug, thiserror::Error)]
#[error("the store changed while it was being copied: {0}")]
struct Raced(&'static str);

fn is_race(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.is::<Raced>()
            || cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    is_dir: bool,
    len: u64,
    mtime_ns: u128,
    inode: u64,
}

/// Identifies the committed state a snapshot captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    current: Option<String>,
    last_segment: usize,
    committed_end: u64,
    aux: Vec<Entry>,
}

/// A private copy of a store. Dropping it removes the copy, so drop the engine that uses it
/// first.
#[derive(Debug)]
pub(crate) struct Snapshot {
    dir: PathBuf,
    stamp: Stamp,
    source: PathBuf,
    root: PathBuf,
}

impl Snapshot {
    /// The shared store this is a copy of.
    pub(crate) fn source(&self) -> &Path {
        &self.source
    }

    /// The directory the copy lives in.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn stamp(&self) -> &Stamp {
        &self.stamp
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %self.dir.display(), %error, "reader snapshot not removed");
            }
        }
    }
}

/// Copies a consistent cut of `store` into a new directory under `root` (by default
/// `<store>/.hms.readers`).
pub(crate) fn take(store: &Path, root: &Path) -> Result<Snapshot> {
    ensure!(
        store.join("store.json").is_file(),
        "{} is not a WARM store (no store.json); a read-only open cannot create one",
        store.display()
    );
    std::fs::create_dir_all(root)
        .with_context(|| format!("failed to create snapshot directory {}", root.display()))?;
    reap(root);
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = root.join(format!("r-{}-{nanos}-{attempt}", std::process::id()));
        match copy_cut(store, &dir) {
            Ok(stamp) => {
                return Ok(Snapshot {
                    dir,
                    stamp,
                    source: store.to_path_buf(),
                    root: root.to_path_buf(),
                })
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&dir);
                if !is_race(&error) {
                    return Err(error);
                }
                last = Some(error);
                std::thread::sleep(Duration::from_millis(10 << attempt.min(5)));
            }
        }
    }
    Err(last.expect("at least one attempt ran").context(format!(
        "could not take a consistent snapshot of {} after {ATTEMPTS} attempts; \
             the writer is changing it continuously, retry",
        store.display()
    )))
}

/// True when the store has committed anything since `previous` was taken.
pub(crate) fn changed_since(store: &Path, previous: &Stamp) -> Result<bool> {
    let arena = store.join(ARENA_DIR);
    if read_current(&arena)? != previous.current || listing(store)? != previous.aux {
        return Ok(true);
    }
    let live = live_dir(&arena, previous.current.as_deref())?;
    let ids = list_segments(&live)?;
    let last = ids.last().copied().unwrap_or(0);
    if last != previous.last_segment {
        return Ok(true);
    }
    if ids.is_empty() {
        return Ok(previous.committed_end != 0);
    }
    let mut file = File::open(live.join(segment_name(last)))?;
    Ok(walk(&mut file, previous.committed_end, |_| Ok(()))? != previous.committed_end)
}

/// Removes snapshot directories whose reader died. A live reader's engine holds the lock on
/// its directory, so a directory that can be locked is abandoned.
pub(crate) fn reap(root: &Path) {
    reap_older_than(root, REAP_GRACE);
}

fn reap_older_than(root: &Path, grace: Duration) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let young = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_none_or(|age| age < grace);
        if young || !path.is_dir() {
            continue;
        }
        // An abandoned directory has an unlocked lock file, or none.
        if matches!(StoreLock::try_probe(&path), Ok(true)) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn segment_name(id: usize) -> String {
    format!("seg_{id}.bin")
}

fn read_current(arena: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(arena.join(CURRENT)) {
        Ok(name) => Ok(Some(name)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn live_dir(arena: &Path, current: Option<&str>) -> Result<PathBuf> {
    let Some(name) = current else {
        return Ok(arena.to_path_buf());
    };
    ensure!(
        name.starts_with("gen-") && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid arena generation pointer"
    );
    Ok(arena.join(name))
}

fn copy_cut(store: &Path, dir: &Path) -> Result<Stamp> {
    let arena = store.join(ARENA_DIR);
    let current = read_current(&arena)?;
    let aux = listing(store)?;
    std::fs::create_dir_all(dir)?;
    for entry in &aux {
        let target = dir.join(&entry.path);
        if entry.is_dir {
            std::fs::create_dir_all(&target)?;
        } else {
            std::fs::copy(store.join(&entry.path), &target)?;
        }
    }
    if listing(store)? != aux {
        return Err(Raced("a file other than the journal was rewritten").into());
    }
    let (last_segment, committed_end) = copy_arena(&arena, &dir.join(ARENA_DIR), &current)?;
    Ok(Stamp {
        current,
        last_segment,
        committed_end,
        aux,
    })
}

fn copy_arena(src: &Path, dst: &Path, current: &Option<String>) -> Result<(usize, u64)> {
    std::fs::create_dir_all(dst)?;
    if !src.exists() {
        return Ok((0, 0));
    }
    let src_live = live_dir(src, current.as_deref())?;
    let dst_live = live_dir(dst, current.as_deref())?;
    ensure_dir(&src_live)?;
    std::fs::create_dir_all(&dst_live)?;
    for (from, to) in [(src, dst), (&src_live, &dst_live)] {
        match std::fs::copy(from.join(FORMAT_MANIFEST), to.join(FORMAT_MANIFEST)) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let ids = list_segments(&src_live)?;
    ensure!(
        ids.iter().enumerate().all(|(expected, &id)| expected == id),
        "arena segments are not contiguous from 0: {ids:?}"
    );
    let mut last = (0, 0);
    for &id in &ids {
        let from = src_live.join(segment_name(id));
        let to = dst_live.join(segment_name(id));
        if Some(&id) == ids.last() {
            last = (id, copy_committed_prefix(&from, &to)?);
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    if let Some(name) = current {
        std::fs::write(dst.join(CURRENT), name)?;
    }
    if read_current(src)? != *current {
        return Err(Raced("a compaction published a new generation").into());
    }
    Ok(last)
}

fn ensure_dir(path: &Path) -> Result<()> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(Raced("the live generation disappeared").into())
    }
}

fn copy_committed_prefix(src: &Path, dst: &Path) -> Result<u64> {
    let mut from = File::open(src)?;
    let mut to = File::create(dst)?;
    walk(&mut from, 0, |bytes| Ok(to.write_all(bytes)?))
}

/// Reads whole, checksummed frames from `start` and hands each frame's bytes to `emit`.
/// Returns the offset after the last complete frame: an end marker, a zero tail, a short
/// read or a CRC mismatch all end the committed prefix.
fn walk(file: &mut File, start: u64, mut emit: impl FnMut(&[u8]) -> Result<()>) -> Result<u64> {
    file.seek(SeekFrom::Start(start))?;
    let mut at = start;
    let mut payload = Vec::new();
    loop {
        let mut header = [0u8; HEADER_SIZE];
        if read_full(file, &mut header)? < HEADER_SIZE {
            break;
        }
        let Some([crc, raw_len, comp_len, _]) = header_words(&header) else {
            break;
        };
        let (raw_len, comp_len) = (raw_len as usize, comp_len as usize);
        if comp_len == 0 || raw_len == 0 || raw_len > MAX_RAW_FRAME_SIZE || comp_len > raw_len {
            break;
        }
        payload.resize(comp_len, 0);
        if read_full(file, &mut payload)? < comp_len {
            break;
        }
        let mut hasher = crc32fast::Hasher::new();
        if comp_len < raw_len {
            match lz4_flex::block::decompress(&payload, raw_len) {
                Ok(raw) if raw.len() == raw_len => hasher.update(&raw),
                _ => break,
            }
        } else {
            hasher.update(&payload);
        }
        if hasher.finalize() != crc {
            break;
        }
        emit(&header)?;
        emit(&payload)?;
        at += (HEADER_SIZE + comp_len) as u64;
    }
    Ok(at)
}

fn read_full(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Every file and directory of the store except the lock, the journal directory, reader
/// snapshots, the text cache and temporary files, sorted by path.
fn listing(store: &Path) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    list_into(store, Path::new(""), &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn skipped(relative: &Path, name: &str) -> bool {
    name.starts_with(".tmp")
        || (relative.as_os_str().is_empty() && name.starts_with("master.key."))
        || (relative.as_os_str().is_empty()
            && (name == LOCK_FILE
                || name == READERS_DIR
                || name == ARENA_DIR
                // Key material is read from its original file (see `open_read_only_at`), never
                // copied into a directory that is deleted without scrubbing.
                || KEY_FILES.contains(&name)
                // The text segment is a cache the journal rebuilds; its staging copy is too.
                || name.starts_with("documents.text")))
}

fn list_into(root: &Path, relative: &Path, out: &mut Vec<Entry>) -> Result<()> {
    for entry in std::fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if skipped(relative, name) {
            continue;
        }
        let path = relative.join(name);
        let meta = std::fs::symlink_metadata(root.join(&path))?;
        if !(meta.is_dir() || meta.is_file()) {
            continue;
        }
        out.push(Entry {
            path: path.clone(),
            is_dir: meta.is_dir(),
            len: if meta.is_file() { meta.len() } else { 0 },
            // A directory's own times change whenever a temporary file comes and goes in it.
            mtime_ns: if meta.is_file() {
                meta.modified()
                    .ok()
                    .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos())
            } else {
                0
            },
            inode: inode(&meta),
        });
        if meta.is_dir() {
            list_into(root, &path, out)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn inode(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

#[cfg(not(unix))]
fn inode(_: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(payload: &[u8], version: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn the_prefix_stops_at_the_first_frame_that_is_not_complete() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("seg_0.bin");
        let first = frame(b"first committed frame", 0);
        let second = frame(b"second committed frame", 1);
        let torn = frame(b"a frame still being written", 2);
        let mut bytes = [first.clone(), second.clone()].concat();
        let committed = bytes.len() as u64;
        // Header complete, payload only partly written: the CRC cannot match.
        bytes.extend_from_slice(&torn[..torn.len() - 5]);
        bytes.resize(bytes.len() + 4096, 0);
        std::fs::write(&path, &bytes)?;

        let mut file = File::open(&path)?;
        assert_eq!(walk(&mut file, 0, |_| Ok(()))?, committed);
        let copy = dir.path().join("copy.bin");
        assert_eq!(copy_committed_prefix(&path, &copy)?, committed);
        assert_eq!(std::fs::read(&copy)?, [first, second].concat());
        // Resuming from the frontier finds nothing new until the frame completes.
        assert_eq!(
            walk(&mut File::open(&path)?, committed, |_| Ok(()))?,
            committed
        );
        Ok(())
    }

    #[test]
    fn a_zero_filled_segment_has_an_empty_prefix() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("seg_0.bin");
        std::fs::write(&path, vec![0u8; 8192])?;
        assert_eq!(walk(&mut File::open(&path)?, 0, |_| Ok(()))?, 0);
        Ok(())
    }

    #[test]
    fn reaping_removes_abandoned_snapshots_and_keeps_live_ones() -> Result<()> {
        let root = tempfile::tempdir()?;
        let abandoned = root.path().join("r-dead");
        std::fs::create_dir(&abandoned)?;
        std::fs::write(abandoned.join(LOCK_FILE), b"pid=1\n")?;
        let live = root.path().join("r-live");
        let _held = StoreLock::acquire(&live)?;
        reap_older_than(root.path(), Duration::ZERO);
        assert!(!abandoned.exists());
        assert!(live.exists());
        // A directory younger than the grace period is left alone, locked or not.
        let young = root.path().join("r-young");
        std::fs::create_dir(&young)?;
        reap_older_than(root.path(), Duration::from_secs(3600));
        assert!(young.exists());
        Ok(())
    }

    #[test]
    fn a_store_without_a_manifest_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = take(dir.path(), &dir.path().join(READERS_DIR)).unwrap_err();
        assert!(format!("{error:#}").contains("not a WARM store"));
        assert!(!dir.path().join("store.json").exists());
    }

    #[test]
    fn listing_skips_the_lock_the_journal_and_snapshots() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for name in [LOCK_FILE, "store.json", "documents.text", "documents.ckpt"] {
            std::fs::write(dir.path().join(name), b"x")?;
        }
        std::fs::create_dir(dir.path().join(ARENA_DIR))?;
        std::fs::create_dir(dir.path().join(READERS_DIR))?;
        std::fs::create_dir(dir.path().join("dense-vectors"))?;
        std::fs::write(dir.path().join("dense-vectors").join("rows.bin"), b"y")?;
        std::fs::write(dir.path().join("dense-vectors").join(".tmpAbc"), b"z")?;
        let names: Vec<String> = listing(dir.path())?
            .iter()
            .map(|e| e.path.display().to_string())
            .collect();
        let expected = ["dense-vectors", "documents.ckpt", "store.json"];
        let nested = Path::new("dense-vectors")
            .join("rows.bin")
            .display()
            .to_string();
        assert_eq!(names.len(), 4, "{names:?}");
        assert!(expected.iter().all(|e| names.iter().any(|n| n == e)));
        assert!(names.contains(&nested));
        Ok(())
    }
}
