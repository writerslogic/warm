// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::keys::{nonce_for, Cipher};
use super::store::OpenGuard;
use super::{INDEX_ENTRY_BYTES, MAX_BLOCKS, MAX_BLOCK_BYTES, TAG_LEN};
use crate::core::durable_file::{
    open_regular, open_regular_read_write_create, positioned_read, positioned_write,
};
use anyhow::{anyhow, ensure, Result};
use std::collections::HashMap;
use std::fs::File;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

const HDR_MAGIC: &[u8; 8] = b"WRMSEG01";
const HDR_FIXED: usize = 80;

/// Source of nonce counters for one segment. Every range handed out is durably recorded as
/// used before it is returned, so a counter is never issued twice under one key.
pub(crate) trait NonceSource: Send + Sync {
    fn reserve(&self, segment: u64) -> Result<Range<u64>>;
}

#[derive(Clone, Copy)]
pub(crate) struct Entry {
    offset: u64,
    len: u32,
    counter: u64,
    tag: [u8; TAG_LEN],
}

pub(crate) struct SegmentCtx {
    pub dir: PathBuf,
    pub store_id: [u8; 16],
    pub encrypted: bool,
    pub id: u64,
    pub gen: u32,
    pub block_bytes: u32,
    pub cipher: Arc<Cipher>,
    /// Persisted nonce high-water mark: every counter on disk is below it.
    pub nonce_hwm: u64,
    pub nonces: Arc<dyn NonceSource>,
    pub guard: Option<OpenGuard>,
}

pub(crate) fn segment_paths(dir: &Path, id: u64, gen: u32) -> (PathBuf, PathBuf) {
    (
        dir.join(format!("seg-{id:016x}-g{gen}.dat")),
        dir.join(format!("seg-{id:016x}-g{gen}.hdr")),
    )
}

fn block_aad(store_id: &[u8; 16], segment: u64, index: u64) -> [u8; 37] {
    let mut aad = [0u8; 37];
    aad[0] = 1;
    aad[1..17].copy_from_slice(store_id);
    aad[17..25].copy_from_slice(&segment.to_le_bytes());
    aad[25..33].copy_from_slice(&index.to_le_bytes());
    aad[33..37].copy_from_slice(&super::FORMAT_VERSION.to_le_bytes());
    aad
}

fn header_aad(body: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(body.len() + 1);
    aad.push(2);
    aad.extend_from_slice(body);
    aad
}

struct Cache {
    cap: usize,
    used: usize,
    tick: u64,
    map: HashMap<u64, (u64, Arc<[u8]>)>,
}

/// Read access to one committed segment. Blocks decrypt on first access.
pub struct SegmentReader {
    pub(crate) id: u64,
    store_id: [u8; 16],
    block_bytes: u32,
    cipher: Arc<Cipher>,
    dat: File,
    pub(crate) entries: Vec<Entry>,
    committed_len: u64,
    garbage: u64,
    cache: Mutex<Cache>,
    _guard: Option<OpenGuard>,
}

impl SegmentReader {
    pub fn segment_id(&self) -> u64 {
        self.id
    }

    pub fn block_count(&self) -> u64 {
        self.entries.len() as u64
    }

    pub fn block_bytes(&self) -> u32 {
        self.block_bytes
    }

    /// Bytes of the data file referenced by live blocks.
    pub fn live_bytes(&self) -> u64 {
        self.committed_len - self.garbage
    }

    /// Bounds the decrypted-block cache; evicts down to the new bound immediately.
    pub fn set_cache_bytes(&self, cap: usize) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.cap = cap;
        evict(&mut cache);
    }

    pub fn read_block(&self, index: u64) -> Result<Arc<[u8]>> {
        let entry = usize::try_from(index)
            .ok()
            .and_then(|i| self.entries.get(i))
            .ok_or_else(|| anyhow!("block index {index} out of range"))?;
        {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.tick += 1;
            let tick = cache.tick;
            if let Some((t, block)) = cache.map.get_mut(&index) {
                *t = tick;
                return Ok(block.clone());
            }
        }
        let mut ciphertext = vec![0u8; entry.len as usize];
        positioned_read(&self.dat, &mut ciphertext, entry.offset)?;
        let aad = block_aad(&self.store_id, self.id, index);
        let plain: Arc<[u8]> = self
            .cipher
            .open(entry.counter, &aad, &ciphertext, &entry.tag)
            .map_err(|_| anyhow!("block {index} of segment {} failed authentication", self.id))?
            .into();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if plain.len() <= cache.cap {
            cache.tick += 1;
            let tick = cache.tick;
            cache.used += plain.len();
            if let Some((_, old)) = cache.map.insert(index, (tick, plain.clone())) {
                cache.used -= old.len();
            }
            evict(&mut cache);
        }
        Ok(plain)
    }

    /// SHA-256 over the segment's identity and every block tag, in block order. Each tag
    /// authenticates its block's ciphertext, index and segment, so the root commits to the
    /// whole segment state and is what a provenance layer signs.
    #[cfg(feature = "security")]
    pub fn root_hash(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"warm-sealed-root-v1");
        hasher.update(self.store_id);
        hasher.update(self.id.to_le_bytes());
        hasher.update((self.entries.len() as u64).to_le_bytes());
        for entry in &self.entries {
            hasher.update(entry.tag);
        }
        hasher.finalize().into()
    }

    fn invalidate(&self, index: u64) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, old)) = cache.map.remove(&index) {
            cache.used -= old.len();
        }
    }
}

fn evict(cache: &mut Cache) {
    while cache.used > cache.cap {
        let Some((&victim, _)) = cache.map.iter().min_by_key(|(_, (tick, _))| *tick) else {
            break;
        };
        if let Some((_, block)) = cache.map.remove(&victim) {
            cache.used -= block.len();
        }
    }
}

fn encode_header(
    ctx: &SegmentCtx,
    entries: &[Entry],
    committed_len: u64,
    garbage: u64,
    counter: u64,
) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(HDR_FIXED + entries.len() * INDEX_ENTRY_BYTES + TAG_LEN);
    out.extend_from_slice(HDR_MAGIC);
    out.extend_from_slice(&super::FORMAT_VERSION.to_le_bytes());
    out.push(u8::from(ctx.encrypted));
    out.extend_from_slice(&[0; 3]);
    out.extend_from_slice(&ctx.store_id);
    out.extend_from_slice(&ctx.id.to_le_bytes());
    out.extend_from_slice(&ctx.gen.to_le_bytes());
    out.extend_from_slice(&ctx.block_bytes.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    out.extend_from_slice(&committed_len.to_le_bytes());
    out.extend_from_slice(&garbage.to_le_bytes());
    out.extend_from_slice(&counter.to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.offset.to_le_bytes());
        out.extend_from_slice(&e.len.to_le_bytes());
        out.extend_from_slice(&e.counter.to_le_bytes());
        out.extend_from_slice(&e.tag);
    }
    let sealed = ctx
        .cipher
        .seal_with_nonce(nonce_for(counter), &header_aad(&out), &[])?;
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().expect("slice length is fixed by the caller"))
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().expect("slice length is fixed by the caller"))
}

struct Parsed {
    entries: Vec<Entry>,
    committed_len: u64,
    garbage: u64,
}

fn parse_header(ctx: &SegmentCtx, path: &Path) -> Result<Parsed> {
    let mut file = open_regular(path, false)
        .map_err(|e| anyhow!("segment header unreadable: {}", e.kind()))?;
    let size = file.metadata()?.len();
    ensure!(
        size >= (HDR_FIXED + TAG_LEN) as u64
            && size <= (HDR_FIXED + TAG_LEN) as u64 + MAX_BLOCKS * INDEX_ENTRY_BYTES as u64,
        "segment header has an invalid length"
    );
    let mut bytes = vec![0u8; size as usize];
    std::io::Read::read_exact(&mut file, &mut bytes)?;
    let body_len = bytes.len() - TAG_LEN;
    let (body, tag) = bytes.split_at(body_len);
    ensure!(
        &body[..8] == HDR_MAGIC && le_u32(&body[8..12]) == super::FORMAT_VERSION,
        "segment header magic or version mismatch"
    );
    let counter = le_u64(&body[72..80]);
    ctx.cipher
        .open_with_nonce(nonce_for(counter), &header_aad(body), tag)
        .map_err(|_| anyhow!("segment header failed authentication"))?;
    ensure!(
        body[12] == u8::from(ctx.encrypted)
            && body[16..32] == ctx.store_id
            && le_u64(&body[32..40]) == ctx.id
            && le_u32(&body[40..44]) == ctx.gen
            && le_u32(&body[44..48]) == ctx.block_bytes,
        "segment header does not belong to this store"
    );
    let count = le_u64(&body[48..56]);
    let committed_len = le_u64(&body[56..64]);
    let garbage = le_u64(&body[64..72]);
    ensure!(
        count <= MAX_BLOCKS && (body.len() - HDR_FIXED) as u64 == count * INDEX_ENTRY_BYTES as u64,
        "segment header block count does not match its length"
    );
    ensure!(
        garbage <= committed_len,
        "segment header garbage exceeds data length"
    );
    ensure!(
        counter < ctx.nonce_hwm,
        "segment header nonce is beyond the key table mark"
    );
    let mut entries = Vec::with_capacity(count as usize);
    for raw in body[HDR_FIXED..].chunks_exact(INDEX_ENTRY_BYTES) {
        let entry = Entry {
            offset: le_u64(&raw[0..8]),
            len: le_u32(&raw[8..12]),
            counter: le_u64(&raw[12..20]),
            tag: raw[20..36].try_into().expect("16-byte tag"),
        };
        ensure!(
            entry.len <= ctx.block_bytes
                && entry.len <= MAX_BLOCK_BYTES
                && entry
                    .offset
                    .checked_add(u64::from(entry.len))
                    .is_some_and(|end| end <= committed_len)
                && entry.counter < ctx.nonce_hwm,
            "segment index entry out of bounds"
        );
        entries.push(entry);
    }
    Ok(Parsed {
        entries,
        committed_len,
        garbage,
    })
}

fn open_core(ctx: &mut SegmentCtx, write: bool) -> Result<(SegmentReader, u64)> {
    let (dat_path, hdr_path) = segment_paths(&ctx.dir, ctx.id, ctx.gen);
    let parsed = if hdr_path.symlink_metadata().is_err() {
        // A header is written by the first commit, which takes a counter first: no counter
        // ever issued means no data was ever written and the segment is simply empty.
        ensure!(ctx.nonce_hwm <= 1, "segment header is missing");
        Parsed {
            entries: Vec::new(),
            committed_len: 0,
            garbage: 0,
        }
    } else {
        parse_header(ctx, &hdr_path)?
    };
    let dat = if write || (parsed.committed_len == 0 && dat_path.symlink_metadata().is_err()) {
        open_regular_read_write_create(&dat_path)?
    } else {
        open_regular(&dat_path, false)?
    };
    ensure!(
        dat.metadata()?.len() >= parsed.committed_len,
        "segment data is shorter than its committed length"
    );
    if write {
        dat.set_len(parsed.committed_len)?;
    }
    let reader = SegmentReader {
        id: ctx.id,
        store_id: ctx.store_id,
        block_bytes: ctx.block_bytes,
        cipher: ctx.cipher.clone(),
        dat,
        entries: parsed.entries,
        committed_len: parsed.committed_len,
        garbage: parsed.garbage,
        cache: Mutex::new(Cache {
            cap: 64 * 1024 * 1024,
            used: 0,
            tick: 0,
            map: HashMap::new(),
        }),
        _guard: ctx.guard.take(),
    };
    Ok((reader, parsed.committed_len))
}

pub(crate) fn open_reader(mut ctx: SegmentCtx) -> Result<SegmentReader> {
    Ok(open_core(&mut ctx, false)?.0)
}

/// Appends and rewrites blocks; nothing is visible to a reopened segment until [`commit`].
///
/// [`commit`]: SegmentWriter::commit
pub struct SegmentWriter {
    reader: SegmentReader,
    ctx: SegmentCtx,
    tail: u64,
    next: u64,
    limit: u64,
}

impl std::ops::Deref for SegmentWriter {
    type Target = SegmentReader;
    fn deref(&self) -> &SegmentReader {
        &self.reader
    }
}

pub(crate) fn open_writer(mut ctx: SegmentCtx) -> Result<SegmentWriter> {
    let (reader, tail) = open_core(&mut ctx, true)?;
    Ok(SegmentWriter {
        reader,
        ctx,
        tail,
        next: 0,
        limit: 0,
    })
}

impl SegmentWriter {
    fn take_counter(&mut self) -> Result<u64> {
        if self.next == self.limit {
            let range = self.ctx.nonces.reserve(self.ctx.id)?;
            ensure!(range.start < range.end, "empty nonce reservation");
            self.next = range.start;
            self.limit = range.end;
        }
        let counter = self.next;
        self.next += 1;
        Ok(counter)
    }

    fn seal_block(&mut self, index: u64, data: &[u8]) -> Result<Entry> {
        ensure!(
            data.len() <= self.ctx.block_bytes as usize,
            "block of {} bytes exceeds the segment block size {}",
            data.len(),
            self.ctx.block_bytes
        );
        let counter = self.take_counter()?;
        let aad = block_aad(&self.ctx.store_id, self.ctx.id, index);
        let (ciphertext, tag) = self.ctx.cipher.seal(counter, &aad, data)?;
        positioned_write(&self.reader.dat, &ciphertext, self.tail)?;
        let entry = Entry {
            offset: self.tail,
            len: ciphertext.len() as u32,
            counter,
            tag,
        };
        self.tail += u64::from(entry.len);
        Ok(entry)
    }

    /// Seals `data` (at most `block_bytes`) as the next block and returns its index.
    pub fn append_block(&mut self, data: &[u8]) -> Result<u64> {
        ensure!(
            (self.reader.entries.len() as u64) < MAX_BLOCKS,
            "segment is full"
        );
        let index = self.reader.entries.len() as u64;
        let entry = self.seal_block(index, data)?;
        self.reader.entries.push(entry);
        Ok(index)
    }

    /// Replaces block `index`. The new record gets a fresh nonce and is appended; the old
    /// record becomes garbage until the segment is physically rewritten.
    pub fn rewrite_block(&mut self, index: u64, data: &[u8]) -> Result<()> {
        let slot = usize::try_from(index)
            .ok()
            .filter(|i| *i < self.reader.entries.len())
            .ok_or_else(|| anyhow!("block index {index} out of range"))?;
        let entry = self.seal_block(index, data)?;
        self.reader.garbage += u64::from(self.reader.entries[slot].len);
        self.reader.entries[slot] = entry;
        self.reader.invalidate(index);
        Ok(())
    }

    /// Makes appended and rewritten blocks durable and visible: fsync of the data, then an
    /// atomic replacement of the header.
    pub fn commit(&mut self) -> Result<()> {
        self.reader.dat.sync_all()?;
        super::failpoint("segment.before_header")?;
        let counter = self.take_counter()?;
        let bytes = encode_header(
            &self.ctx,
            &self.reader.entries,
            self.tail,
            self.reader.garbage,
            counter,
        )?;
        let (_, hdr_path) = segment_paths(&self.ctx.dir, self.ctx.id, self.ctx.gen);
        super::write_atomic(
            &hdr_path,
            &bytes,
            false,
            ["hdr.before_rename", "hdr.after_rename"],
        )?;
        self.reader.committed_len = self.tail;
        Ok(())
    }
}

impl SegmentReader {
    #[cfg(all(test, feature = "security"))]
    pub(crate) fn counters(&self) -> Vec<u64> {
        self.entries.iter().map(|e| e.counter).collect()
    }
}
