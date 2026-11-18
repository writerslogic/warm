// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::keys::{random_key, Cipher, Key, KeySource};
use super::segment::{
    open_reader, open_writer, segment_paths, NonceSource, SegmentCtx, SegmentReader, SegmentWriter,
};
use super::{
    failpoint, random_bytes, write_atomic, FORMAT_VERSION, MAX_BLOCK_BYTES, MAX_SEGMENTS,
    NONCE_CHUNK,
};
use anyhow::{anyhow, bail, ensure, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

const TBL_MAGIC: &[u8; 8] = b"WRMSKT01";
const TBL_HEAD: usize = 52;
const TBL_ENTRY: usize = 84;
const TBL_TRAILER: usize = 28;
const KEYS_FILE: &str = "keys.tbl";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Explicit opt-out: CRC32 integrity only, no confidentiality or authenticity.
    Plain,
    Aes256Gcm,
}

/// Mode for new stores: encrypted whenever `security` is compiled in. `WARM_ENCRYPTION=off`
/// selects plaintext and `on` demands encryption (an error without `security`).
pub fn default_mode() -> Result<Mode> {
    let compiled = cfg!(feature = "security");
    match crate::compatibility::var("WARM_ENCRYPTION")
        .ok()
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") | Some("default") => Ok(if compiled {
            Mode::Aes256Gcm
        } else {
            Mode::Plain
        }),
        Some("on" | "1" | "true" | "aes") => {
            ensure!(
                compiled,
                "WARM_ENCRYPTION=on requires the `security` feature"
            );
            Ok(Mode::Aes256Gcm)
        }
        Some("off" | "0" | "false" | "plain" | "plaintext") => Ok(Mode::Plain),
        Some(_) => bail!("WARM_ENCRYPTION must be on, off or default"),
    }
}

#[derive(Default)]
pub struct StoreOptions {
    /// `None` takes [`default_mode`].
    pub mode: Option<Mode>,
    /// Master key provider for encrypted stores; `None` uses `<dir>/master.key`.
    pub source: Option<KeySource>,
}

#[derive(Clone)]
struct TEntry {
    gen: u32,
    block_bytes: u32,
    hwm: u64,
    wrap_nonce: [u8; 12],
    wrapped: [u8; 48],
}

#[derive(Clone)]
struct Table {
    store_id: [u8; 16],
    epoch: u64,
    mode: Mode,
    next_id: u64,
    entries: BTreeMap<u64, TEntry>,
}

fn wrap_aad(store_id: &[u8; 16], segment: u64) -> [u8; 29] {
    let mut aad = [0u8; 29];
    aad[0] = 3;
    aad[1..17].copy_from_slice(store_id);
    aad[17..25].copy_from_slice(&segment.to_le_bytes());
    aad[25..29].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    aad
}

fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().expect("slice length is fixed by the caller"))
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().expect("slice length is fixed by the caller"))
}

impl Table {
    fn encode(&self, master: Option<&Key>) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(TBL_HEAD + self.entries.len() * TBL_ENTRY + TBL_TRAILER);
        out.extend_from_slice(TBL_MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.push(u8::from(self.mode == Mode::Aes256Gcm));
        out.extend_from_slice(&[0; 3]);
        out.extend_from_slice(&self.store_id);
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&self.next_id.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (id, e) in &self.entries {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&e.gen.to_le_bytes());
            out.extend_from_slice(&e.block_bytes.to_le_bytes());
            out.extend_from_slice(&e.hwm.to_le_bytes());
            out.extend_from_slice(&e.wrap_nonce);
            out.extend_from_slice(&e.wrapped);
        }
        let (cipher, nonce) = match (self.mode, master) {
            (Mode::Plain, _) => (Cipher::Null, [0u8; 12]),
            (Mode::Aes256Gcm, Some(key)) => {
                let mut nonce = [0u8; 12];
                random_bytes(&mut nonce)?;
                (Cipher::aes(key)?, nonce)
            }
            (Mode::Aes256Gcm, None) => bail!("encrypted key table needs the master key"),
        };
        let tag = cipher.seal_with_nonce(nonce, &out, &[])?;
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&tag);
        Ok(out)
    }

    fn peek_mode(bytes: &[u8]) -> Result<Mode> {
        ensure!(
            bytes.len() >= TBL_HEAD + TBL_TRAILER && &bytes[..8] == TBL_MAGIC,
            "not a key table"
        );
        match bytes[12] {
            0 => Ok(Mode::Plain),
            1 => Ok(Mode::Aes256Gcm),
            _ => bail!("key table has an unknown mode"),
        }
    }

    /// Authenticates under `master` (ignored for plaintext tables) and parses.
    fn decode(bytes: &[u8], master: Option<&Key>) -> Result<Table> {
        let mode = Self::peek_mode(bytes)?;
        ensure!(
            le_u32(&bytes[8..12]) == FORMAT_VERSION,
            "unsupported key table version"
        );
        let count = le_u32(&bytes[48..52]) as usize;
        ensure!(count <= MAX_SEGMENTS, "key table entry count out of range");
        ensure!(
            bytes.len() == TBL_HEAD + count * TBL_ENTRY + TBL_TRAILER,
            "key table length does not match its entry count"
        );
        let cipher = match (mode, master) {
            (Mode::Plain, _) => Cipher::Null,
            (Mode::Aes256Gcm, Some(key)) => Cipher::aes(key)?,
            (Mode::Aes256Gcm, None) => bail!("encrypted key table needs the master key"),
        };
        let body_len = bytes.len() - TBL_TRAILER;
        let nonce: [u8; 12] = bytes[body_len..body_len + 12].try_into().expect("12 bytes");
        cipher
            .open_with_nonce(nonce, &bytes[..body_len], &bytes[body_len + 12..])
            .map_err(|_| anyhow!("key table failed authentication"))?;
        let mut entries = BTreeMap::new();
        for raw in bytes[TBL_HEAD..body_len].chunks_exact(TBL_ENTRY) {
            let id = le_u64(&raw[0..8]);
            let entry = TEntry {
                gen: le_u32(&raw[8..12]),
                block_bytes: le_u32(&raw[12..16]),
                hwm: le_u64(&raw[16..24]),
                wrap_nonce: raw[24..36].try_into().expect("12 bytes"),
                wrapped: raw[36..84].try_into().expect("48 bytes"),
            };
            ensure!(
                (1..=MAX_BLOCK_BYTES).contains(&entry.block_bytes) && entry.hwm >= 1,
                "key table entry out of range"
            );
            ensure!(
                entries.insert(id, entry).is_none(),
                "duplicate key table entry"
            );
        }
        let next_id = le_u64(&bytes[40..48]);
        ensure!(
            entries.keys().all(|id| *id < next_id),
            "key table segment id beyond the allocation counter"
        );
        Ok(Table {
            store_id: bytes[16..32].try_into().expect("16 bytes"),
            epoch: le_u64(&bytes[32..40]),
            mode,
            next_id,
            entries,
        })
    }

    fn wrap(&self, master: &Key, id: u64, dek: &Key) -> Result<([u8; 12], [u8; 48])> {
        let mut nonce = [0u8; 12];
        random_bytes(&mut nonce)?;
        let sealed =
            Cipher::aes(master)?.seal_with_nonce(nonce, &wrap_aad(&self.store_id, id), &dek[..])?;
        Ok((
            nonce,
            sealed
                .try_into()
                .map_err(|_| anyhow!("unexpected wrapped key length"))?,
        ))
    }

    fn unwrap(&self, master: &Key, id: u64, entry: &TEntry) -> Result<Key> {
        let plain = zeroizing(
            Cipher::aes(master)?
                .open_with_nonce(
                    entry.wrap_nonce,
                    &wrap_aad(&self.store_id, id),
                    &entry.wrapped,
                )
                .map_err(|_| anyhow!("segment key unwrap failed"))?,
        );
        let mut key = super::keys::new_key([0; 32]);
        ensure!(plain.len() == 32, "segment key has an invalid length");
        key.copy_from_slice(&plain);
        Ok(key)
    }
}

#[cfg(feature = "security")]
fn zeroizing(v: Vec<u8>) -> zeroize::Zeroizing<Vec<u8>> {
    zeroize::Zeroizing::new(v)
}
#[cfg(not(feature = "security"))]
fn zeroizing(v: Vec<u8>) -> Vec<u8> {
    v
}

struct Inner {
    table: Table,
    master: Option<Key>,
    source: Option<KeySource>,
    readers: HashMap<u64, usize>,
    writers: HashSet<u64>,
    poisoned: bool,
}

pub(crate) struct Shared {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn persist(&self, table: &Table, master: Option<&Key>) -> Result<()> {
        write_atomic(
            &self.dir.join(KEYS_FILE),
            &table.encode(master)?,
            false,
            ["table.before_rename", "table.after_rename"],
        )
    }
}

impl NonceSource for Shared {
    fn reserve(&self, segment: u64) -> Result<Range<u64>> {
        let mut inner = self.lock();
        ensure!(
            !inner.poisoned,
            "store needs to be reopened after a failed key operation"
        );
        let start = inner
            .table
            .entries
            .get(&segment)
            .ok_or_else(|| anyhow!("segment {segment} no longer exists"))?
            .hwm;
        let end = start
            .checked_add(NONCE_CHUNK)
            .ok_or_else(|| anyhow!("segment nonce space exhausted"))?;
        let mut table = inner.table.clone();
        table.entries.get_mut(&segment).expect("checked above").hwm = end;
        self.persist(&table, inner.master.as_ref())?;
        inner.table = table;
        Ok(start..end)
    }
}

pub(crate) struct OpenGuard {
    shared: Arc<Shared>,
    id: u64,
    writer: bool,
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        let mut inner = self.shared.lock();
        if self.writer {
            inner.writers.remove(&self.id);
        }
        if let Some(n) = inner.readers.get_mut(&self.id) {
            *n -= 1;
            if *n == 0 {
                inner.readers.remove(&self.id);
            }
        }
    }
}

/// Nonce counters for a segment built before it has a key table entry (migration).
struct LocalNonces(AtomicU64);

impl NonceSource for LocalNonces {
    fn reserve(&self, _: u64) -> Result<Range<u64>> {
        let start = self.0.fetch_add(NONCE_CHUNK, Ordering::SeqCst);
        Ok(start..start + NONCE_CHUNK)
    }
}

pub struct SealedStore {
    shared: Arc<Shared>,
}

fn is_segment_name(name: &str) -> Option<(u64, u32)> {
    let rest = name.strip_prefix("seg-")?;
    let (id, rest) = rest.split_once("-g")?;
    let (gen, ext) = rest.split_once('.')?;
    matches!(ext, "dat" | "hdr").then_some(())?;
    Some((u64::from_str_radix(id, 16).ok()?, gen.parse().ok()?))
}

impl SealedStore {
    pub fn create(dir: &Path, options: StoreOptions) -> Result<Self> {
        let mode = match options.mode {
            Some(mode) => mode,
            None => default_mode()?,
        };
        std::fs::create_dir_all(dir)?;
        ensure!(
            dir.join(KEYS_FILE).symlink_metadata().is_err(),
            "a store already exists in this directory"
        );
        let (master, source) = if mode == Mode::Aes256Gcm {
            ensure!(
                cfg!(feature = "security"),
                "encryption requires the `security` feature"
            );
            let mut source = options.source.unwrap_or_else(|| KeySource::auto(dir, None));
            let master = match source.load()? {
                Some(key) => key,
                None => {
                    let key = random_key()?;
                    source.store_new(&key)?;
                    key
                }
            };
            (Some(master), Some(source))
        } else {
            (None, None)
        };
        let mut store_id = [0u8; 16];
        random_bytes(&mut store_id)?;
        let table = Table {
            store_id,
            epoch: 1,
            mode,
            next_id: 1,
            entries: BTreeMap::new(),
        };
        let shared = Arc::new(Shared {
            dir: dir.to_path_buf(),
            inner: Mutex::new(Inner {
                table: table.clone(),
                master,
                source,
                readers: HashMap::new(),
                writers: HashSet::new(),
                poisoned: false,
            }),
        });
        shared.persist(&table, shared.lock().master.as_ref())?;
        Ok(Self { shared })
    }

    /// Opens an existing store. Passing `source` requires the store to be encrypted, which
    /// guards against a table downgraded to plaintext.
    pub fn open(dir: &Path, source: Option<KeySource>) -> Result<Self> {
        let bytes = read_table_file(&dir.join(KEYS_FILE))?;
        let (table, master, source) = match Table::peek_mode(&bytes)? {
            Mode::Plain => {
                ensure!(source.is_none(), "store is not encrypted");
                (Table::decode(&bytes, None)?, None, None)
            }
            Mode::Aes256Gcm => {
                let mut source = source.unwrap_or_else(|| KeySource::auto(dir, None));
                let current = source.load()?;
                let staged = source.load_staged()?;
                let attempt = current.as_ref().map(|k| Table::decode(&bytes, Some(k)));
                match (attempt, staged) {
                    (Some(Ok(table)), staged) => {
                        if staged.is_some() {
                            source.discard_staged()?;
                        }
                        (table, current, Some(source))
                    }
                    (_, Some(next)) => {
                        let table = Table::decode(&bytes, Some(&next)).map_err(|_| {
                            anyhow!("key table does not authenticate under the provided keys")
                        })?;
                        source.commit_staged()?;
                        (table, Some(next), Some(source))
                    }
                    (Some(Err(e)), None) => return Err(e),
                    (None, None) => bail!("master key not found"),
                }
            }
        };
        let store = Self {
            shared: Arc::new(Shared {
                dir: dir.to_path_buf(),
                inner: Mutex::new(Inner {
                    table,
                    master,
                    source,
                    readers: HashMap::new(),
                    writers: HashSet::new(),
                    poisoned: false,
                }),
            }),
        };
        store.collect_orphans()?;
        Ok(store)
    }

    fn collect_orphans(&self) -> Result<()> {
        let inner = self.shared.lock();
        for item in std::fs::read_dir(&self.shared.dir)? {
            let item = item?;
            let name = item.file_name();
            let Some(name) = name.to_str() else { continue };
            let stale =
                if name == "keys.new" || (name.starts_with("seg-") && name.ends_with(".new")) {
                    true
                } else if let Some((id, gen)) = is_segment_name(name) {
                    inner.table.entries.get(&id).is_none_or(|e| e.gen != gen)
                } else {
                    false
                };
            if stale {
                let _ = crate::core::durable_file::scrub_remove_file(&item.path());
            }
        }
        Ok(())
    }

    pub fn mode(&self) -> Mode {
        self.shared.lock().table.mode
    }

    pub fn store_id(&self) -> [u8; 16] {
        self.shared.lock().table.store_id
    }

    pub fn segment_ids(&self) -> Vec<u64> {
        self.shared.lock().table.entries.keys().copied().collect()
    }

    /// Creates a segment with a fresh random data key and returns its writer.
    pub fn create_segment(&self, block_bytes: u32) -> Result<SegmentWriter> {
        ensure!(
            (1..=MAX_BLOCK_BYTES).contains(&block_bytes),
            "block size must be between 1 and {MAX_BLOCK_BYTES} bytes"
        );
        let id = {
            let mut inner = self.shared.lock();
            ensure!(
                !inner.poisoned,
                "store needs to be reopened after a failed key operation"
            );
            ensure!(
                inner.table.entries.len() < MAX_SEGMENTS,
                "too many segments"
            );
            let id = inner.table.next_id;
            let mut table = inner.table.clone();
            let (wrap_nonce, wrapped) = match (&inner.master, table.mode) {
                (Some(master), Mode::Aes256Gcm) => table.wrap(master, id, &random_key()?)?,
                _ => ([0; 12], [0; 48]),
            };
            table.next_id = id + 1;
            table.entries.insert(
                id,
                TEntry {
                    gen: 1,
                    block_bytes,
                    hwm: 1,
                    wrap_nonce,
                    wrapped,
                },
            );
            self.shared.persist(&table, inner.master.as_ref())?;
            inner.table = table;
            id
        };
        self.open_segment_writer(id)
    }

    fn segment_ctx(&self, id: u64, writer: bool) -> Result<SegmentCtx> {
        let mut inner = self.shared.lock();
        ensure!(
            !inner.poisoned,
            "store needs to be reopened after a failed key operation"
        );
        let entry = inner
            .table
            .entries
            .get(&id)
            .ok_or_else(|| anyhow!("segment {id} does not exist"))?
            .clone();
        let cipher = match (&inner.master, inner.table.mode) {
            (Some(master), Mode::Aes256Gcm) => {
                let dek = inner.table.unwrap(master, id, &entry)?;
                Cipher::aes(&dek)?
            }
            _ => Cipher::Null,
        };
        if writer {
            ensure!(
                inner.writers.insert(id),
                "segment {id} already has a writer"
            );
        }
        *inner.readers.entry(id).or_insert(0) += 1;
        Ok(SegmentCtx {
            dir: self.shared.dir.clone(),
            store_id: inner.table.store_id,
            encrypted: inner.table.mode == Mode::Aes256Gcm,
            id,
            gen: entry.gen,
            block_bytes: entry.block_bytes,
            cipher: Arc::new(cipher),
            nonce_hwm: entry.hwm,
            nonces: self.shared.clone(),
            guard: Some(OpenGuard {
                shared: self.shared.clone(),
                id,
                writer,
            }),
        })
    }

    pub fn open_segment_writer(&self, id: u64) -> Result<SegmentWriter> {
        open_writer(self.segment_ctx(id, true)?)
    }

    /// Opens from the key table and the segment header only; no block is read or decrypted.
    pub fn open_segment_reader(&self, id: u64) -> Result<SegmentReader> {
        open_reader(self.segment_ctx(id, false)?)
    }

    /// Crypto-erases a segment: its data key is dropped, the remaining keys are re-wrapped
    /// under a rotated master key, and the new table replaces the old one atomically. The
    /// rename of the table is the commit point; a crash before it leaves the segment intact
    /// and a crash after it leaves it erased, with leftover files collected on the next open.
    pub fn purge_segment(&self, id: u64) -> Result<()> {
        let mut inner = self.shared.lock();
        ensure!(
            !inner.poisoned,
            "store needs to be reopened after a failed key operation"
        );
        ensure!(!inner.readers.contains_key(&id), "segment {id} is open");
        let gen = inner
            .table
            .entries
            .get(&id)
            .ok_or_else(|| anyhow!("segment {id} does not exist"))?
            .gen;
        let mut next = inner.table.clone();
        next.entries.remove(&id);
        let new_master = self.rewrap(&inner, &mut next, None)?;
        self.swap_table(&mut inner, next, new_master)?;
        let (dat, hdr) = segment_paths(&self.shared.dir, id, gen);
        for path in [dat, hdr] {
            let _ = crate::core::durable_file::scrub_remove_file(&path);
        }
        Ok(())
    }

    /// Replaces the master key (random, or `new_key` for a `Raw` provider whose caller
    /// stores it) and re-wraps every segment key under it.
    pub fn rotate_master(&self, new_key: Option<[u8; 32]>) -> Result<()> {
        let mut inner = self.shared.lock();
        ensure!(
            !inner.poisoned,
            "store needs to be reopened after a failed key operation"
        );
        let mut next = inner.table.clone();
        let new_master = self.rewrap(&inner, &mut next, new_key.map(super::keys::new_key))?;
        self.swap_table(&mut inner, next, new_master)
    }

    /// Re-wraps every entry of `table` (already pruned) under a new master key.
    fn rewrap(
        &self,
        inner: &Inner,
        table: &mut Table,
        supplied: Option<Key>,
    ) -> Result<Option<Key>> {
        let Some(old) = inner.master.as_ref() else {
            return Ok(None);
        };
        let new = match supplied {
            Some(key) => key,
            None => random_key()?,
        };
        table.epoch += 1;
        let ids: Vec<u64> = table.entries.keys().copied().collect();
        for id in ids {
            let entry = table.entries[&id].clone();
            let dek = inner.table.unwrap(old, id, &entry)?;
            let (wrap_nonce, wrapped) = table.wrap(&new, id, &dek)?;
            let slot = table.entries.get_mut(&id).expect("listed above");
            slot.wrap_nonce = wrap_nonce;
            slot.wrapped = wrapped;
        }
        Ok(Some(new))
    }

    fn swap_table(&self, inner: &mut Inner, table: Table, new_master: Option<Key>) -> Result<()> {
        inner.poisoned = true;
        if let (Some(master), Some(source)) = (new_master.as_ref(), inner.source.as_mut()) {
            source.stage(master)?;
            failpoint("rotate.staged")?;
        }
        self.shared.persist(&table, new_master.as_ref())?;
        failpoint("rotate.table_committed")?;
        if let Some(source) = inner.source.as_mut() {
            source.commit_staged()?;
        }
        if new_master.is_some() {
            inner.master = new_master;
        }
        inner.table = table;
        inner.poisoned = false;
        Ok(())
    }

    /// Converts a plaintext store to an encrypted one: every segment is rewritten under a
    /// fresh data key at the next generation, then the table is replaced atomically. Until
    /// that commit the store is still the plaintext one; leftover files of either side are
    /// collected on the next open.
    pub fn migrate_to_encrypted(&self, source: Option<KeySource>) -> Result<()> {
        ensure!(
            cfg!(feature = "security"),
            "encryption requires the `security` feature"
        );
        let mut inner = self.shared.lock();
        ensure!(
            !inner.poisoned,
            "store needs to be reopened after a failed key operation"
        );
        ensure!(
            inner.table.mode == Mode::Plain,
            "store is already encrypted"
        );
        ensure!(
            inner.readers.is_empty() && inner.writers.is_empty(),
            "close all segment handles before migrating"
        );
        let mut source = source.unwrap_or_else(|| KeySource::auto(&self.shared.dir, None));
        let master = match source.load()? {
            Some(key) => key,
            None => {
                let key = random_key()?;
                source.store_new(&key)?;
                key
            }
        };
        let old = inner.table.clone();
        let mut next = Table {
            mode: Mode::Aes256Gcm,
            epoch: old.epoch + 1,
            ..old.clone()
        };
        let local = Arc::new(LocalNonces(AtomicU64::new(1)));
        for (&id, entry) in &old.entries {
            let new_gen = entry.gen + 1;
            let (dat, hdr) = segment_paths(&self.shared.dir, id, new_gen);
            let _ = std::fs::remove_file(dat);
            let _ = std::fs::remove_file(hdr);
            let dek = random_key()?;
            local.0.store(1, Ordering::SeqCst);
            let ctx = |gen, encrypted, cipher, nonce_hwm| SegmentCtx {
                dir: self.shared.dir.clone(),
                store_id: old.store_id,
                encrypted,
                id,
                gen,
                block_bytes: entry.block_bytes,
                cipher: Arc::new(cipher),
                nonce_hwm,
                nonces: local.clone(),
                guard: None,
            };
            let reader = open_reader(ctx(entry.gen, false, Cipher::Null, entry.hwm))?;
            let mut writer = open_writer(ctx(new_gen, true, Cipher::aes(&dek)?, 1))?;
            for i in 0..reader.block_count() {
                writer.append_block(&reader.read_block(i)?)?;
            }
            writer.commit()?;
            let (wrap_nonce, wrapped) = next.wrap(&master, id, &dek)?;
            let slot = next.entries.get_mut(&id).expect("cloned from old");
            slot.gen = new_gen;
            slot.hwm = local.0.load(Ordering::SeqCst);
            slot.wrap_nonce = wrap_nonce;
            slot.wrapped = wrapped;
        }
        failpoint("migrate.before_commit")?;
        self.shared.persist(&next, Some(&master))?;
        failpoint("migrate.after_commit")?;
        for (&id, entry) in &old.entries {
            let (dat, hdr) = segment_paths(&self.shared.dir, id, entry.gen);
            for path in [dat, hdr] {
                let _ = crate::core::durable_file::scrub_remove_file(&path);
            }
        }
        inner.table = next;
        inner.master = Some(master);
        inner.source = Some(source);
        Ok(())
    }
}

fn read_table_file(path: &Path) -> Result<Vec<u8>> {
    let mut file = crate::core::durable_file::open_regular(path, false)
        .map_err(|e| anyhow!("key table unreadable: {}", e.kind()))?;
    let len = file.metadata()?.len();
    ensure!(
        len <= (TBL_HEAD + MAX_SEGMENTS * TBL_ENTRY + TBL_TRAILER) as u64,
        "key table is too large"
    );
    let mut bytes = vec![0u8; len as usize];
    std::io::Read::read_exact(&mut file, &mut bytes)?;
    Ok(bytes)
}

#[cfg(feature = "security")]
const BACKUP_MAGIC: &[u8; 8] = b"WRMBKP01";

#[cfg(feature = "security")]
impl SealedStore {
    /// Writes the master key and the current key table to `path`, sealed under a key derived
    /// from `passphrase` with Argon2id. Segment keys created after the backup are not in it.
    pub fn export_backup(&self, path: &Path, passphrase: &str) -> Result<()> {
        use argon2::Argon2;
        let inner = self.shared.lock();
        let master = inner
            .master
            .as_ref()
            .ok_or_else(|| anyhow!("plaintext stores have no keys to back up"))?;
        let mut plain = zeroizing(Vec::with_capacity(32 + 4096));
        plain.extend_from_slice(&master[..]);
        plain.extend_from_slice(&inner.table.encode(Some(master))?);
        let mut salt = [0u8; 16];
        random_bytes(&mut salt)?;
        let mut nonce = [0u8; 12];
        random_bytes(&mut nonce)?;
        let mut kek = super::keys::new_key([0; 32]);
        Argon2::default()
            .hash_password_into(passphrase.as_bytes(), &salt, &mut kek[..])
            .map_err(|_| anyhow!("backup key derivation failed"))?;
        let sealed = Cipher::aes(&kek)?.seal_with_nonce(nonce, BACKUP_MAGIC, &plain)?;
        let mut out = Vec::with_capacity(36 + sealed.len());
        out.extend_from_slice(BACKUP_MAGIC);
        out.extend_from_slice(&salt);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        write_atomic(
            path,
            &out,
            true,
            ["backup.before_rename", "backup.after_rename"],
        )
    }

    /// Restores a backup into `dir` (which must hold no store) and publishes the master key
    /// through `source`. Segment files are copied separately by the caller.
    pub fn import_backup(
        path: &Path,
        passphrase: &str,
        dir: &Path,
        mut source: KeySource,
    ) -> Result<Self> {
        use argon2::Argon2;
        let mut file = crate::core::durable_file::open_regular(path, false)?;
        let len = file.metadata()?.len();
        ensure!(
            (36 + 16 + 32 + TBL_HEAD as u64 + TBL_TRAILER as u64..=(64 << 20)).contains(&len),
            "backup file has an invalid length"
        );
        let mut bytes = vec![0u8; len as usize];
        std::io::Read::read_exact(&mut file, &mut bytes)?;
        ensure!(&bytes[..8] == BACKUP_MAGIC, "not a key backup");
        let mut kek = super::keys::new_key([0; 32]);
        Argon2::default()
            .hash_password_into(passphrase.as_bytes(), &bytes[8..24], &mut kek[..])
            .map_err(|_| anyhow!("backup key derivation failed"))?;
        let nonce: [u8; 12] = bytes[24..36].try_into().expect("12 bytes");
        let plain = zeroizing(
            Cipher::aes(&kek)?
                .open_with_nonce(nonce, BACKUP_MAGIC, &bytes[36..])
                .map_err(|_| anyhow!("backup failed authentication"))?,
        );
        let mut master = super::keys::new_key([0; 32]);
        master.copy_from_slice(&plain[..32]);
        Table::decode(&plain[32..], Some(&master))?;
        std::fs::create_dir_all(dir)?;
        ensure!(
            dir.join(KEYS_FILE).symlink_metadata().is_err(),
            "a store already exists in this directory"
        );
        match source.load()? {
            Some(existing) => ensure!(
                bool::from(subtle::ConstantTimeEq::ct_eq(&existing[..], &master[..])),
                "the key provider holds a different key"
            ),
            None => source.store_new(&master)?,
        }
        write_atomic(
            &dir.join(KEYS_FILE),
            &plain[32..],
            false,
            ["table.before_rename", "table.after_rename"],
        )?;
        Self::open(dir, Some(source))
    }
}
