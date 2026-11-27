// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[derive(Clone, Copy)]
pub(super) struct Ranked {
    pub(super) score: f32,
    pub(super) row: u32,
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Ranked {}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked {
    /// Greater is worse: a lower score, or the same score at a higher row.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then(self.row.cmp(&other.row))
    }
}

/// Bounded best-`k` set; the heap top is the worst kept candidate.
pub(super) struct TopK {
    pub(super) heap: BinaryHeap<Ranked>,
    pub(super) k: usize,
}

impl TopK {
    pub(super) fn new(k: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(k + 1),
            k,
        }
    }

    pub(super) fn offer(&mut self, row: u32, score: f32) {
        let candidate = Ranked { score, row };
        if self.k == 0 {
            return;
        }
        if self.heap.len() < self.k {
            self.heap.push(candidate);
        } else if let Some(mut worst) = self.heap.peek_mut() {
            if candidate < *worst {
                *worst = candidate;
            }
        }
    }

    pub(super) fn into_sorted(self) -> Vec<(u32, f32)> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|ranked| (ranked.row, ranked.score))
            .collect()
    }
}

/// Fewer rows are scored exactly outright; screening them saves nothing.
pub(super) const SCREEN_MIN_ROWS: usize = 256;

/// Dot product with eight independent accumulators so the loop vectorises.
#[inline]
pub(super) fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut lanes = [0.0f32; 8];
    let (wide_a, tail_a) = a.as_chunks::<8>();
    let (wide_b, tail_b) = b.as_chunks::<8>();
    for (x, y) in wide_a.iter().zip(wide_b) {
        for ((lane, x), y) in lanes.iter_mut().zip(x).zip(y) {
            *lane += x * y;
        }
    }
    let mut sum = lanes.iter().sum::<f32>();
    for (x, y) in tail_a.iter().zip(tail_b) {
        sum += x * y;
    }
    sum
}

/// Exact cosine over the snapshot rows, one query per thread. Each row is the store's own
/// embedding allocation (one pointer per row, shared with the chunk).
pub(crate) struct ExactScan {
    pub(super) dim: usize,
    pub(super) rows: Arc<[Embedding]>,
}

impl ExactScan {
    pub(super) fn row(&self, row: u32) -> &[f32] {
        &self.rows[row as usize]
    }

    pub(super) fn search_canonical(
        &self,
        query: &[f64],
        k: usize,
        include: impl Fn(usize) -> bool,
    ) -> Vec<(u32, f64)> {
        #[derive(Clone, Copy)]
        struct Candidate {
            row: u32,
            score: f64,
        }
        impl PartialEq for Candidate {
            fn eq(&self, other: &Self) -> bool {
                self.cmp(other) == Ordering::Equal
            }
        }
        impl Eq for Candidate {}
        impl PartialOrd for Candidate {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for Candidate {
            fn cmp(&self, other: &Self) -> Ordering {
                other
                    .score
                    .total_cmp(&self.score)
                    .then(self.row.cmp(&other.row))
            }
        }
        #[cfg(test)]
        tests::before_canonical_scoring();
        let k = k.min(self.rows.len());
        let mut best = BinaryHeap::<Candidate>::with_capacity(k);
        let mut offer = |row: usize, score: f64| {
            let candidate = Candidate {
                row: row as u32,
                score,
            };
            if best.len() < k {
                best.push(candidate);
            } else if let Some(mut worst) = best.peek_mut() {
                if candidate < *worst {
                    *worst = candidate;
                }
            }
        };
        if k > 0 && k < self.rows.len() && self.rows.len() >= SCREEN_MIN_ROWS {
            for row in self.screened_candidates(query, k, &include) {
                offer(row, self.rows[row].cosine(query));
            }
        } else {
            for (row, values) in self.rows.iter().enumerate() {
                if include(row) {
                    offer(row, values.cosine(query));
                }
            }
        }
        best.into_sorted_vec()
            .into_iter()
            .map(|hit| (hit.row, hit.score))
            .collect()
    }

    /// The included rows, in row order, that can rank among the `k` best canonical scores.
    ///
    /// Every row is screened with the `f32` kernel whose error against [`Embedding::cosine`] is bounded
    /// ([`ScreenBound`]). At least `k` included rows score at least their lower bound, so a row whose upper bound is
    /// below the `k`-th largest lower bound is outranked by `k` rows and cannot be kept. Ties are not pruned, so the
    /// survivors, scored exactly and ranked with the unchanged order, give the unscreened result bit for bit.
    fn screened_candidates(
        &self,
        query: &[f64],
        k: usize,
        include: &impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        const BLOCK: usize = 1024;
        let query32: Vec<f32> = query.iter().map(|&q| q as f32).collect();
        let bound = ScreenBound::new(query);
        let mut spans: Vec<(f64, f64)> = Vec::with_capacity(self.rows.len());
        let mut refs: Vec<&Embedding> = Vec::with_capacity(BLOCK);
        let mut screened = Vec::with_capacity(BLOCK);
        for block in self.rows.chunks(BLOCK) {
            refs.clear();
            refs.extend(block);
            Embedding::screen(&refs, &query32, &mut screened);
            spans.extend(screened.iter().map(|&(dot, norm)| {
                let (dot, slack) = (f64::from(dot), bound.of(norm));
                if dot.is_finite() && slack.is_finite() {
                    ((dot - slack).max(-1.0), (dot + slack).min(1.0))
                } else {
                    (f64::NEG_INFINITY, f64::INFINITY)
                }
            }));
        }
        let included: Vec<bool> = (0..spans.len()).map(include).collect();
        let mut lower: Vec<f64> = spans
            .iter()
            .zip(&included)
            .filter(|(_, &keep)| keep)
            .map(|(span, _)| span.0)
            .collect();
        let threshold = if lower.len() > k {
            *lower.select_nth_unstable_by(k - 1, |a, b| b.total_cmp(a)).1
        } else {
            f64::NEG_INFINITY
        };
        (0..spans.len())
            .filter(|&row| included[row] && spans[row].1 >= threshold)
            .collect()
    }
}

impl VectorIndex for ExactScan {
    fn kind(&self) -> VectorIndexKind {
        VectorIndexKind::Exact
    }

    fn rows(&self) -> usize {
        self.rows.len()
    }

    fn search(&self, query: &[f32], k: usize, _ef: usize) -> Result<Vec<(u32, f32)>> {
        ensure!(
            query.len() == self.dim,
            "query has {} dimensions, the index {}",
            query.len(),
            self.dim
        );
        let mut best = TopK::new(k.min(self.rows()));
        for (row, values) in self.rows.iter().enumerate() {
            best.offer(row as u32, dot(query, values));
        }
        Ok(best.into_sorted())
    }
}

/// The vertex graph over the snapshot rows; fetched ids are re-scored exactly.
pub(crate) struct GraphIndex {
    pub(super) graph: VGraph,
    pub(super) exact: ExactScan,
    pub(super) candidate_dimensions: Option<usize>,
    pub(super) build_degree: usize,
    pub(super) build_ef: usize,
    pub(super) high_recall: bool,
}

impl VectorIndex for GraphIndex {
    fn kind(&self) -> VectorIndexKind {
        VectorIndexKind::Graph
    }

    fn rows(&self) -> usize {
        self.exact.rows()
    }

    fn search(&self, query: &[f32], k: usize, ef: usize) -> Result<Vec<(u32, f32)>> {
        self.search_candidates(query, k, ef)?
            .context("query prefix has zero norm; use the full exact route")
    }
}

impl GraphIndex {
    pub(super) fn search_candidates(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
    ) -> Result<Option<Vec<(u32, f32)>>> {
        let Some(ids) = self.search_candidate_ids(query, k, ef)? else {
            return Ok(None);
        };
        let mut hits: Vec<_> = ids
            .into_iter()
            .map(|id| (id, dot(query, self.exact.row(id))))
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(Some(hits))
    }

    pub(super) fn search_candidate_ids(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
    ) -> Result<Option<Vec<u32>>> {
        ensure!(
            query.len() == self.exact.dim,
            "query has {} dimensions, the index {}",
            query.len(),
            self.exact.dim
        );
        let k = k.min(self.rows());
        if k == 0 {
            return Ok(Some(Vec::new()));
        }
        let ef = ef.clamp(k, MAX_EF.max(k));
        let params = VSearchParams {
            ef,
            rerank: ef,
            ..VSearchParams::default()
        };
        let mut ids = Vec::with_capacity(k);
        let mut projected = Vec::new();
        let candidate_query = if let Some(dimensions) = self.candidate_dimensions {
            if !append_normalized_prefix(query, dimensions, &mut projected)? {
                return Ok(None);
            }
            projected.as_slice()
        } else {
            query
        };
        #[cfg(not(target_arch = "wasm32"))]
        if self.high_recall {
            self.graph
                .search_cached(candidate_query, k, params, &mut ids)
        } else {
            self.graph
                .search_two_hop_cached(candidate_query, k, params, &mut ids)
        }
        .map_err(|e| anyhow::anyhow!("graph search: {e}"))?;
        #[cfg(target_arch = "wasm32")]
        if self.high_recall {
            self.graph
                .searcher()
                .search(candidate_query, k, params, &mut ids)
        } else {
            self.graph
                .searcher()
                .search_two_hop(candidate_query, k, params, &mut ids)
        }
        .map_err(|e| anyhow::anyhow!("graph search: {e}"))?;
        validate_graph_candidates(&mut ids, self.rows())?;
        Ok(Some(ids))
    }
}

pub(super) fn validate_graph_candidates(ids: &mut [u32], rows: usize) -> Result<()> {
    ensure!(
        ids.iter().all(|&id| (id as usize) < rows),
        "graph returned a candidate outside the vector snapshot"
    );
    ids.sort_unstable();
    ensure!(
        ids.windows(2).all(|pair| pair[0] != pair[1]),
        "graph returned duplicate candidates"
    );
    Ok(())
}

pub(super) fn append_normalized_prefix(
    values: &[f32],
    dimensions: usize,
    out: &mut Vec<f32>,
) -> Result<bool> {
    ensure!(
        (1..=values.len()).contains(&dimensions),
        "invalid vector candidate dimensions"
    );
    let prefix = &values[..dimensions];
    ensure!(
        prefix.iter().all(|value| value.is_finite()),
        "vector prefix must be finite"
    );
    let norm = prefix
        .iter()
        .map(|&value| f64::from(value).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return Ok(false);
    }
    out.try_reserve(dimensions)
        .context("cannot allocate normalized vector prefix")?;
    out.extend(prefix.iter().map(|&value| (f64::from(value) / norm) as f32));
    Ok(true)
}
