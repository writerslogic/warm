// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The vertex graph behind `memorize_vector` and `query_vector`.
//!
//! `memorize_vector` still commits the sparse code to the journal, so every sparse-path
//! surface (`query`, relations, forget, replay) keeps working on the same ids. The dense
//! vector additionally goes to an [`UpdatableVGraph`] in `dense-vectors/`, and
//! `query_vector` is answered from the graph.
//!
//! Layout: `dense-ids.log` is an append-only list of `u16` little-endian length plus UTF-8
//! bytes; entry `i` names graph id `i` and is synced before the graph batch that uses it, so
//! an unreferenced entry is harmless. The graph directory is created by the first
//! `memorize_vector`; a store without it (every store written before this module) answers
//! `query_vector` from the sparse path until vectors are re-memorized, which moves them to the
//! graph. Once the graph exists, sparse-only vectors fill the result only when the graph
//! returns fewer than `k` hits, because their scores are not comparable with cosine. A graph row whose id no longer has a sparse vector (a crash between the journal
//! commit and the graph commit) is dropped at query time.
//!
//! Not written for encrypted stores: the graph files are plaintext.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};

use super::WarmCore;
use crate::core::entangled::EntangledHVec;
use crate::core::qgraph::{
    BuildParams, ConsolidationMode, ConsolidationPolicy, UpdatableVGraph, VSearchParams,
};
use crate::core::sync::RwLock;
use crate::RetrievalResult;

const DIR: &str = "dense-vectors";
const NAMES: &str = "dense-ids.log";
const MAX_DIM: usize = 65536;

pub(super) struct DenseVectors {
    dir: PathBuf,
    names_path: PathBuf,
    inner: RwLock<Option<Inner>>,
}

struct Inner {
    graph: UpdatableVGraph,
    dim: usize,
    slots: HashMap<String, u32>,
    names: Vec<String>,
    log: File,
}

fn build_params(dim: usize) -> BuildParams {
    BuildParams {
        degree: if dim <= 128 { 32 } else { 64 },
        build_ef: 200,
        ..BuildParams::default()
    }
}

fn read_names(path: &Path) -> Result<(Vec<String>, u64)> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let (mut names, mut at) = (Vec::new(), 0usize);
    while at + 2 <= bytes.len() {
        let len = u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
        let Some(raw) = bytes.get(at + 2..at + 2 + len) else {
            break;
        };
        let Ok(name) = std::str::from_utf8(raw) else {
            break;
        };
        names.push(name.to_owned());
        at += 2 + len;
    }
    Ok((names, at as u64))
}

fn encode_names<'a>(names: impl IntoIterator<Item = &'a String>) -> Vec<u8> {
    let mut out = Vec::new();
    for name in names {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }
    out
}

impl DenseVectors {
    pub(super) fn open(store: &Path) -> Result<Self> {
        let dir = store.join(DIR);
        let names_path = store.join(NAMES);
        let this = Self {
            dir,
            names_path,
            inner: RwLock::new(None),
        };
        if this.dir.join("CURRENT").exists() {
            let graph = UpdatableVGraph::open(&this.dir).context("open dense vector index")?;
            let (names, valid) = read_names(&this.names_path).context("read dense id log")?;
            let truncate = OpenOptions::new().write(true).open(&this.names_path)?;
            if truncate.metadata()?.len() != valid {
                truncate.set_len(valid)?;
                truncate.sync_all()?;
            }
            let log = OpenOptions::new().append(true).open(&this.names_path)?;
            let slots = names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), i as u32))
                .collect();
            *this.inner.write() = Some(Inner {
                dim: graph.dim(),
                graph,
                slots,
                names,
                log,
            });
        } else {
            if this.dir.exists() {
                std::fs::remove_dir_all(&this.dir)?;
            }
            if this.names_path.exists() {
                std::fs::remove_file(&this.names_path)?;
            }
        }
        Ok(this)
    }

    pub(super) fn is_active(&self) -> bool {
        self.inner.read().is_some()
    }

    /// Inserts or replaces rows; the last entry of a repeated id wins.
    pub(super) fn upsert(&self, items: Vec<(String, Vec<f32>)>) -> Result<()> {
        let Some(first) = items.first() else {
            return Ok(());
        };
        let dim = first.1.len();
        ensure!(
            dim > 0 && dim <= MAX_DIM && items.iter().all(|(_, v)| v.len() == dim),
            "dense vectors must share one dimension in 1..={MAX_DIM}"
        );
        let mut position: HashMap<&str, usize> = HashMap::with_capacity(items.len());
        let mut unique: Vec<usize> = Vec::with_capacity(items.len());
        for (i, (id, _)) in items.iter().enumerate() {
            match position.insert(id.as_str(), unique.len()) {
                Some(old) => unique[old] = i,
                None => unique.push(i),
            }
        }
        let mut guard = self.inner.write();
        let Some(inner) = guard.as_mut() else {
            let names: Vec<String> = unique.iter().map(|&i| items[i].0.clone()).collect();
            let data: Vec<f32> = unique
                .iter()
                .flat_map(|&i| items[i].1.iter().copied())
                .collect();
            let mut log = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&self.names_path)?;
            log.write_all(&encode_names(&names))?;
            log.sync_all()?;
            let graph = match UpdatableVGraph::create(&self.dir, &data, dim, &build_params(dim)) {
                Ok(graph) => graph,
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&self.dir);
                    let _ = std::fs::remove_file(&self.names_path);
                    return Err(error).context("build dense vector index");
                }
            };
            let log = OpenOptions::new().append(true).open(&self.names_path)?;
            let slots = names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), i as u32))
                .collect();
            *guard = Some(Inner {
                graph,
                dim,
                slots,
                names,
                log,
            });
            return Ok(());
        };
        ensure!(
            dim == inner.dim,
            "dense vector dimension {dim} does not match the index dimension {}",
            inner.dim
        );
        let mut fresh = Vec::new();
        for &i in &unique {
            let id = &items[i].0;
            if !inner.slots.contains_key(id) {
                fresh.push(id.clone());
            }
        }
        ensure!(
            inner.names.len() + fresh.len() < u32::MAX as usize,
            "dense vector id space exhausted"
        );
        if !fresh.is_empty() {
            inner.log.write_all(&encode_names(&fresh))?;
            inner.log.sync_data()?;
            for id in fresh {
                inner.slots.insert(id.clone(), inner.names.len() as u32);
                inner.names.push(id);
            }
        }
        let (mut put_ids, mut put) = (Vec::new(), Vec::new());
        let (mut new_ids, mut new) = (Vec::new(), Vec::new());
        for &i in &unique {
            let slot = inner.slots[&items[i].0];
            if inner.graph.contains(slot) {
                put_ids.push(slot);
                put.extend_from_slice(&items[i].1);
            } else {
                new_ids.push(slot);
                new.extend_from_slice(&items[i].1);
            }
        }
        let capacity = inner.graph.batch_capacity();
        for (ids, rows, replace) in [(&put_ids, &put, true), (&new_ids, &new, false)] {
            for (id_chunk, row_chunk) in ids.chunks(capacity).zip(rows.chunks(capacity * dim)) {
                if replace {
                    inner.graph.replace_batch(id_chunk, row_chunk)?;
                } else {
                    inner.graph.insert_batch(id_chunk, row_chunk)?;
                }
                inner.graph.commit()?;
            }
        }
        Self::consolidate_if_needed(&inner.graph)
    }

    fn consolidate_if_needed(graph: &UpdatableVGraph) -> Result<()> {
        if graph.needs_consolidation(ConsolidationPolicy::default())? {
            graph.consolidate_with(ConsolidationMode::Merge { rebuild_after: 2.0 })?;
        }
        Ok(())
    }

    pub(super) fn remove(&self, id: &str) -> Result<()> {
        {
            let guard = self.inner.read();
            let Some(inner) = guard.as_ref() else {
                return Ok(());
            };
            if !inner
                .slots
                .get(id)
                .is_some_and(|&s| inner.graph.contains(s))
            {
                return Ok(());
            }
        }
        let guard = self.inner.write();
        let Some(inner) = guard.as_ref() else {
            return Ok(());
        };
        if let Some(&slot) = inner.slots.get(id) {
            if inner.graph.contains(slot) {
                inner.graph.delete(slot)?;
                inner.graph.commit()?;
                Self::consolidate_if_needed(&inner.graph)?;
            }
        }
        Ok(())
    }

    /// The `k` nearest rows by cosine, best first; `None` when no index exists.
    pub(super) fn search(&self, query: &[f32], k: usize) -> Result<Option<Vec<(String, f64)>>> {
        let guard = self.inner.read();
        let Some(inner) = guard.as_ref() else {
            return Ok(None);
        };
        ensure!(
            query.len() == inner.dim,
            "dense query dimension {} does not match the index dimension {}",
            query.len(),
            inner.dim
        );
        let norm = query
            .iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum::<f64>()
            .sqrt();
        let params = VSearchParams {
            ef: 128.max(k),
            rerank: 64.max(k),
            ..VSearchParams::default()
        };
        let slots = inner.graph.search(query, k, params)?;
        let mut hits = Vec::with_capacity(slots.len());
        for slot in slots {
            let Some(row) = inner.graph.vector(slot)? else {
                continue;
            };
            let dot: f64 = query
                .iter()
                .zip(&row)
                .map(|(a, b)| f64::from(*a) * f64::from(*b))
                .sum();
            hits.push((inner.names[slot as usize].clone(), dot / norm));
        }
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(Some(hits))
    }
}

impl WarmCore {
    fn dense_graph_enabled(&self) -> bool {
        !self.config.security.encryption_enabled
    }

    /// Drops the graph row of an id whose sparse vector was replaced or deleted.
    pub(super) fn forget_dense(&self, id: &str) {
        if self.dense.is_active() {
            if let Err(error) = self.dense.remove(id) {
                tracing::warn!("dense vector removal for {id} failed: {error:#}");
            }
        }
    }

    /// Converts a dense f32 vector to sparse, memorizes it, and indexes the dense vector in
    /// the vertex graph that `query_vector` searches.
    pub fn memorize_vector(&self, id: String, dense: &[f32]) -> Result<()> {
        self.memorize_vectors(vec![(id, dense.to_vec())])
    }

    /// [`Self::memorize_vector`] for many vectors: encoded in parallel, journaled in
    /// transactions of up to 4096, and added to the graph in one build when it is new.
    pub fn memorize_vectors(&self, items: Vec<(String, Vec<f32>)>) -> Result<()> {
        use rayon::prelude::*;
        for (id, dense) in &items {
            super::mutation::validate_public_id(id)?;
            ensure!(
                !id.starts_with(crate::core::documents::CHUNK_PREFIX),
                "chunk IDs are reserved for document ingestion"
            );
            self.validate_dense(dense)?;
        }
        let dimensions = self.dimensions;
        for batch in items.chunks(4096) {
            let mutations: Vec<_> = batch
                .par_iter()
                .map(|(id, dense)| super::Mutation::Vector {
                    id: id.clone(),
                    vector: EntangledHVec::from_dense(dense, dimensions),
                })
                .collect();
            self.commit(&mutations)?;
        }
        if self.dense_graph_enabled() {
            self.dense.upsert(items)?;
        }
        Ok(())
    }

    pub fn query_vector(&self, dense: &[f32], k: u32) -> Result<Vec<RetrievalResult>> {
        self.validate_dense(dense)?;
        let wanted = k.min(10000) as usize;
        let mut results = Vec::new();
        if wanted > 0 {
            if let Some(hits) = self.dense.search(dense, wanted)? {
                let _transaction = self.mutation_gate.read();
                let shards = self.shards.read();
                results.extend(
                    hits.into_iter()
                        .filter(|(id, _)| shards.get_vector(id).is_some())
                        .map(|(id, similarity)| RetrievalResult { id, similarity }),
                );
            }
        }
        if results.len() >= wanted && wanted > 0 {
            return Ok(results);
        }
        let sparse = EntangledHVec::from_dense(dense, self.dimensions);
        for hit in self.query(&sparse, k)? {
            if results.len() >= wanted {
                break;
            }
            if !results.iter().any(|r| r.id == hit.id) {
                results.push(hit);
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPARSE_DIM: u32 = 1024;
    const DIM: usize = 24;

    fn vectors(n: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut state = seed;
        (0..n)
            .map(|_| {
                (0..DIM)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        (state >> 40) as f32 / (1u64 << 23) as f32 - 1.0
                    })
                    .collect()
            })
            .collect()
    }

    fn cosine(a: &[f32], b: &[f32]) -> f64 {
        let dot: f64 = a
            .iter()
            .zip(b)
            .map(|(x, y)| f64::from(*x) * f64::from(*y))
            .sum();
        let norm = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
        dot / (norm(a) * norm(b))
    }

    fn exact(data: &[Vec<f32>], query: &[f32], k: usize) -> Vec<String> {
        let mut scored: Vec<_> = data
            .iter()
            .enumerate()
            .map(|(i, v)| (cosine(query, v), format!("v{i}")))
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.into_iter().take(k).map(|(_, id)| id).collect()
    }

    #[test]
    fn store_without_a_dense_index_opens_and_answers_from_the_sparse_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().display().to_string();
        let data = vectors(40, 7);
        {
            let core = WarmCore::new(SPARSE_DIM, Some(path.clone()), None).unwrap();
            for (i, v) in data.iter().enumerate() {
                core.memorize(
                    format!("v{i}"),
                    EntangledHVec::from_dense(v, SPARSE_DIM as usize),
                )
                .unwrap();
            }
        }
        let core = WarmCore::new(SPARSE_DIM, Some(path.clone()), None).unwrap();
        assert!(!dir.path().join(DIR).exists());
        let hits = core.query_vector(&data[3], 3).unwrap();
        assert_eq!(hits[0].id, "v3");
        core.memorize_vector("fresh".into(), &data[9]).unwrap();
        let hits = core.query_vector(&data[9], 5).unwrap();
        assert_eq!(hits.len(), 5);
        assert!(hits.iter().any(|h| h.id == "fresh"));
        assert!(hits.iter().any(|h| h.id == "v9"));
        drop(core);
        let core = WarmCore::new(SPARSE_DIM, Some(path), None).unwrap();
        assert!(core
            .query_vector(&data[3], 5)
            .unwrap()
            .iter()
            .any(|h| h.id == "v3"));
    }

    #[test]
    fn query_vector_matches_the_graph_index_across_reopen_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().display().to_string();
        let data = vectors(400, 11);
        let queries = vectors(25, 99);
        let core = WarmCore::new(SPARSE_DIM, Some(path.clone()), None).unwrap();
        let items: Vec<_> = data[..300]
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("v{i}"), v.clone()))
            .collect();
        core.memorize_vectors(items).unwrap();
        for (i, v) in data.iter().enumerate().skip(300) {
            core.memorize_vector(format!("v{i}"), v).unwrap();
        }
        let check = |core: &WarmCore| {
            let mut found = 0;
            for q in &queries {
                let truth = exact(&data, q, 5);
                let hits = core.query_vector(q, 5).unwrap();
                assert_eq!(hits.len(), 5);
                for hit in &hits {
                    let i: usize = hit.id[1..].parse().unwrap();
                    assert!((hit.similarity - cosine(q, &data[i])).abs() < 1e-5);
                }
                assert!(hits.windows(2).all(|w| w[0].similarity >= w[1].similarity));
                found += hits.iter().filter(|h| truth.contains(&h.id)).count();
            }
            assert!(found >= 24 * 5, "graph recall {found}/{}", 25 * 5);
        };
        check(&core);
        let before: Vec<_> = queries
            .iter()
            .map(|q| core.query_vector(q, 5).unwrap())
            .collect();
        drop(core);
        let core = WarmCore::new(SPARSE_DIM, Some(path), None).unwrap();
        for (q, old) in queries.iter().zip(&before) {
            let now = core.query_vector(q, 5).unwrap();
            assert_eq!(
                now.iter().map(|h| &h.id).collect::<Vec<_>>(),
                old.iter().map(|h| &h.id).collect::<Vec<_>>()
            );
        }
        let top = core.query_vector(&queries[0], 1).unwrap()[0].id.clone();
        core.commit(&[crate::core::engine::Mutation::Delete { id: top.clone() }])
            .unwrap();
        assert!(core
            .query_vector(&queries[0], 5)
            .unwrap()
            .iter()
            .all(|h| h.id != top));
    }
}
