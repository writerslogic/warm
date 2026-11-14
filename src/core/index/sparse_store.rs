// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! An updatable, persistent store of learned-sparse vectors (term id to weight) keyed by a
//! string id, searched by inner product.
//!
//! The file `sparse-docs.wsp` is an append-only log: a 16-byte header (the magic `WARMSPRS`,
//! the format version as a little-endian `u32`, four reserved zero bytes) followed by framed
//! records, each `payload length: u32 LE`, `crc32 of the payload: u32 LE`, then the payload:
//!
//! - put: `1u8`, `id length: u16 LE`, the id (UTF-8), `nnz: u32 LE`, `nnz` term ids (`u16 LE`,
//!   strictly ascending), `nnz` weights (`f32 LE`, finite and positive);
//! - delete: `2u8`, `id length: u16 LE`, the id.
//!
//! A record is applied in file order, so the last record of an id wins. A torn tail (a record
//! cut short, or a last record failing its checksum) is dropped and truncated at open; a bad
//! checksum anywhere else is an error. The file is created by the first put, so a store that
//! never receives a vector has no file. When the log holds more than twice the bytes of the
//! live rows (plus a floor), it is rewritten through a temporary file and an atomic rename.
//!
//! Search is exact over the stored rows. Once the store holds [`SparseTuning::pruned_min_rows`]
//! live rows, a [`SparseImpactIndex`] is built from them and answers instead (approximate
//! under the default parameters); rows put after the build are scored exactly and merged, rows
//! removed or replaced since are dropped, and the index is rebuilt when those changes exceed a
//! tenth of the live rows.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};

use super::sparse_impact::{CsrView, SparseBuildParams, SparseImpactIndex, SparseSearchParams};
use crate::core::sync::RwLock;

pub const FILE_NAME: &str = "sparse-docs.wsp";
pub const MAGIC: &[u8; 8] = b"WARMSPRS";
pub const VERSION: u32 = 1;
/// Exclusive bound on term ids: the impact index stores them in two bytes.
pub const MAX_TERM_ID: u32 = 1 << 16;
/// Largest `k` a search accepts.
pub const MAX_K: usize = 10_000;

const HEADER_LEN: usize = 16;
const FRAME_LEN: usize = 8;
const PUT: u8 = 1;
const DELETE: u8 = 2;
const COMPACT_FLOOR: u64 = 1 << 20;

/// Size limits enforced at the boundary and at load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SparseLimits {
    pub max_doc_nnz: usize,
    pub max_query_nnz: usize,
    /// Nonzero entries over all live rows (about 6 bytes each in memory).
    pub max_total_nnz: u64,
    pub max_rows: usize,
}

impl Default for SparseLimits {
    fn default() -> Self {
        Self {
            max_doc_nnz: 4096,
            max_query_nnz: 1024,
            max_total_nnz: 1 << 29,
            max_rows: 1 << 26,
        }
    }
}

/// When the pruned index takes over, and its parameters.
#[derive(Clone, Copy, Debug)]
pub struct SparseTuning {
    pub pruned_min_rows: usize,
    pub build: SparseBuildParams,
    pub search: SparseSearchParams,
}

impl Default for SparseTuning {
    fn default() -> Self {
        Self {
            pruned_min_rows: 100_000,
            build: SparseBuildParams::default(),
            search: SparseSearchParams {
                query_cut: 8,
                heap_factor: 0.9,
                ..SparseSearchParams::exact()
            },
        }
    }
}

/// A sparse vector as supplied by a caller: parallel term ids and weights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SparseVector {
    pub ids: Vec<u32>,
    pub weights: Vec<f32>,
}

impl SparseVector {
    pub fn new(ids: Vec<u32>, weights: Vec<f32>) -> Self {
        Self { ids, weights }
    }

    /// Sorted, zero-free `(id, weight)` pairs. A document must not repeat a term; a query's
    /// repeated terms add up.
    fn validated(&self, max_nnz: usize, what: &str, merge: bool) -> Result<Row> {
        ensure!(
            self.ids.len() == self.weights.len(),
            "sparse {what}: {} term ids but {} weights",
            self.ids.len(),
            self.weights.len()
        );
        ensure!(
            self.ids.len() <= max_nnz,
            "sparse {what} has {} entries, at most {max_nnz}",
            self.ids.len()
        );
        let mut pairs = Vec::with_capacity(self.ids.len());
        for (&id, &weight) in self.ids.iter().zip(&self.weights) {
            ensure!(
                id < MAX_TERM_ID,
                "sparse {what}: term id {id} is not below {MAX_TERM_ID}"
            );
            ensure!(
                weight.is_finite() && weight >= 0.0,
                "sparse {what}: weight {weight} for term {id} must be finite and >= 0"
            );
            if weight > 0.0 {
                pairs.push((id as u16, weight));
            }
        }
        pairs.sort_unstable_by_key(|p| p.0);
        let mut merged: Vec<(u16, f32)> = Vec::with_capacity(pairs.len());
        for (id, weight) in pairs {
            match merged.last_mut() {
                Some(last) if last.0 == id => {
                    ensure!(merge, "sparse {what}: term id {id} appears twice");
                    last.1 += weight;
                    ensure!(last.1.is_finite(), "sparse {what}: weight overflow");
                }
                _ => merged.push((id, weight)),
            }
        }
        Ok(Row {
            ids: merged.iter().map(|p| p.0).collect(),
            weights: merged.iter().map(|p| p.1).collect(),
        })
    }
}

struct Row {
    ids: Box<[u16]>,
    weights: Box<[f32]>,
}

impl Row {
    fn record_len(&self, id: &str) -> u64 {
        (FRAME_LEN + 1 + 2 + id.len() + 4 + 6 * self.ids.len()) as u64
    }
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME_LEN + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn encode_put(id: &str, row: &Row) -> Vec<u8> {
    let mut payload = Vec::with_capacity(7 + id.len() + 6 * row.ids.len());
    payload.push(PUT);
    payload.extend_from_slice(&(id.len() as u16).to_le_bytes());
    payload.extend_from_slice(id.as_bytes());
    payload.extend_from_slice(&(row.ids.len() as u32).to_le_bytes());
    for id in row.ids.iter() {
        payload.extend_from_slice(&id.to_le_bytes());
    }
    for w in row.weights.iter() {
        payload.extend_from_slice(&w.to_le_bytes());
    }
    frame(&payload)
}

fn encode_delete(id: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(3 + id.len());
    payload.push(DELETE);
    payload.extend_from_slice(&(id.len() as u16).to_le_bytes());
    payload.extend_from_slice(id.as_bytes());
    frame(&payload)
}

fn header() -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&VERSION.to_le_bytes());
    h
}

enum Record {
    Put(String, Row),
    Delete(String),
}

fn decode(payload: &[u8], limits: &SparseLimits) -> Result<Record> {
    let corrupt = || anyhow::anyhow!("sparse record is malformed");
    let kind = *payload.first().ok_or_else(corrupt)?;
    let id_len = u16::from_le_bytes(payload.get(1..3).ok_or_else(corrupt)?.try_into()?) as usize;
    let id = std::str::from_utf8(payload.get(3..3 + id_len).ok_or_else(corrupt)?)?.to_owned();
    let rest = &payload[3 + id_len..];
    match kind {
        DELETE => {
            ensure!(rest.is_empty(), "sparse delete record has trailing bytes");
            Ok(Record::Delete(id))
        }
        PUT => {
            let nnz = u32::from_le_bytes(rest.get(..4).ok_or_else(corrupt)?.try_into()?) as usize;
            ensure!(
                nnz <= limits.max_doc_nnz,
                "sparse record exceeds the entry limit"
            );
            ensure!(
                rest.len() == 4 + 6 * nnz,
                "sparse put record has the wrong length"
            );
            let ids: Vec<u16> = rest[4..4 + 2 * nnz]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            let weights: Vec<f32> = rest[4 + 2 * nnz..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            ensure!(
                ids.windows(2).all(|w| w[0] < w[1])
                    && weights.iter().all(|w| w.is_finite() && *w > 0.0),
                "sparse put record holds unordered ids or invalid weights"
            );
            Ok(Record::Put(
                id,
                Row {
                    ids: ids.into(),
                    weights: weights.into(),
                },
            ))
        }
        other => bail!("unknown sparse record kind {other}"),
    }
}

struct Built {
    index: SparseImpactIndex,
    /// Slot of each indexed row, in index order.
    slot_of: Vec<u32>,
    built_slots: usize,
    /// Rows put or removed since the build.
    changed: usize,
}

struct Inner {
    /// Slots are never reused between compactions: a replaced row takes a new slot.
    rows: Vec<Option<(String, Row)>>,
    slots: HashMap<String, u32>,
    live_nnz: u64,
    live_bytes: u64,
    file: File,
    file_len: u64,
    built: Option<Built>,
}

pub struct SparseStore {
    path: PathBuf,
    limits: SparseLimits,
    tuning: SparseTuning,
    inner: RwLock<Option<Inner>>,
}

fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

impl SparseStore {
    /// Opens the store in `dir`; nothing is created when the file is absent.
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with(dir, SparseLimits::default(), SparseTuning::default())
    }

    pub fn open_with(dir: &Path, limits: SparseLimits, tuning: SparseTuning) -> Result<Self> {
        let path = dir.join(FILE_NAME);
        let inner = if path.exists() {
            Self::load(&path, &limits)?
        } else {
            None
        };
        Ok(Self {
            path,
            limits,
            tuning,
            inner: RwLock::new(inner),
        })
    }

    fn load(path: &Path, limits: &SparseLimits) -> Result<Option<Inner>> {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        if bytes.len() < HEADER_LEN {
            std::fs::remove_file(path)?;
            return Ok(None);
        }
        ensure!(
            &bytes[..8] == MAGIC,
            "{} is not a sparse vector file",
            path.display()
        );
        let version = u32::from_le_bytes(bytes[8..12].try_into()?);
        ensure!(
            version == VERSION,
            "sparse vector file version {version} is not supported (this build reads {VERSION})"
        );
        let mut rows: Vec<Option<(String, Row)>> = Vec::new();
        let mut slots: HashMap<String, u32> = HashMap::new();
        let (mut live_nnz, mut live_bytes) = (0u64, 0u64);
        let mut at = HEADER_LEN;
        while at < bytes.len() {
            let Some(head) = bytes.get(at..at + FRAME_LEN) else {
                break;
            };
            let len = u32::from_le_bytes(head[..4].try_into()?) as usize;
            let crc = u32::from_le_bytes(head[4..].try_into()?);
            let end = at + FRAME_LEN + len;
            if end > bytes.len() {
                break;
            }
            let payload = &bytes[at + FRAME_LEN..end];
            if crc32fast::hash(payload) != crc {
                ensure!(
                    end == bytes.len(),
                    "sparse vector file is corrupt at byte {at}"
                );
                break;
            }
            match decode(payload, limits)? {
                Record::Put(id, row) => {
                    if let Some(old) = slots.remove(&id) {
                        if let Some((old_id, old_row)) = rows[old as usize].take() {
                            live_nnz -= old_row.ids.len() as u64;
                            live_bytes -= old_row.record_len(&old_id);
                        }
                    }
                    live_nnz += row.ids.len() as u64;
                    live_bytes += row.record_len(&id);
                    ensure!(
                        live_nnz <= limits.max_total_nnz && slots.len() < limits.max_rows,
                        "sparse vector file exceeds the store limits"
                    );
                    slots.insert(id.clone(), rows.len() as u32);
                    rows.push(Some((id, row)));
                }
                Record::Delete(id) => {
                    if let Some(old) = slots.remove(&id) {
                        if let Some((old_id, old_row)) = rows[old as usize].take() {
                            live_nnz -= old_row.ids.len() as u64;
                            live_bytes -= old_row.record_len(&old_id);
                        }
                    }
                }
            }
            at = end;
        }
        let file = OpenOptions::new().append(true).read(true).open(path)?;
        if at < bytes.len() {
            file.set_len(at as u64)?;
            file.sync_all()?;
        }
        Ok(Some(Inner {
            rows,
            slots,
            live_nnz,
            live_bytes,
            file,
            file_len: at as u64,
            built: None,
        }))
    }

    pub fn is_active(&self) -> bool {
        self.inner.read().is_some()
    }

    /// Live rows.
    pub fn len(&self) -> usize {
        self.inner.read().as_ref().map_or(0, |i| i.slots.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, id: &str) -> bool {
        self.inner
            .read()
            .as_ref()
            .is_some_and(|i| i.slots.contains_key(id))
    }

    /// Whether the pruned index currently serves searches.
    pub fn index_built(&self) -> bool {
        self.inner
            .read()
            .as_ref()
            .is_some_and(|i| i.built.is_some())
    }

    /// Inserts or replaces rows durably; the last entry of a repeated id wins. A vector with
    /// no positive weight removes the id's row. Everything is validated before anything is
    /// written.
    pub fn put_many(&self, items: Vec<(String, SparseVector)>) -> Result<()> {
        let mut staged: HashMap<String, Option<Row>> = HashMap::with_capacity(items.len());
        for (id, vector) in &items {
            ensure!(
                !id.is_empty() && id.len() <= u16::MAX as usize,
                "sparse vector id must be 1..=65535 bytes"
            );
            let row = vector.validated(self.limits.max_doc_nnz, "document vector", false)?;
            staged.insert(id.clone(), (!row.ids.is_empty()).then_some(row));
        }
        let mut guard = self.inner.write();
        let mut order: Vec<String> = Vec::with_capacity(staged.len());
        let mut queued: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (id, _) in &items {
            if queued.insert(id.as_str()) {
                order.push(id.clone());
            }
        }
        let (mut nnz, mut rows) = guard
            .as_ref()
            .map_or((0u64, 0usize), |i| (i.live_nnz, i.slots.len()));
        for id in &order {
            let old = guard
                .as_ref()
                .and_then(|i| i.slots.get(id).map(|&s| i.rows[s as usize].as_ref()))
                .flatten();
            if let Some((_, old)) = old {
                nnz -= old.ids.len() as u64;
                rows -= 1;
            }
            if let Some(new) = &staged[id] {
                nnz += new.ids.len() as u64;
                rows += 1;
            }
        }
        ensure!(
            nnz <= self.limits.max_total_nnz && rows <= self.limits.max_rows,
            "sparse store would exceed its limits ({} entries, {} rows)",
            self.limits.max_total_nnz,
            self.limits.max_rows
        );
        if guard.is_none() {
            if staged.values().all(Option::is_none) {
                return Ok(());
            }
            *guard = Some(self.create()?);
        }
        let inner = guard.as_mut().expect("created above");
        let mut buf = Vec::new();
        for id in &order {
            match &staged[id] {
                Some(row) => buf.extend_from_slice(&encode_put(id, row)),
                None if inner.slots.contains_key(id) => buf.extend_from_slice(&encode_delete(id)),
                None => {}
            }
        }
        if !buf.is_empty() {
            inner.file.write_all(&buf)?;
            inner.file.sync_data()?;
            inner.file_len += buf.len() as u64;
        }
        for id in order {
            let row = staged.remove(&id).expect("staged");
            inner.apply(id, row);
        }
        inner.compact_if_needed(&self.path)
    }

    /// Checks a document vector against the store limits without storing it.
    pub fn validate_document(&self, vector: &SparseVector) -> Result<()> {
        vector
            .validated(self.limits.max_doc_nnz, "document vector", false)
            .map(drop)
    }

    pub fn put(&self, id: &str, vector: &SparseVector) -> Result<()> {
        self.put_many(vec![(id.to_owned(), vector.clone())])
    }

    /// Removes a row durably; false when it was absent.
    pub fn remove(&self, id: &str) -> Result<bool> {
        Ok(self.remove_many(&[id.to_owned()])? == 1)
    }

    /// Removes the present rows among `ids` with one write and one sync; returns how many were
    /// present.
    pub fn remove_many(&self, ids: &[String]) -> Result<usize> {
        if !ids.iter().any(|id| self.contains(id)) {
            return Ok(0);
        }
        let mut guard = self.inner.write();
        let Some(inner) = guard.as_mut() else {
            return Ok(0);
        };
        let mut seen = std::collections::HashSet::new();
        let present: Vec<&String> = ids
            .iter()
            .filter(|id| inner.slots.contains_key(id.as_str()) && seen.insert(id.as_str()))
            .collect();
        if present.is_empty() {
            return Ok(0);
        }
        let buf: Vec<u8> = present.iter().flat_map(|id| encode_delete(id)).collect();
        inner.file.write_all(&buf)?;
        inner.file.sync_data()?;
        inner.file_len += buf.len() as u64;
        for id in &present {
            inner.apply((*id).clone(), None);
        }
        inner.compact_if_needed(&self.path)?;
        Ok(present.len())
    }

    fn create(&self) -> Result<Inner> {
        let tmp = self.path.with_extension("wsp.tmp");
        let mut file = File::create(&tmp)?;
        file.write_all(&header())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &self.path).context("create sparse vector file")?;
        if let Some(dir) = self.path.parent() {
            sync_dir(dir)?;
        }
        Ok(Inner {
            rows: Vec::new(),
            slots: HashMap::new(),
            live_nnz: 0,
            live_bytes: 0,
            file: OpenOptions::new()
                .append(true)
                .read(true)
                .open(&self.path)?,
            file_len: HEADER_LEN as u64,
            built: None,
        })
    }

    /// The top-`k` live ids by inner product with `query`, best first; ties go to the lower
    /// id and rows scoring zero never appear. `live` vets each candidate (a row whose owner
    /// is gone must not surface); candidates are drawn deeper until `k` pass or none remain.
    pub fn search(
        &self,
        query: &SparseVector,
        k: usize,
        live: &mut dyn FnMut(&str) -> bool,
    ) -> Result<Vec<(String, f64)>> {
        ensure!((1..=MAX_K).contains(&k), "k must be in 1..={MAX_K}");
        let q = query.validated(self.limits.max_query_nnz, "query", true)?;
        if q.ids.is_empty() {
            return Ok(Vec::new());
        }
        self.maybe_rebuild()?;
        let guard = self.inner.read();
        let Some(inner) = guard.as_ref() else {
            return Ok(Vec::new());
        };
        let mut depth = k;
        loop {
            let ranked = inner.rank(&q, depth, &self.tuning);
            let exhausted = ranked.len() < depth;
            let mut out: Vec<(String, f64)> = Vec::with_capacity(k);
            for (slot, score) in &ranked {
                let id = &inner.rows[*slot as usize]
                    .as_ref()
                    .expect("ranked rows are live")
                    .0;
                if live(id) {
                    out.push((id.clone(), *score));
                    if out.len() == k {
                        break;
                    }
                }
            }
            if out.len() == k || exhausted {
                return Ok(out);
            }
            depth = depth.saturating_mul(4);
        }
    }

    fn maybe_rebuild(&self) -> Result<()> {
        {
            let guard = self.inner.read();
            let Some(inner) = guard.as_ref() else {
                return Ok(());
            };
            if !inner.needs_build(&self.tuning) {
                return Ok(());
            }
        }
        let mut guard = self.inner.write();
        if let Some(inner) = guard.as_mut().filter(|i| i.needs_build(&self.tuning)) {
            inner.build(&self.tuning)?;
        }
        Ok(())
    }
}

impl Inner {
    fn apply(&mut self, id: String, row: Option<Row>) {
        if let Some(old) = self.slots.remove(&id) {
            if let Some((old_id, old_row)) = self.rows[old as usize].take() {
                self.live_nnz -= old_row.ids.len() as u64;
                self.live_bytes -= old_row.record_len(&old_id);
            }
            if let Some(built) = &mut self.built {
                built.changed += 1;
            }
        }
        if let Some(row) = row {
            self.live_nnz += row.ids.len() as u64;
            self.live_bytes += row.record_len(&id);
            self.slots.insert(id.clone(), self.rows.len() as u32);
            self.rows.push(Some((id, row)));
            if let Some(built) = &mut self.built {
                built.changed += 1;
            }
        }
    }

    fn compact_if_needed(&mut self, path: &Path) -> Result<()> {
        if self.file_len <= 2 * self.live_bytes + COMPACT_FLOOR {
            return Ok(());
        }
        let tmp = path.with_extension("wsp.tmp");
        let mut out = Vec::with_capacity(self.live_bytes as usize + HEADER_LEN);
        out.extend_from_slice(&header());
        let mut rows = Vec::with_capacity(self.slots.len());
        let mut slots = HashMap::with_capacity(self.slots.len());
        for (id, row) in self.rows.drain(..).flatten() {
            out.extend_from_slice(&encode_put(&id, &row));
            slots.insert(id.clone(), rows.len() as u32);
            rows.push(Some((id, row)));
        }
        let mut file = File::create(&tmp)?;
        file.write_all(&out)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        if let Some(dir) = path.parent() {
            sync_dir(dir)?;
        }
        self.rows = rows;
        self.slots = slots;
        self.file = OpenOptions::new().append(true).read(true).open(path)?;
        self.file_len = out.len() as u64;
        self.built = None;
        Ok(())
    }

    fn needs_build(&self, tuning: &SparseTuning) -> bool {
        let live = self.slots.len();
        match &self.built {
            None => live >= tuning.pruned_min_rows && live > 0,
            Some(b) => b.changed > live / 10 && live >= tuning.pruned_min_rows,
        }
    }

    fn build(&mut self, tuning: &SparseTuning) -> Result<()> {
        let mut slot_of = Vec::with_capacity(self.slots.len());
        let mut indptr = vec![0i64];
        let mut indices: Vec<i32> = Vec::with_capacity(self.live_nnz as usize);
        let mut data: Vec<f32> = Vec::with_capacity(self.live_nnz as usize);
        let mut n_dims = 1usize;
        for (slot, entry) in self.rows.iter().enumerate() {
            let Some((_, row)) = entry else { continue };
            slot_of.push(slot as u32);
            indices.extend(row.ids.iter().map(|&t| i32::from(t)));
            data.extend_from_slice(&row.weights);
            indptr.push(indices.len() as i64);
            if let Some(&last) = row.ids.last() {
                n_dims = n_dims.max(last as usize + 1);
            }
        }
        let index = SparseImpactIndex::build(
            CsrView {
                indptr: &indptr,
                indices: &indices,
                data: &data,
                n_dims,
            },
            tuning.build,
        )?;
        self.built = Some(Built {
            index,
            slot_of,
            built_slots: self.rows.len(),
            changed: 0,
        });
        Ok(())
    }

    /// The best `depth` live `(slot, score)` pairs, scores descending then ids ascending.
    fn rank(&self, q: &Row, depth: usize, tuning: &SparseTuning) -> Vec<(u32, f64)> {
        let mut dense = vec![0f32; q.ids.last().map_or(0, |&t| t as usize + 1)];
        for (&t, &w) in q.ids.iter().zip(q.weights.iter()) {
            dense[t as usize] = w;
        }
        let score = |row: &Row| -> f64 {
            row.ids
                .iter()
                .zip(row.weights.iter())
                .map(|(&t, &w)| {
                    dense
                        .get(t as usize)
                        .map_or(0.0, |&qw| f64::from(qw) * f64::from(w))
                })
                .sum()
        };
        let mut scored: Vec<(u32, f64)> = Vec::new();
        let from = match &self.built {
            Some(built) => {
                let stale = built.changed;
                let ids: Vec<u32> = q.ids.iter().map(|&t| u32::from(t)).collect();
                let mut searcher = built.index.searcher();
                for (doc, _) in searcher.search(&ids, &q.weights, depth + stale, tuning.search) {
                    let slot = built.slot_of[doc as usize];
                    if let Some((_, row)) = &self.rows[slot as usize] {
                        scored.push((slot, score(row)));
                    }
                }
                built.built_slots
            }
            None => 0,
        };
        for slot in from..self.rows.len() {
            if let Some((_, row)) = &self.rows[slot] {
                let s = score(row);
                if s > 0.0 {
                    scored.push((slot as u32, s));
                }
            }
        }
        scored.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.id_of(a.0).cmp(self.id_of(b.0)))
        });
        scored.truncate(depth);
        scored
    }

    fn id_of(&self, slot: u32) -> &str {
        &self.rows[slot as usize].as_ref().expect("live slot").0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn vector(&mut self, dims: u64, max_nnz: u64) -> SparseVector {
            let nnz = 1 + self.below(max_nnz) as usize;
            let mut ids: Vec<u32> = (0..nnz).map(|_| self.below(dims) as u32).collect();
            ids.sort_unstable();
            ids.dedup();
            let weights = ids
                .iter()
                .map(|_| 0.01 + self.below(1000) as f32 / 100.0)
                .collect();
            SparseVector::new(ids, weights)
        }
    }

    fn brute(
        model: &BTreeMap<String, SparseVector>,
        q: &SparseVector,
        k: usize,
    ) -> Vec<(String, f64)> {
        let mut scored: Vec<(String, f64)> = model
            .iter()
            .map(|(id, v)| {
                let s: f64 = v
                    .ids
                    .iter()
                    .zip(&v.weights)
                    .map(|(t, w)| {
                        q.ids
                            .iter()
                            .zip(&q.weights)
                            .filter(|(qt, _)| *qt == t)
                            .map(|(_, qw)| f64::from(*qw) * f64::from(*w))
                            .sum::<f64>()
                    })
                    .sum();
                (id.clone(), s)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(k);
        scored
    }

    fn brute_ids(
        model: &BTreeMap<String, SparseVector>,
        q: &SparseVector,
        k: usize,
    ) -> Vec<String> {
        brute(model, q, k).into_iter().map(|p| p.0).collect()
    }

    fn ids(store: &SparseStore, q: &SparseVector, k: usize) -> Vec<String> {
        store
            .search(q, k, &mut |_| true)
            .unwrap()
            .into_iter()
            .map(|p| p.0)
            .collect()
    }

    fn exact_tuning(min_rows: usize) -> SparseTuning {
        SparseTuning {
            pruned_min_rows: min_rows,
            build: SparseBuildParams::exact(),
            search: SparseSearchParams::exact(),
        }
    }

    /// A random put/replace/remove history, searched against a brute-force model, with a
    /// reopen in the middle: with and without the impact index, results equal the model's.
    #[test]
    fn random_history_matches_brute_force() {
        for min_rows in [usize::MAX, 40] {
            let dir = tempfile::tempdir().unwrap();
            let open = || {
                SparseStore::open_with(dir.path(), SparseLimits::default(), exact_tuning(min_rows))
                    .unwrap()
            };
            let mut store = open();
            let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
            let mut model: BTreeMap<String, SparseVector> = BTreeMap::new();
            for step in 0..600 {
                let id = format!("d{}", rng.below(120));
                match rng.below(5) {
                    0 => {
                        store.remove(&id).unwrap();
                        model.remove(&id);
                    }
                    _ => {
                        let v = rng.vector(300, 30);
                        store.put(&id, &v).unwrap();
                        model.insert(id, v);
                    }
                }
                if step == 300 {
                    drop(store);
                    store = open();
                }
                if step % 7 == 0 {
                    let q = rng.vector(300, 12);
                    for k in [1, 5, 50] {
                        let want = brute(&model, &q, k);
                        let got = store.search(&q, k, &mut |_| true).unwrap();
                        if min_rows == usize::MAX {
                            let names = |h: &[(String, f64)]| {
                                h.iter().map(|p| p.0.clone()).collect::<Vec<_>>()
                            };
                            assert_eq!(names(&got), names(&want), "step {step} k {k}");
                        } else {
                            // Candidates come from the quantized index; scores are exact, so
                            // only a near-tie at the k-th place may swap members.
                            assert_eq!(got.len(), want.len(), "step {step} k {k}");
                            for (g, w) in got.iter().zip(&want) {
                                assert!(
                                    (g.1 - w.1).abs() <= 0.02 * w.1 + 1e-6,
                                    "step {step} k {k}: {g:?} vs {w:?}"
                                );
                            }
                        }
                    }
                }
            }
            assert_eq!(store.len(), model.len());
            if min_rows == 40 {
                assert!(store.index_built());
            }
        }
    }

    #[test]
    fn liveness_filter_draws_deeper_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let store = SparseStore::open(dir.path()).unwrap();
        for i in 0..50u32 {
            store
                .put(
                    &format!("d{i:02}"),
                    &SparseVector::new(vec![1], vec![100.0 - i as f32]),
                )
                .unwrap();
        }
        let q = SparseVector::new(vec![1], vec![1.0]);
        let hits = store.search(&q, 3, &mut |id| id >= "d40").unwrap();
        let got: Vec<&str> = hits.iter().map(|h| h.0.as_str()).collect();
        assert_eq!(got, ["d40", "d41", "d42"]);
    }

    #[test]
    fn torn_tail_is_dropped_and_survivors_stay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let v = |t: u32| SparseVector::new(vec![t], vec![1.0]);
        {
            let store = SparseStore::open(dir.path()).unwrap();
            store.put("a", &v(1)).unwrap();
            store.put("b", &v(2)).unwrap();
        }
        let whole = std::fs::read(&path).unwrap();
        let last_len = encode_put(
            "b",
            &Row {
                ids: [2u16].into(),
                weights: [1.0f32].into(),
            },
        )
        .len();
        for cut in 1..last_len {
            std::fs::write(&path, &whole[..whole.len() - cut]).unwrap();
            let store = SparseStore::open(dir.path()).unwrap();
            assert!(store.contains("a") && !store.contains("b"), "cut {cut}");
            store.put("c", &v(3)).unwrap();
            drop(store);
            let store = SparseStore::open(dir.path()).unwrap();
            assert!(store.contains("a") && store.contains("c"), "cut {cut}");
        }
        let mut flipped = whole.clone();
        let n = flipped.len();
        flipped[n - 1] ^= 0xff;
        std::fs::write(&path, &flipped).unwrap();
        assert!(!SparseStore::open(dir.path()).unwrap().contains("b"));
        let mut mid = whole.clone();
        mid[HEADER_LEN + FRAME_LEN + 1] ^= 0xff;
        std::fs::write(&path, &mid).unwrap();
        assert!(SparseStore::open(dir.path()).is_err());
    }

    #[test]
    fn foreign_and_future_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut h = header();
        h[8] = 2;
        std::fs::write(&path, h).unwrap();
        assert!(SparseStore::open(dir.path()).is_err());
        let mut h = header();
        h[0] = b'X';
        std::fs::write(&path, h).unwrap();
        assert!(SparseStore::open(dir.path()).is_err());
    }

    #[test]
    fn limits_hold_at_limit_minus_one_limit_and_limit_plus_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = SparseStore::open(dir.path()).unwrap();
        let limits = SparseLimits::default();
        let dense = |n: usize| SparseVector::new((0..n as u32).collect(), vec![1.0; n]);
        for n in [limits.max_doc_nnz - 1, limits.max_doc_nnz] {
            store.put("a", &dense(n)).unwrap();
        }
        assert!(store.put("a", &dense(limits.max_doc_nnz + 1)).is_err());
        for n in [limits.max_query_nnz - 1, limits.max_query_nnz] {
            store.search(&dense(n), 5, &mut |_| true).unwrap();
        }
        assert!(store
            .search(&dense(limits.max_query_nnz + 1), 5, &mut |_| true)
            .is_err());
        for t in [MAX_TERM_ID - 2, MAX_TERM_ID - 1] {
            store
                .put("t", &SparseVector::new(vec![t], vec![1.0]))
                .unwrap();
        }
        assert!(store
            .put("t", &SparseVector::new(vec![MAX_TERM_ID], vec![1.0]))
            .is_err());
        for bad in [-1.0f32, f32::NAN, f32::INFINITY] {
            assert!(store
                .put("w", &SparseVector::new(vec![1], vec![bad]))
                .is_err());
        }
        assert!(store
            .put("dup", &SparseVector::new(vec![1, 1], vec![1.0, 1.0]))
            .is_err());
        assert!(store
            .put("len", &SparseVector::new(vec![1, 2], vec![1.0]))
            .is_err());
        for k in [0, MAX_K + 1] {
            assert!(store.search(&dense(1), k, &mut |_| true).is_err());
        }
        assert!(!store.contains("w") && !store.contains("dup"));
    }

    #[test]
    fn total_limits_hold_at_the_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let limits = SparseLimits {
            max_total_nnz: 6,
            max_rows: 3,
            ..SparseLimits::default()
        };
        let store = SparseStore::open_with(dir.path(), limits, SparseTuning::default()).unwrap();
        let v = |n: usize| SparseVector::new((0..n as u32).collect(), vec![1.0; n]);
        store.put("a", &v(3)).unwrap();
        store.put("b", &v(2)).unwrap();
        assert!(store.put("c", &v(2)).is_err());
        store.put("c", &v(1)).unwrap();
        assert!(store.put("d", &v(1)).is_err());
        store.put("a", &v(2)).unwrap();
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn churn_compacts_the_log_without_changing_contents() {
        let dir = tempfile::tempdir().unwrap();
        let store = SparseStore::open(dir.path()).unwrap();
        let mut rng = Rng(7);
        let mut model = BTreeMap::new();
        for i in 0..4000 {
            let id = format!("d{}", i % 5);
            let v = rng.vector(200, 150);
            store.put(&id, &v).unwrap();
            model.insert(id, v);
        }
        let size = std::fs::metadata(dir.path().join(FILE_NAME)).unwrap().len();
        let live: u64 = model
            .iter()
            .map(|(id, v)| (FRAME_LEN + 7 + id.len() + 6 * v.ids.len()) as u64)
            .sum();
        assert!(size <= 2 * live + COMPACT_FLOOR + 4096, "{size} vs {live}");
        let q = rng.vector(200, 10);
        let expected = brute_ids(&model, &q, 5);
        assert_eq!(ids(&store, &q, 5), expected);
        drop(store);
        assert_eq!(
            ids(&SparseStore::open(dir.path()).unwrap(), &q, 5),
            expected
        );
    }

    #[test]
    fn nothing_is_created_until_a_vector_is_supplied() {
        let dir = tempfile::tempdir().unwrap();
        let store = SparseStore::open(dir.path()).unwrap();
        assert!(store
            .search(&SparseVector::new(vec![1], vec![1.0]), 5, &mut |_| true)
            .unwrap()
            .is_empty());
        assert!(!store.remove("x").unwrap());
        store
            .put("empty", &SparseVector::new(vec![1], vec![0.0]))
            .unwrap();
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    fn golden_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/sparse-docs-v1")
    }

    fn row(pairs: &[(u16, f32)]) -> Row {
        Row {
            ids: pairs.iter().map(|p| p.0).collect(),
            weights: pairs.iter().map(|p| p.1).collect(),
        }
    }

    /// The bytes a store holding a put, a replaced put, a delete and a put with the largest term id
    /// must have: pins the header, the framing and the record layouts.
    fn golden_bytes() -> Vec<u8> {
        let mut out = header().to_vec();
        out.extend(encode_put("alpha", &row(&[(3, 0.5), (17, 2.0)])));
        out.extend(encode_put("beta", &row(&[(1, 1.0)])));
        out.extend(encode_put(
            "alpha",
            &row(&[(3, 1.5), (17, 2.0), (9000, 0.25)]),
        ));
        out.extend(encode_delete("beta"));
        out.extend(encode_put("gamma\u{e9}", &row(&[(65535, 4.0)])));
        out
    }

    #[test]
    #[ignore = "writes tests/golden; run deliberately"]
    fn write_golden() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(golden_dir().join(FILE_NAME), golden_bytes()).unwrap();
    }

    /// The committed version-1 file is append-only history: it must keep opening, with the
    /// same rows, and the writer must keep producing the same bytes.
    #[test]
    fn golden_version_1_file_round_trips() {
        let committed = std::fs::read(golden_dir().join(FILE_NAME)).unwrap();
        assert_eq!(committed, golden_bytes());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), &committed).unwrap();
        let store = SparseStore::open(dir.path()).unwrap();
        assert_eq!(store.len(), 2);
        assert!(!store.contains("beta"));
        let q = SparseVector::new(vec![3, 9000, 65535], vec![1.0, 4.0, 0.5]);
        let hits = store.search(&q, 5, &mut |_| true).unwrap();
        assert_eq!(
            hits,
            vec![("alpha".to_owned(), 2.5), ("gamma\u{e9}".to_owned(), 2.0)]
        );
        let fresh = tempfile::tempdir().unwrap();
        let rebuilt = SparseStore::open(fresh.path()).unwrap();
        rebuilt
            .put("alpha", &SparseVector::new(vec![3, 17], vec![0.5, 2.0]))
            .unwrap();
        rebuilt
            .put("beta", &SparseVector::new(vec![1], vec![1.0]))
            .unwrap();
        rebuilt
            .put(
                "alpha",
                &SparseVector::new(vec![17, 9000, 3], vec![2.0, 0.25, 1.5]),
            )
            .unwrap();
        rebuilt.remove("beta").unwrap();
        rebuilt
            .put("gamma\u{e9}", &SparseVector::new(vec![65535], vec![4.0]))
            .unwrap();
        assert_eq!(
            std::fs::read(fresh.path().join(FILE_NAME)).unwrap(),
            committed
        );
    }
}
