// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Filtered search: the nearest rows among an allowed set of original ids.
//!
//! A selective filter is answered by scoring the allowed rows directly (the traversal codes
//! estimate every allowed row, the best few are re-scored with the float query and the
//! residual); a permissive one by a wider ordinary search whose results are kept when allowed,
//! widened until `k` pass. The routing depends only on how many rows are allowed, so a result
//! never depends on which path answered it beyond the index's own approximation.

use super::{key, key_id, query_sum, SearchScratch, VGraph, VSearchParams, VSearcher};
use crate::core::error::WarmError;
use crate::core::qgraph::kernels::prefetch_bytes;

/// Allowed sets at most this large are scored directly.
const SCAN_ROWS: usize = 50_000;
/// Allowed sets smaller than `1 / SCAN_FRACTION` of the index are scored directly.
const SCAN_FRACTION: usize = 20;
/// Rows prefetched ahead of the scoring cursor.
const AHEAD: usize = 8;

/// The original ids a filtered search may return.
#[derive(Clone, Copy, Debug)]
pub enum Allow<'a> {
    /// Ids `start..end`.
    Range { start: u32, end: u32 },
    /// Bit `i % 64` of word `i / 64` allows id `i`; ids past the mask are not allowed.
    Mask(&'a [u64]),
}

impl Allow<'_> {
    /// Whether original id `id` is allowed.
    #[inline]
    pub fn contains(&self, id: u32) -> bool {
        match *self {
            Self::Range { start, end } => id >= start && id < end,
            Self::Mask(words) => words
                .get((id >> 6) as usize)
                .is_some_and(|w| w >> (id & 63) & 1 == 1),
        }
    }

    /// How many ids below `n` are allowed.
    pub fn count(&self, n: usize) -> usize {
        match *self {
            Self::Range { start, end } => (end as usize).min(n).saturating_sub(start as usize),
            Self::Mask(words) => {
                let whole = (n / 64).min(words.len());
                let mut count: usize = words[..whole].iter().map(|w| w.count_ones() as usize).sum();
                if !n.is_multiple_of(64) && whole < words.len() && whole == n / 64 {
                    count += (words[whole] & ((1u64 << (n % 64)) - 1)).count_ones() as usize;
                }
                count
            }
        }
    }

    /// Calls `f` for every allowed id below `n`, ascending.
    fn for_each(&self, n: usize, mut f: impl FnMut(u32)) {
        match *self {
            Self::Range { start, end } => {
                for id in start..(end as usize).min(n) as u32 {
                    f(id);
                }
            }
            Self::Mask(words) => {
                for (w, &word) in words.iter().enumerate().take(n.div_ceil(64)) {
                    let mut bits = word;
                    while bits != 0 {
                        let id = w * 64 + bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        if id < n {
                            f(id as u32);
                        }
                    }
                }
            }
        }
    }
}

impl VGraph {
    /// The stored vertex of every original id; only for renumbered indexes.
    fn vertex_of(&self) -> &[u32] {
        self.inverse.get_or_init(|| {
            let mut inverse = vec![0u32; self.n];
            for (vertex, &original) in self.ids.iter().enumerate() {
                if let Some(slot) = inverse.get_mut(original as usize) {
                    *slot = vertex as u32;
                }
            }
            inverse
        })
    }
}

impl VSearcher<'_> {
    /// Writes the ids of the (approximate) `k` nearest allowed vectors into `out`, nearest
    /// first (fewer when fewer are allowed), and returns the number of distance estimates.
    pub fn search_allowed(
        &mut self,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        allow: &Allow<'_>,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        self.scratch.search_allowed(
            self.index,
            query,
            k,
            params,
            allow,
            (SCAN_ROWS, SCAN_FRACTION),
            out,
        )
    }
}

impl SearchScratch {
    /// `scan` is (rows, fraction): allowed sets of at most `rows` rows, or under `1 / fraction`
    /// of the index, are scored directly.
    #[allow(clippy::too_many_arguments)]
    fn search_allowed(
        &mut self,
        g: &VGraph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        allow: &Allow<'_>,
        scan: (usize, usize),
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        Self::validate_query(g, query)?;
        out.clear();
        let count = allow.count(g.n);
        let k = k.min(count);
        if k == 0 {
            return Ok(0);
        }
        if count <= scan.0 || count < g.n / scan.1.max(1) {
            return Ok(self.scan_allowed(g, query, k, params, count, allow, out));
        }
        let rate = count as f64 / g.n as f64;
        let mut take = (((k as f64 / rate) * 1.3).ceil() as usize + 16).clamp(k, g.n);
        let mut evals = 0;
        let mut found = Vec::new();
        loop {
            let wide = VSearchParams {
                ef: params.ef.max(take),
                rerank: if params.rerank > 0 {
                    params.rerank.max(take)
                } else {
                    0
                },
                reservoir: if params.reservoir > 0 {
                    params.reservoir.max(take)
                } else {
                    0
                },
                ..params
            };
            evals += self.search_validated(g, query, take, wide, &mut found)?;
            out.clear();
            out.extend(
                found
                    .iter()
                    .copied()
                    .filter(|&id| allow.contains(id))
                    .take(k),
            );
            if out.len() >= k || take >= g.n {
                return Ok(evals);
            }
            take = take.saturating_mul(2).min(g.n);
            if take.saturating_mul(8) >= count {
                return Ok(evals + self.scan_allowed(g, query, k, params, count, allow, out));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_allowed(
        &mut self,
        g: &VGraph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        count: usize,
        allow: &Allow<'_>,
        out: &mut Vec<u32>,
    ) -> usize {
        self.qf[..g.dim].copy_from_slice(query);
        let sq = g.encode_query(
            &mut self.qf,
            &mut self.qc,
            &mut self.qtmp,
            &mut self.qrot,
            &mut self.qs,
        );
        let w = if sq > 0.0 { 1.0 / sq } else { 1.0 };
        self.qsum = if cfg!(target_arch = "x86_64") {
            query_sum(&self.qc)
        } else {
            0
        };
        let mut vertices: Vec<u32> = Vec::with_capacity(count);
        if g.ids.is_empty() {
            allow.for_each(g.n, |id| vertices.push(id));
        } else {
            let inverse = g.vertex_of();
            allow.for_each(g.n, |id| vertices.push(inverse[id as usize]));
        }
        let mut keys = std::mem::take(&mut self.keys);
        keys.clear();
        keys.reserve(vertices.len());
        for (at, &v) in vertices.iter().enumerate() {
            if let Some(&ahead) = vertices.get(at + AHEAD) {
                prefetch_bytes(g.code(ahead));
                g.prefetch_scale(ahead);
            }
            keys.push(key(g.est(&self.qc, self.qsum, v, w), v));
        }
        let take = k
            .saturating_mul(4)
            .max(params.rerank)
            .max(params.ef)
            .clamp(k, keys.len());
        if keys.len() > take {
            keys.select_nth_unstable(take - 1);
            keys.truncate(take);
        }
        self.results.clear();
        self.results.reserve(keys.len());
        for &c in &keys {
            let v = key_id(c);
            self.results.push((g.fine(&self.qf, v), g.original(v)));
        }
        let evals = vertices.len() + keys.len();
        self.keys = keys;
        self.results
            .sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out.clear();
        out.extend(self.results.iter().take(k).map(|r| r.1));
        evals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::qgraph::{BuildParams, Graph};

    fn rows(n: usize, dim: usize) -> Vec<f32> {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        (0..n * dim)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 40) as f32 / (1u64 << 24) as f32 - 0.5
            })
            .collect()
    }

    fn graph(n: usize, dim: usize) -> (VGraph, Vec<f32>) {
        let data = rows(n, dim);
        let built = Graph::build_owned(data.clone(), dim, &BuildParams::default()).unwrap();
        (VGraph::from_graph(&built, true).unwrap(), data)
    }

    fn exact(data: &[f32], dim: usize, q: &[f32], k: usize, allow: &Allow<'_>) -> Vec<u32> {
        let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
        let mut scored: Vec<(f32, u32)> = data
            .chunks_exact(dim)
            .enumerate()
            .filter(|(i, _)| allow.contains(*i as u32))
            .map(|(i, r)| {
                let dot: f32 = r.iter().zip(q).map(|(a, b)| a * b).sum();
                (-dot / norm(r), i as u32)
            })
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(k).map(|s| s.1).collect()
    }

    /// Every returned id is allowed, a selective set is answered as the exact filtered top-k
    /// (up to the codes' ties), and a permissive one reaches the same recall as unfiltered.
    #[test]
    fn results_are_allowed_and_match_the_exact_filtered_neighbours() {
        let (n, dim, k) = (3000, 32, 10);
        let (g, data) = graph(n, dim);
        let queries = rows(20, dim);
        let params = VSearchParams {
            ef: 64,
            rerank: 64,
            ..VSearchParams::default()
        };
        let mut mask = vec![0u64; n.div_ceil(64)];
        for i in (0..n).filter(|i| i % 7 == 0) {
            mask[i / 64] |= 1 << (i % 64);
        }
        let scan = (SCAN_ROWS, SCAN_FRACTION);
        let graph_path = (0, 1_000_000);
        let range = |start, end| Allow::Range { start, end };
        let cases: [(Allow<'_>, f64, (usize, usize)); 6] = [
            (range(2900, 3000), 0.95, scan),
            (range(0, 2970), 0.9, scan),
            (range(0, 2970), 0.9, graph_path),
            (Allow::Mask(&mask), 0.9, scan),
            (Allow::Mask(&mask), 0.8, graph_path),
            (range(5000, 6000), 1.0, scan),
        ];
        let mut searcher = g.searcher();
        let mut out = Vec::new();
        for (allow, min_recall, scan) in cases {
            let allowed = allow.count(n);
            let mut hit = 0usize;
            let mut want = 0usize;
            for q in queries.chunks_exact(dim) {
                searcher
                    .scratch
                    .search_allowed(&g, q, k, params, &allow, scan, &mut out)
                    .unwrap();
                assert!(out.len() == k.min(allowed));
                assert!(out.iter().all(|&id| allow.contains(id)));
                let truth = exact(&data, dim, q, k, &allow);
                want += truth.len();
                hit += out.iter().filter(|id| truth.contains(id)).count();
            }
            let recall = if want == 0 {
                1.0
            } else {
                hit as f64 / want as f64
            };
            assert!(recall >= min_recall, "recall {recall} below {min_recall}");
        }
    }

    #[test]
    fn counts_stop_at_the_index_size() {
        let words = [u64::MAX; 3];
        assert_eq!(Allow::Mask(&words).count(130), 130);
        assert_eq!(Allow::Mask(&words).count(128), 128);
        assert_eq!(Allow::Mask(&words).count(10), 10);
        assert_eq!(Allow::Range { start: 5, end: 99 }.count(50), 45);
        assert_eq!(Allow::Range { start: 60, end: 99 }.count(50), 0);
    }
}
