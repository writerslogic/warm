// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Query-reinforced edge repair: an optional step after construction that searches the
//! graph with construction (train) queries, adds edges toward the true neighbours a fixed
//! beam missed, gives low in-degree vertices in-edges from their own nearest out-neighbours,
//! and can replace the upper layers with one complete layer over k-means medoids.
//!
//! Queries must be independent of evaluation data. A graph is only changed through
//! [`Graph::apply_query_repair`]; builds without it are the legacy builds.

use super::projection::validate_queries;
use super::{dot, invalid, Adjacency, Graph, Metric};
use crate::core::error::WarmError;
use rayon::prelude::*;
use std::collections::HashSet;

/// Upper bound on construction queries scored exactly (cost grows with `queries * n`).
const QUERY_LIMIT: usize = 32768;
/// Nearest reached vertices tried, in order, as the source of one repair edge.
const SOURCE_CHOICES: usize = 4;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct RepairParams {
    /// Result depth whose misses are repaired (1..=n).
    pub k: usize,
    /// Beam width at which train queries are searched for misses (at least `k`).
    pub ef: usize,
    /// Slots added to every list's capacity; 0 only fills slots the build left empty, so
    /// fixed-width encodings keep their size.
    pub extra_degree: usize,
    /// Add edges toward the missed true neighbours of failing train queries.
    pub failure_edges: bool,
    /// Give every vertex with fewer in-edges than this some from its out-neighbours; 0 is off.
    pub antihub_in_degree: usize,
    /// Replace the upper layers with one complete layer over this many k-means medoids;
    /// 0 keeps the build's layers.
    pub entry_points: usize,
    /// Matched control: keep every planned edge's source (so per-vertex counts, capacity and
    /// memory are identical) but point it at a seeded random vertex instead.
    pub random_targets: bool,
    pub seed: u64,
}

impl Default for RepairParams {
    fn default() -> Self {
        Self {
            k: 10,
            ef: 32,
            extra_degree: 0,
            failure_edges: true,
            antihub_in_degree: 0,
            entry_points: 0,
            random_targets: false,
            seed: 0x5EED,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct RepairStats {
    pub training_queries: usize,
    /// Train-query recall@k of the exact beam at `ef` before any change.
    pub train_recall_before: f64,
    pub failing_queries: usize,
    pub missed_neighbors: usize,
    pub failure_edges: usize,
    pub antihub_vertices: usize,
    pub antihub_edges: usize,
    /// Planned edges dropped because every candidate source list was full.
    pub skipped_full: usize,
    pub added_edges: usize,
    pub entry_points: usize,
    pub degree_capacity: usize,
}

/// splitmix64: a seeded stream for the k-means sample and the random control.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

impl Graph {
    /// Builds a graph and, when `repair` is given, applies [`Graph::apply_query_repair`].
    /// `None` is the legacy build.
    pub fn build_with_repair(
        data: &[f32],
        dim: usize,
        params: &super::BuildParams,
        repair: Option<(&[f32], &RepairParams)>,
    ) -> Result<Self, WarmError> {
        let Some((queries, options)) = repair else {
            return Self::build(data, dim, params);
        };
        validate_queries(queries, dim, params.metric)?;
        let mut graph = Self::build(data, dim, params)?;
        graph.apply_query_repair(queries, options)?;
        Ok(graph)
    }

    /// Applies query-reinforced repair with the first 32768 construction queries (graph
    /// coordinates), whose exact neighbours are found by scoring every base row. Every
    /// existing edge is kept; added edges go to the end of their lists. The result can be
    /// encoded and saved as a `VGraph`; its graph cache is refused.
    pub fn apply_query_repair(
        &mut self,
        queries: &[f32],
        params: &RepairParams,
    ) -> Result<RepairStats, WarmError> {
        validate_queries(queries, self.dim, self.metric)?;
        self.validate_repair(params)?;
        let count = (queries.len() / self.dim).min(QUERY_LIMIT);
        let queries = &queries[..count * self.dim];
        let truth = self.training_neighbors(queries, params.k)?;
        self.apply_query_repair_with_neighbors(queries, &truth, params)
    }

    /// [`Graph::apply_query_repair`] with the caller's exact neighbours of every query,
    /// `params.k` ids per query, nearest first. They must come from construction queries,
    /// never evaluation data.
    pub fn apply_query_repair_with_neighbors(
        &mut self,
        queries: &[f32],
        truth: &[u32],
        params: &RepairParams,
    ) -> Result<RepairStats, WarmError> {
        validate_queries(queries, self.dim, self.metric)?;
        let capacity = self.validate_repair(params)?;
        let count = queries.len() / self.dim;
        if count.checked_mul(params.k) != Some(truth.len())
            || truth.iter().any(|&id| id as usize >= self.n)
        {
            return Err(invalid(
                "repair neighbours need k ids inside the graph per query".into(),
            ));
        }
        let mut stats = RepairStats {
            training_queries: count,
            degree_capacity: capacity,
            ..RepairStats::default()
        };

        let mut added: Vec<Vec<u32>> = vec![Vec::new(); self.n];
        let room = |adj: &Adjacency, added: &[Vec<u32>], s: u32| {
            adj.get(s as usize).len() + added[s as usize].len() < capacity
        };
        let linked = |adj: &Adjacency, added: &[Vec<u32>], s: u32, t: u32| {
            adj.get(s as usize).contains(&t) || added[s as usize].contains(&t)
        };
        // Planned edges as (source, target), in a deterministic order.
        let mut planned: Vec<(u32, u32)> = Vec::new();

        let found = self.exact_beams(queries, params.ef)?;
        let mut hits = 0usize;
        let proposals: Vec<Vec<(u32, [u32; SOURCE_CHOICES], usize)>> = truth
            .par_chunks_exact(params.k)
            .zip(found.par_iter())
            .map(|(truth, beam)| {
                let top: HashSet<u32> = beam.iter().take(params.k).copied().collect();
                let rows = self.rows();
                truth
                    .iter()
                    .filter(|m| !top.contains(m))
                    .map(|&m| {
                        let mut near: Vec<(f32, u32)> = beam
                            .iter()
                            .filter(|&&u| u != m)
                            .map(|&u| (rows.d2(u, m), u))
                            .collect();
                        near.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                        let mut sources = [u32::MAX; SOURCE_CHOICES];
                        for (slot, &(_, u)) in sources.iter_mut().zip(&near) {
                            *slot = u;
                        }
                        (m, sources, near.len().min(SOURCE_CHOICES))
                    })
                    .collect()
            })
            .collect();
        for misses in &proposals {
            hits += params.k - misses.len();
            if !misses.is_empty() {
                stats.failing_queries += 1;
            }
            stats.missed_neighbors += misses.len();
        }
        stats.train_recall_before = hits as f64 / (count * params.k) as f64;
        if params.failure_edges {
            for misses in &proposals {
                for &(m, sources, len) in misses {
                    if sources[..len]
                        .iter()
                        .any(|&s| linked(&self.adj, &added, s, m))
                    {
                        continue;
                    }
                    match sources[..len].iter().find(|&&s| room(&self.adj, &added, s)) {
                        Some(&s) => {
                            added[s as usize].push(m);
                            planned.push((s, m));
                            stats.failure_edges += 1;
                        }
                        None => stats.skipped_full += 1,
                    }
                }
            }
        }

        if params.antihub_in_degree > 0 {
            let mut in_degree = vec![0usize; self.n];
            for (v, list) in self.adj.iter().enumerate() {
                for &u in list.iter().chain(&added[v]) {
                    in_degree[u as usize] += 1;
                }
            }
            for v in 0..self.n as u32 {
                if in_degree[v as usize] >= params.antihub_in_degree {
                    continue;
                }
                stats.antihub_vertices += 1;
                for i in 0..self.adj.get(v as usize).len() {
                    if in_degree[v as usize] >= params.antihub_in_degree {
                        break;
                    }
                    let u = self.adj.get(v as usize)[i];
                    if linked(&self.adj, &added, u, v) {
                        continue;
                    }
                    if !room(&self.adj, &added, u) {
                        stats.skipped_full += 1;
                        continue;
                    }
                    added[u as usize].push(v);
                    planned.push((u, v));
                    in_degree[v as usize] += 1;
                    stats.antihub_edges += 1;
                }
            }
        }

        if params.random_targets && self.n > 1 {
            let mut stream = Stream(params.seed ^ 0xC0_47_20_1E);
            added.iter_mut().for_each(Vec::clear);
            for &(s, _) in &planned {
                let full = self.adj.get(s as usize).len() + added[s as usize].len() + 1 >= self.n;
                if full {
                    continue;
                }
                loop {
                    let t = stream.below(self.n) as u32;
                    if t != s && !linked(&self.adj, &added, s, t) {
                        added[s as usize].push(t);
                        break;
                    }
                }
            }
        }

        stats.added_edges = added.iter().map(Vec::len).sum();
        if stats.added_edges > 0 || capacity != self.degree {
            let mut replacement = Adjacency::new(self.n, capacity);
            let mut combined = Vec::with_capacity(capacity);
            for (v, extra) in added.iter().enumerate() {
                combined.clear();
                combined.extend_from_slice(self.adj.get(v));
                combined.extend_from_slice(extra);
                replacement.set(v, &combined);
            }
            self.adj = replacement;
            self.degree = capacity;
        }
        if params.entry_points > 0 {
            let medoids = self.kmeans_medoids(params.entry_points, params.seed);
            let complete = (0..medoids.len() as u32)
                .map(|i| (0..medoids.len() as u32).filter(|&j| j != i).collect())
                .collect();
            stats.entry_points = medoids.len();
            self.entry = medoids[0];
            self.upper_ids = medoids;
            self.layers = vec![complete];
        }
        self.repaired = true;
        Ok(stats)
    }

    /// The list capacity after repair, once `params` are checked against the graph.
    fn validate_repair(&self, params: &RepairParams) -> Result<usize, WarmError> {
        if params.k == 0 || params.k > self.n || params.ef < params.k {
            return Err(invalid("repair requires 1 <= k <= n and ef >= k".into()));
        }
        if params.entry_points > self.n || params.entry_points > 4096 {
            return Err(invalid(
                "repair entry points must be at most n and 4096".into(),
            ));
        }
        self.degree
            .checked_add(params.extra_degree)
            .filter(|&c| c <= 65_536)
            .ok_or_else(|| invalid("repair degree capacity overflows".into()))
    }

    fn query_distance(&self, query: &[f32], id: u32) -> f32 {
        let row = &self.vectors[id as usize * self.dim..(id as usize + 1) * self.dim];
        let product = dot(query, row);
        match self.metric {
            Metric::Angular => -product,
            Metric::Euclidean => self.norms[id as usize] - 2.0 * product,
        }
    }

    /// The `ef` best vertices, nearest first, of an exact-distance beam search per query,
    /// entered through the upper layers as the serving index enters them.
    fn exact_beams(&self, queries: &[f32], ef: usize) -> Result<Vec<Vec<u32>>, WarmError> {
        let ef = ef.min(self.n);
        Ok(queries
            .par_chunks_exact(self.dim)
            .map_init(
                || (vec![0u32; self.n], 0u32),
                |(seen, epoch), query| {
                    *epoch = epoch.wrapping_add(1);
                    if *epoch == 0 {
                        seen.fill(0);
                        *epoch = 1;
                    }
                    self.exact_beam(query, ef, seen, *epoch)
                },
            )
            .collect())
    }

    fn exact_beam(&self, query: &[f32], ef: usize, seen: &mut [u32], epoch: u32) -> Vec<u32> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        #[derive(Clone, Copy, PartialEq)]
        struct Scored(f32, u32);
        impl Eq for Scored {}
        impl PartialOrd for Scored {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for Scored {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.0.total_cmp(&other.0).then(self.1.cmp(&other.1))
            }
        }
        let score = |id: u32| Scored(self.query_distance(query, id), id);
        let mut entry = self.entry;
        if !self.layers.is_empty() {
            let mut p = 0usize;
            let mut best = score(self.upper_ids[0]);
            for layer in self.layers.iter().rev() {
                loop {
                    let mut moved = false;
                    for &u in &layer[p] {
                        let candidate = score(self.upper_ids[u as usize]);
                        if candidate < best {
                            best = candidate;
                            p = u as usize;
                            moved = true;
                        }
                    }
                    if !moved {
                        break;
                    }
                }
            }
            entry = self.upper_ids[p];
        }
        let mut candidates = BinaryHeap::new();
        let mut best = BinaryHeap::new();
        let first = score(entry);
        candidates.push(Reverse(first));
        best.push(first);
        seen[entry as usize] = epoch;
        while let Some(Reverse(candidate)) = candidates.pop() {
            if best.len() >= ef && best.peek().is_some_and(|&worst| candidate > worst) {
                break;
            }
            for &id in self.adj.get(candidate.1 as usize) {
                if seen[id as usize] == epoch {
                    continue;
                }
                seen[id as usize] = epoch;
                let s = score(id);
                if best.len() < ef {
                    best.push(s);
                } else {
                    let mut worst = best.peek_mut().expect("positive beam width");
                    if s >= *worst {
                        continue;
                    }
                    *worst = s;
                }
                candidates.push(Reverse(s));
            }
        }
        let mut ranked = best.into_vec();
        ranked.sort_unstable();
        ranked.into_iter().map(|s| s.1).collect()
    }

    /// Distinct medoids of `k` clusters from ten Lloyd iterations over a seeded sample of at
    /// most `64 k` rows (squared Euclidean on the stored rows, which are unit for angular).
    fn kmeans_medoids(&self, k: usize, seed: u64) -> Vec<u32> {
        let mut stream = Stream(seed ^ 0x4B_4D_45_41_4E_53);
        let sample_len = self.n.min(k.saturating_mul(64)).max(k);
        let mut sample: Vec<u32> = (0..self.n as u32).collect();
        for i in 0..sample_len {
            let j = i + stream.below(self.n - i);
            sample.swap(i, j);
        }
        sample.truncate(sample_len);
        let dim = self.dim;
        let row = |id: u32| &self.vectors[id as usize * dim..(id as usize + 1) * dim];
        let d2 =
            |a: &[f32], b: &[f32]| -> f32 { a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum() };
        let mut centroids: Vec<f32> = sample[..k]
            .iter()
            .flat_map(|&id| row(id))
            .copied()
            .collect();
        let nearest = |centroids: &[f32], x: &[f32]| -> usize {
            centroids
                .chunks_exact(dim)
                .enumerate()
                .map(|(c, centre)| (d2(centre, x), c))
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map_or(0, |(_, c)| c)
        };
        for _ in 0..10 {
            let assigned: Vec<usize> = sample
                .par_iter()
                .map(|&id| nearest(&centroids, row(id)))
                .collect();
            let mut sums = vec![0f64; k * dim];
            let mut counts = vec![0usize; k];
            for (&id, &c) in sample.iter().zip(&assigned) {
                counts[c] += 1;
                for (s, &v) in sums[c * dim..(c + 1) * dim].iter_mut().zip(row(id)) {
                    *s += f64::from(v);
                }
            }
            for c in 0..k {
                if counts[c] > 0 {
                    for (dst, &s) in centroids[c * dim..(c + 1) * dim]
                        .iter_mut()
                        .zip(&sums[c * dim..(c + 1) * dim])
                    {
                        *dst = (s / counts[c] as f64) as f32;
                    }
                }
            }
        }
        let mut medoids = Vec::with_capacity(k);
        for centre in centroids.chunks_exact(dim) {
            let best = sample
                .iter()
                .map(|&id| (d2(centre, row(id)), id))
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, id)| id);
            if let Some(id) = best {
                if !medoids.contains(&id) {
                    medoids.push(id);
                }
            }
        }
        medoids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::qgraph::{BuildParams, Residency, SearchKnobs, VGraph, VSearchParams};

    fn data(n: usize, dim: usize, seed: u64) -> Vec<f32> {
        let mut stream = Stream(seed);
        (0..n * dim)
            .map(|_| (stream.next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5)
            .collect()
    }

    fn params(metric: Metric) -> BuildParams {
        BuildParams {
            metric,
            degree: 8,
            build_ef: 16,
            seed: 7,
            ..BuildParams::default()
        }
    }

    #[test]
    fn absent_repair_preserves_graph_cache_and_index_bytes() {
        let directory = tempfile::tempdir().unwrap();
        for metric in [Metric::Angular, Metric::Euclidean] {
            let rows = data(300, 6, 1);
            let params = params(metric);
            let legacy = Graph::build(&rows, 6, &params).unwrap();
            let optional = Graph::build_with_repair(&rows, 6, &params, None).unwrap();
            let mut bytes = Vec::new();
            for (id, graph) in [legacy, optional].iter().enumerate() {
                let cache = directory.path().join(format!("{metric:?}-graph-{id}"));
                graph.save(&cache).unwrap();
                let index = directory.path().join(format!("{metric:?}-index-{id}"));
                VGraph::from_graph_with(graph, 8, 8, true, false, 3, false)
                    .unwrap()
                    .save(&index)
                    .unwrap();
                bytes.push((std::fs::read(cache).unwrap(), std::fs::read(index).unwrap()));
            }
            assert!(bytes[0] == bytes[1], "{metric:?} bytes differ");
        }
    }

    #[test]
    fn repair_keeps_edges_respects_capacity_and_refuses_the_graph_cache() {
        let rows = data(400, 6, 2);
        let queries = data(64, 6, 3);
        let params = params(Metric::Euclidean);
        let base = Graph::build(&rows, 6, &params).unwrap();
        let options = RepairParams {
            ef: 10,
            extra_degree: 2,
            antihub_in_degree: 3,
            entry_points: 8,
            ..RepairParams::default()
        };
        let mut repaired = Graph::build(&rows, 6, &params).unwrap();
        let stats = repaired.apply_query_repair(&queries, &options).unwrap();
        assert_eq!(repaired.degree, 10);
        assert_eq!(stats.added_edges, stats.failure_edges + stats.antihub_edges);
        let mut total = 0;
        for v in 0..rows.len() / 6 {
            let (old, new) = (base.adj.get(v), repaired.adj.get(v));
            assert!(new.len() <= 10 && new.starts_with(old));
            let mut unique = new.to_vec();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), new.len());
            assert!(!new.contains(&(v as u32)));
            total += new.len() - old.len();
        }
        assert_eq!(total, stats.added_edges);
        assert_eq!(repaired.upper_ids.len(), stats.entry_points);
        assert!(stats.entry_points > 0 && repaired.layers.len() == 1);

        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("cache");
        std::fs::write(&cache, b"keep").unwrap();
        assert_eq!(
            repaired.save(&cache).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(std::fs::read(&cache).unwrap(), b"keep");
        let index = VGraph::from_graph(&repaired, true).unwrap();
        let path = directory.path().join("index");
        index.save(&path).unwrap();
        let opened = VGraph::open(&path, Residency::Heap).unwrap();
        let search = VSearchParams {
            ef: 12,
            rerank: 12,
            ..VSearchParams::default()
        };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for query in queries.as_chunks::<6>().0 {
            index
                .searcher_with(SearchKnobs::OFF)
                .search(query, 5, search, &mut a)
                .unwrap();
            opened
                .searcher_with(SearchKnobs::OFF)
                .search(query, 5, search, &mut b)
                .unwrap();
            assert_eq!(a, b);
        }
    }

    #[test]
    fn random_control_matches_per_source_counts() {
        let rows = data(400, 6, 4);
        let queries = data(64, 6, 5);
        let params = params(Metric::Angular);
        let options = RepairParams {
            ef: 10,
            antihub_in_degree: 2,
            extra_degree: 2,
            ..RepairParams::default()
        };
        let mut repaired = Graph::build(&rows, 6, &params).unwrap();
        let mut control = Graph::build(&rows, 6, &params).unwrap();
        let first = repaired.apply_query_repair(&queries, &options).unwrap();
        let second = control
            .apply_query_repair(
                &queries,
                &RepairParams {
                    random_targets: true,
                    ..options.clone()
                },
            )
            .unwrap();
        assert!(first.added_edges > 0);
        assert_eq!(first.added_edges, second.added_edges);
        assert_eq!(repaired.degree, control.degree);
        for v in 0..rows.len() / 6 {
            assert_eq!(repaired.adj.get(v).len(), control.adj.get(v).len());
        }
        assert_ne!(repaired.structure_hash(), control.structure_hash());
    }

    #[test]
    fn invalid_repair_leaves_the_graph_unchanged() {
        let rows = data(100, 4, 6);
        let params = params(Metric::Angular);
        let mut graph = Graph::build(&rows, 4, &params).unwrap();
        let hash = graph.structure_hash();
        for (queries, options) in [
            (vec![0.0; 4], RepairParams::default()),
            (vec![1.0; 3], RepairParams::default()),
            (
                vec![1.0; 4],
                RepairParams {
                    ef: 5,
                    ..RepairParams::default()
                },
            ),
            (
                vec![1.0; 4],
                RepairParams {
                    entry_points: 101,
                    ..RepairParams::default()
                },
            ),
        ] {
            assert!(graph.apply_query_repair(&queries, &options).is_err());
        }
        assert_eq!(graph.structure_hash(), hash);
        assert!(!graph.repaired);
    }
}
