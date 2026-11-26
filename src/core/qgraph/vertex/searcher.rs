// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Per-thread search state for one [`VGraph`].
pub struct VSearcher<'a> {
    pub(super) index: &'a VGraph,
    pub(super) scratch: SearchScratch,
}

pub(in super::super) struct SearchScratch {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) dim: usize,
    pub(super) visited: Marks,
    pub(super) expanded_hops: Option<Marks>,
    pub(super) qf: Vec<f32>,
    /// Query code in the traversal kernel's layout, and its coordinate sum.
    pub(super) qc: Vec<i8>,
    pub(super) qsum: i32,
    pub(super) qtmp: Vec<i8>,
    /// Rotated, scaled query and its bit planes for the screen.
    pub(super) qrot: Vec<f32>,
    pub(super) qs: QueryCode,
    /// Query rotated for the neighbour blocks, its lookup table, and one block's sums.
    pub(super) bqrot: Vec<f32>,
    pub(super) bq: BlockQuery,
    pub(super) scan: [u32; BLOCK],
    pub(super) fresh: Vec<u32>,
    /// Fresh neighbours that passed the screen.
    pub(super) survivors: Vec<u32>,
    /// Neighbours the screen skipped in the last search.
    pub(super) screened: usize,
    /// The `ef` best candidates seen, with their expansion state.
    pub(super) pool: Pool,
    /// With blocks: the best `max(k, rerank)` expanded vertices by their own-code estimate,
    /// farthest on top. Block estimates only order the exploration; results come from here.
    pub(super) expanded: BinaryHeap<u64>,
    pub(super) reservoir: Reservoir,
    pub(super) keys: Vec<u64>,
    pub(super) results: Vec<(f32, u32)>,
    /// Query lists: the normalized query and the probed `(score, list)` pairs.
    pub(super) probe_query: Vec<f32>,
    pub(super) probed: Vec<(f32, u32)>,
    pub(super) knobs: SearchKnobs,
}

impl VSearcher<'_> {
    /// Neighbours the screen skipped (no 8-bit estimate) in the last search.
    pub fn last_screened(&self) -> usize {
        self.scratch.screened
    }

    /// The knobs this searcher runs with.
    pub fn knobs(&self) -> SearchKnobs {
        self.scratch.knobs
    }

    /// Searches this index with the searcher's reusable buffers.
    /// Returns `min(k, n)` distinct neighbors, restarting from an unseen vertex only when
    /// traversal would otherwise return too few candidates.
    #[inline]
    pub fn search(
        &mut self,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        self.scratch.search(self.index, query, k, params, out)
    }

    pub(crate) fn search_two_hop(
        &mut self,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        self.scratch
            .search_two_hop(self.index, query, k, params, out)
    }
}

impl SearchScratch {
    pub(super) fn search_two_hop(
        &mut self,
        index: &VGraph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        Self::validate_query(index, query)?;
        self.search_inner::<false, { cfg!(feature = "experimental-batch-pool") }, true, true>(
            index,
            query,
            k,
            params,
            out,
            &mut QueryProfile::default(),
        )
    }

    /// Row estimates of the admitted neighbours in `fresh`, inserted into the pool in list
    /// order (four rows at a time with `batch_est`; the same values either way); returns the
    /// number of estimates. `stale` resets on an insert among the `k` best when `patience`
    /// is on.
    #[inline]
    pub(super) fn score_fresh<const BATCH_POOL: bool, const QUERY_X4: bool>(
        &mut self,
        g: &VGraph,
        k: usize,
        patience: usize,
        stale: &mut usize,
        w: f32,
    ) -> usize {
        #[cfg(feature = "experimental-batch-pool")]
        if BATCH_POOL {
            return self.score_fresh_batch::<QUERY_X4>(g, k, patience, stale, w);
        }
        let qc = &self.qc[..];
        let n = self.fresh.len();
        let mut i = 0;
        if QUERY_X4 || self.knobs.batch_est {
            while i + 4 <= n {
                let ids = [
                    self.fresh[i],
                    self.fresh[i + 1],
                    self.fresh[i + 2],
                    self.fresh[i + 3],
                ];
                for (e, u) in g
                    .est4::<QUERY_X4>(qc, self.qsum, ids, w)
                    .into_iter()
                    .zip(ids)
                {
                    let kk = key(e, u);
                    self.reservoir.offer(kk);
                    if let Some((pos, _)) = self.pool.insert(kk) {
                        if patience > 0 && pos < k {
                            *stale = 0;
                        }
                    }
                }
                i += 4;
            }
        }
        for &u in &self.fresh[i..] {
            let kk = key(g.est(qc, self.qsum, u, w), u);
            self.reservoir.offer(kk);
            if let Some((pos, _)) = self.pool.insert(kk) {
                if patience > 0 && pos < k {
                    *stale = 0;
                }
            }
        }
        n
    }

    #[cfg(feature = "experimental-batch-pool")]
    pub(super) fn score_fresh_batch<const QUERY_X4: bool>(
        &mut self,
        g: &VGraph,
        k: usize,
        patience: usize,
        stale: &mut usize,
        w: f32,
    ) -> usize {
        let qc = &self.qc[..];
        let n = self.fresh.len();
        let (reservoir, pending) = (&mut self.reservoir, &mut self.keys);
        pending.clear();
        let mut offer = |estimate, id| {
            let key = key(estimate, id);
            reservoir.offer(key);
            pending.push(key);
        };
        let mut i = 0;
        if QUERY_X4 || self.knobs.batch_est {
            while i + 4 <= n {
                let ids = [
                    self.fresh[i],
                    self.fresh[i + 1],
                    self.fresh[i + 2],
                    self.fresh[i + 3],
                ];
                for (estimate, id) in g
                    .est4::<QUERY_X4>(qc, self.qsum, ids, w)
                    .into_iter()
                    .zip(ids)
                {
                    offer(estimate, id);
                }
                i += 4;
            }
        }
        for &id in &self.fresh[i..] {
            offer(g.est(qc, self.qsum, id, w), id);
        }
        if self.pool.insert_batch(pending, k) && patience > 0 {
            *stale = 0;
        }
        n
    }

    /// Writes the ids of the (approximate) `k` nearest vectors by the index's metric (cosine
    /// or Euclidean) into `out`, nearest first, and returns the number of distance estimates
    /// (descent, traversal and re-rank). A query of the wrong dimension, with a non-finite
    /// value, or all zero on an angular index (cosine is undefined) is an error; the pool and
    /// heaps are bounded by the vertex count whatever `ef` is.
    pub(super) fn search(
        &mut self,
        g: &VGraph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        Self::validate_query(g, query)?;
        self.search_validated(g, query, k, params, out)
    }

    pub(super) fn validate_query(g: &VGraph, query: &[f32]) -> Result<(), WarmError> {
        if query.len() != g.dim {
            return Err(invalid(format!(
                "query has {} dimensions, the index has {}",
                query.len(),
                g.dim
            )));
        }
        if let Some(at) = query.iter().position(|v| !v.is_finite()) {
            return Err(invalid(format!(
                "non-finite query value at coordinate {at}"
            )));
        }
        if g.metric == VMetric::Angular && query.iter().all(|&v| v == 0.0) {
            return Err(invalid("zero query: cosine similarity is undefined".into()));
        }
        Ok(())
    }

    // The caller has checked dimension, finite coordinates, and nonzero angular norm.
    pub(super) fn search_validated(
        &mut self,
        g: &VGraph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        self.search_inner::<false, { cfg!(feature = "experimental-batch-pool") }, true, false>(
            g,
            query,
            k,
            params,
            out,
            &mut QueryProfile::default(),
        )
    }

    pub(super) fn search_inner<
        const PROFILE: bool,
        const BATCH_POOL: bool,
        const QUERY_X4: bool,
        const TWO_HOP: bool,
    >(
        &mut self,
        g: &VGraph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
        profile: &mut QueryProfile,
    ) -> Result<usize, WarmError> {
        if TWO_HOP && (g.has_blocks() || params.neighbour_cap != 0) {
            return Err(invalid(
                "two-hop search requires uncapped row-code traversal".into(),
            ));
        }
        if params.reservoir > 0 && (g.has_blocks() || (g.has_screen() && params.screen)) {
            return Err(invalid(
                "independent retention requires unscreened row-code search".into(),
            ));
        }
        let started = PROFILE.then(Instant::now);
        let ef = params.ef.max(k).max(1);
        self.reservoir.reset(if params.reservoir == 0 {
            0
        } else {
            params.reservoir.max(k).min(g.n)
        });
        // Only the order matters, so the query is used as given (no centring needed: the
        // `<q, mean>` term is common to all candidates) and its scale is dropped.
        self.qf[..g.dim].copy_from_slice(query);
        let sq = g.encode_query(
            &mut self.qf,
            &mut self.qc,
            &mut self.qtmp,
            &mut self.qrot,
            &mut self.qs,
        );
        // Weight of the Euclidean norm term in the estimate's units (unused for angular); a
        // zero query has every inner product zero, so the norms alone order it.
        let w = if sq > 0.0 { 1.0 / sq } else { 1.0 };
        // Only the x86 VNNI kernels read the sum.
        self.qsum = if cfg!(target_arch = "x86_64") {
            query_sum(&self.qc)
        } else {
            0
        };
        if let Some(started) = started {
            profile.query_encoding_ns = started.elapsed().as_nanos() as u64;
        }
        if params.probes > 0
            && params.probe_only
            && params.reservoir == 0
            && !TWO_HOP
            && !g.has_blocks()
            && g.lists.as_ref().is_some_and(QueryLists::is_packed)
        {
            if let Some(scanned) = self.scan_packed(g, query, k, ef, params.probes, true) {
                let take = params.rerank.max(k).min(g.n);
                let rerank_started = PROFILE.then(Instant::now);
                if PROFILE {
                    profile.beam_retained = self.keys.len();
                    profile.traversal_evaluations = scanned;
                }
                return self.rerank_keys::<PROFILE>(
                    g,
                    k,
                    take,
                    params,
                    sq,
                    scanned,
                    out,
                    profile,
                    rerank_started,
                    started,
                );
            }
        }
        let qc = &self.qc[..];
        let blocks = g.has_blocks();
        if blocks {
            g.encode_block_query(&self.qf, sq, &mut self.bqrot, &mut self.bq);
        }
        let knobs = self.knobs;
        let block_screen = blocks && knobs.block_screen;
        let screen = g.has_screen() && params.screen && !blocks;
        let margin = params.screen_sigmas * g.screen_sigma;
        self.screened = 0;

        self.visited.next();
        if TWO_HOP {
            let expanded = self
                .expanded_hops
                .get_or_insert_with(|| Marks::new(g.n, false));
            if expanded.n != g.n {
                *expanded = Marks::new(g.n, false);
            }
            expanded.next();
        }
        let entry_started = PROFILE.then(Instant::now);
        let (entry, mut evals) = g.descend(qc, self.qsum, w, &mut self.reservoir);
        if let Some(started) = entry_started {
            profile.entry_ns = started.elapsed().as_nanos() as u64;
            profile.entry_evaluations = evals;
        }
        self.visited.insert(entry);
        let fuse = blocks && params.fuse && !block_screen;
        if fuse {
            self.visited.trace(entry);
        }
        // Query terms of the error model: `|q - x_v|^2 = |q|^2 + 1 - 2 (<q, x_v - mean> + <q, mean>)`
        // for unit index rows.
        let (qn2, qm) = if blocks {
            let q = &self.qf[..g.dim];
            (
                q.iter().map(|a| a * a).sum::<f32>(),
                q.iter().zip(&g.mean).map(|(a, m)| a * m).sum::<f32>(),
            )
        } else {
            (0.0, 0.0)
        };
        self.pool.reset(ef);
        self.expanded.clear();
        let take = params.rerank.max(k).min(g.n);
        let k0 = key(g.est(qc, self.qsum, entry, w), entry);
        self.reservoir.offer(k0);
        self.pool.insert(k0);
        if fuse {
            self.visited.trace(entry).1.key = k0;
        }
        evals += 1;
        let probe_only = params.probes > 0 && params.probe_only && g.lists.is_some();
        if let (Some(lists), true) = (g.lists.as_ref(), params.probes > 0) {
            if blocks {
                return Err(invalid(
                    "query lists require row-code search, not neighbour blocks".into(),
                ));
            }
            if lists.is_packed() && params.reservoir == 0 {
                // The scan marks every row it met; the best `ef` of them seed the pool.
                if let Some(scanned) = self.scan_packed(g, query, 0, ef, params.probes, false) {
                    evals += scanned;
                    for &seed in &self.keys {
                        self.pool.insert(seed);
                    }
                }
            } else {
                lists.probe(
                    query,
                    params.probes,
                    &mut self.probe_query,
                    &mut self.probed,
                );
                for &(_, c) in &self.probed {
                    let ids = lists.list(c);
                    for (i, &v) in ids.iter().enumerate() {
                        if let Some(&u) = ids.get(i + 8) {
                            prefetch_bytes(g.code(u));
                            g.prefetch_scale(u);
                        }
                        if !self.visited.insert(v) {
                            continue;
                        }
                        let candidate = key(g.est(qc, self.qsum, v, w), v);
                        self.reservoir.offer(candidate);
                        self.pool.insert(candidate);
                        evals += 1;
                    }
                }
            }
        }
        // Best-first expansion of the nearest unexpanded candidate among the `ef` best seen;
        // the pool holds exactly those, so the search ends when none is left. With
        // `patience`, it also ends after that many expansions without an insert among the
        // `k` best.
        let patience = params.patience;
        let frontier = if knobs.frontier.is_finite() {
            knobs.frontier.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let mut stale = 0usize;
        let target = k.min(g.n);
        let enough = |search: &Self| {
            let count = if params.reservoir > 0 {
                search.reservoir.keys.len()
            } else if blocks && !block_screen {
                search.expanded.len()
            } else {
                search.pool.keys.len()
            };
            count >= target
        };
        let mut seed_cursor = 0usize;
        loop {
            if probe_only && enough(self) {
                break;
            }
            let c = match self.pool.next() {
                Some(c) => c,
                None => {
                    if enough(self) {
                        break;
                    }
                    // Exhausting a full pool would have retained at least `target` distinct
                    // results. A short result therefore leaves room for an unseen seed.
                    let seed = (seed_cursor..g.n)
                        .find(|&v| {
                            if fuse {
                                self.visited.traces[v].epoch != self.visited.epoch
                            } else {
                                !self.visited.seen(v as u32)
                            }
                        })
                        .ok_or_else(|| {
                            invalid(
                                "search exhausted vertices before retaining enough candidates"
                                    .into(),
                            )
                        })?;
                    seed_cursor = seed + 1;
                    let seed = seed as u32;
                    self.visited.insert(seed);
                    let candidate = key(g.est(&self.qc, self.qsum, seed, w), seed);
                    self.reservoir.offer(candidate);
                    let (_, evicted) = self
                        .pool
                        .insert(candidate)
                        .ok_or_else(|| invalid("search could not admit a recovery seed".into()))?;
                    if fuse {
                        self.visited.trace(seed).1.key = candidate;
                        if let Some(evicted) = evicted {
                            self.visited.trace(key_id(evicted)).1.key = u64::MAX;
                        }
                    }
                    evals += 1;
                    if PROFILE {
                        profile.traversal_evaluations += 1;
                    }
                    stale = 0;
                    continue;
                }
            };
            if patience > 0 && stale >= patience && enough(self) {
                break;
            }
            if frontier < 1.0 && k > 0 && self.pool.is_full() {
                // Frontier rule: the nearest unexpanded candidate already lies past the point
                // `frontier` of the way from the `k`-th best estimate to the `ef`-th. At 1.0
                // it never fires (every pool key is at most the `ef`-th).
                let len = self.pool.keys.len();
                let kth = key_dist(self.pool.keys[k.min(len) - 1]);
                let worst = key_dist(self.pool.keys[len - 1]);
                if key_dist(c) > kth + frontier * (worst - kth) && enough(self) {
                    break;
                }
            }
            if fuse {
                // A re-admitted vertex may sit in the pool twice; expand it once.
                let (_, t) = self.visited.trace(key_id(c));
                if t.expanded {
                    continue;
                }
                t.expanded = true;
            }
            stale += 1;
            if PROFILE {
                profile.expansions += 1;
            }
            let adjacency_started = PROFILE.then(Instant::now);
            if let Some(next) = self.pool.peek_next() {
                g.prefetch_neighbours(key_id(next) as usize);
                if blocks {
                    g.prefetch_blocks(key_id(next) as usize);
                    if !block_screen {
                        prefetch_bytes(g.code(key_id(next)));
                    }
                }
            }
            if knobs.prefetch >= 2 {
                if let Some(second) = self.pool.peek_unexpanded(1) {
                    g.prefetch_neighbours(key_id(second) as usize);
                    if blocks {
                        g.prefetch_blocks(key_id(second) as usize);
                    }
                }
            }
            if block_screen {
                // The neighbour block screens and the pool holds row estimates, so the popped
                // key carries the row estimate of `v` (`key` round-trips the f32 bits). A fresh
                // neighbour whose block estimate, improved by `screen_sigmas` deviations of
                // the calibrated edge error, could not enter the full pool is skipped (it
                // stays visited); the survivors' rows are prefetched together, then estimated.
                let v = key_id(c) as usize;
                let ip_v = -key_dist(c) * sq;
                let d2_v = (qn2 + 1.0 - 2.0 * (ip_v + qm)).max(1e-6);
                let (delta, min_sum, inv_sq) = (self.bq.delta, self.bq.min_sum, self.bq.inv_sq);
                let worst = self.pool.worst().map(key_dist);
                let sigmas = params.screen_sigmas;
                self.fresh.clear();
                let list = &mut self.fresh;
                g.for_each_neighbour(v, |u| list.push(u));
                self.survivors.clear();
                for (b, ids) in self
                    .fresh
                    .chunks(BLOCK)
                    .take(g.blocks_per_vertex)
                    .enumerate()
                {
                    let (codes, scales, offsets) = g.block(v, b);
                    scan32(codes, &self.bq.lut, g.block_m, &mut self.scan);
                    let (scales, offsets) = (scales.as_chunks::<4>().0, offsets.as_chunks::<4>().0);
                    for (((&u, &raw), s), o) in ids.iter().zip(&self.scan).zip(scales).zip(offsets)
                    {
                        if !self.visited.insert(u) {
                            continue;
                        }
                        let s = f32::from_le_bytes(*s);
                        let ip = ip_v + s * (delta * raw as f32 + min_sum) + f32::from_le_bytes(*o);
                        if let Some(w) = worst {
                            let sigma = (g.block_kappa * s * s * d2_v).sqrt();
                            if -(ip + sigmas * sigma) * inv_sq >= w {
                                self.screened += 1;
                                continue;
                            }
                        }
                        prefetch_bytes(g.code(u));
                        g.prefetch_scale(u);
                        self.survivors.push(u);
                    }
                }
                std::mem::swap(&mut self.fresh, &mut self.survivors);
                evals += self.score_fresh::<BATCH_POOL, QUERY_X4>(g, k, patience, &mut stale, w);
                continue;
            }
            if blocks {
                // The expanded vertex is scored with its own code (the per-row estimate the
                // plain search uses everywhere) into the result pool. One table scan per
                // block then estimates all its neighbours, ordering the exploration; list
                // padding is skipped, and a vertex met again is re-admitted only when its
                // fused estimate improves on the entry it holds.
                let v = key_id(c) as usize;
                let est_v = g.est(&self.qc, self.qsum, v as u32, w);
                let ek = key(est_v, v as u32);
                evals += 1;
                // `<q, x_v - mean>` in float units: the estimate dropped the query scale.
                let ip_v = -est_v * sq;
                let d2_v = (qn2 + 1.0 - 2.0 * (ip_v + qm)).max(1e-6);
                if self.expanded.len() < take {
                    self.expanded.push(ek);
                } else if let Some(mut worst) = self.expanded.peek_mut() {
                    if ek < *worst {
                        *worst = ek;
                    }
                }
                let (delta, min_sum, inv_sq) = (self.bq.delta, self.bq.min_sum, self.bq.inv_sq);
                self.fresh.clear();
                let list = &mut self.fresh;
                g.for_each_neighbour(v, |u| list.push(u));
                for (b, ids) in self
                    .fresh
                    .chunks(BLOCK)
                    .take(g.blocks_per_vertex)
                    .enumerate()
                {
                    let (codes, scales, offsets) = g.block(v, b);
                    scan32(codes, &self.bq.lut, g.block_m, &mut self.scan);
                    let (scales, offsets) = (scales.as_chunks::<4>().0, offsets.as_chunks::<4>().0);
                    for (((&u, &raw), s), o) in ids.iter().zip(&self.scan).zip(scales).zip(offsets)
                    {
                        let s = f32::from_le_bytes(*s);
                        let ip = ip_v + s * (delta * raw as f32 + min_sum) + f32::from_le_bytes(*o);
                        if !fuse {
                            // Without fusion the block path keeps the plain path's byte marks
                            // (the per-vertex trace records would not stay in cache).
                            if !self.visited.insert(u) {
                                continue;
                            }
                            evals += 1;
                            if let Some((pos, _)) = self.pool.insert(key(-ip * inv_sq, u)) {
                                if patience > 0 && pos < k {
                                    stale = 0;
                                }
                            }
                            continue;
                        }
                        // Precision of this estimate under the calibrated error model.
                        let w = 1.0 / (g.block_kappa * s * s * d2_v + 1e-12);
                        let (fresh, t) = self.visited.trace(u);
                        let ip = if fresh {
                            t.w = w;
                            t.wsum = w * ip;
                            ip
                        } else {
                            if t.expanded {
                                continue;
                            }
                            t.w += w;
                            t.wsum += w * ip;
                            t.wsum / t.w
                        };
                        let kk = key(-ip * inv_sq, u);
                        let old = t.key;
                        evals += 1;
                        if !fresh && kk >= old {
                            continue;
                        }
                        if old != u64::MAX {
                            self.pool.remove(old);
                        }
                        // `t` borrows `visited` only, so it stays live across the pool update.
                        let Some((pos, evicted)) = self.pool.insert(kk) else {
                            t.key = u64::MAX;
                            continue;
                        };
                        t.key = kk;
                        if let Some(e) = evicted {
                            self.visited.trace(key_id(e)).1.key = u64::MAX;
                        }
                        if patience > 0 && pos < k {
                            stale = 0;
                        }
                    }
                }
                continue;
            }
            self.fresh.clear();
            let (visited, fresh, expanded_hops) =
                (&mut self.visited, &mut self.fresh, &mut self.expanded_hops);
            let neighbors_read = if TWO_HOP {
                let expanded = expanded_hops.as_mut().expect("two-hop marks initialized");
                let mut neighbors_read = 0;
                let admit = |u: u32, visited: &mut Marks, fresh: &mut Vec<u32>| {
                    if visited.insert(u) {
                        if screen {
                            g.prefetch_screen(u);
                        } else {
                            prefetch_bytes(g.code(u));
                            g.prefetch_scale(u);
                        }
                        fresh.push(u);
                    }
                };
                g.for_each_neighbour(key_id(c) as usize, |u| {
                    if PROFILE {
                        neighbors_read += 1;
                    }
                    admit(u, visited, fresh);
                    // Every neighbor stays visited for the rest of this query, including screened candidates.
                    if expanded.insert(u) {
                        g.for_each_neighbour(u as usize, |v| {
                            if PROFILE {
                                neighbors_read += 1;
                            }
                            admit(v, visited, fresh);
                        });
                    }
                });
                neighbors_read
            } else {
                #[cfg(feature = "experimental-neighbor-admission")]
                {
                    if screen {
                        g.admit_neighbours::<PROFILE, true>(
                            key_id(c) as usize,
                            params.neighbour_cap,
                            visited,
                            fresh,
                        )
                    } else {
                        g.admit_neighbours::<PROFILE, false>(
                            key_id(c) as usize,
                            params.neighbour_cap,
                            visited,
                            fresh,
                        )
                    }
                }
                #[cfg(not(feature = "experimental-neighbor-admission"))]
                {
                    let mut neighbors_read = 0;
                    let admit = |u: u32, visited: &mut Marks, fresh: &mut Vec<u32>| {
                        if visited.insert(u) {
                            if screen {
                                g.prefetch_screen(u);
                            } else {
                                prefetch_bytes(g.code(u));
                                g.prefetch_scale(u);
                            }
                            fresh.push(u);
                        }
                    };
                    if params.neighbour_cap == 0 {
                        g.for_each_neighbour(key_id(c) as usize, |u| {
                            if PROFILE {
                                neighbors_read += 1;
                            }
                            admit(u, visited, fresh);
                        });
                    } else {
                        let mut seen = 0usize;
                        g.for_each_neighbour(key_id(c) as usize, |u| {
                            seen += 1;
                            if seen <= params.neighbour_cap {
                                admit(u, visited, fresh);
                            }
                        });
                    }
                    neighbors_read
                }
            };
            if screen {
                // A neighbour whose optimistic 1-bit estimate cannot enter the full pool is
                // skipped (it stays visited); the rest get the 8-bit estimate.
                let worst = self.pool.worst().map(key_dist);
                self.survivors.clear();
                for &u in &self.fresh {
                    if let Some(w) = worst {
                        if g.screen_est(&self.qs, u) - margin >= w {
                            self.screened += 1;
                            continue;
                        }
                    }
                    prefetch_bytes(g.code(u));
                    g.prefetch_scale(u);
                    self.survivors.push(u);
                }
                std::mem::swap(&mut self.fresh, &mut self.survivors);
            }
            if let Some(started) = adjacency_started {
                profile.adjacency_visited_prefetch_ns += started.elapsed().as_nanos() as u64;
                profile.neighbors_read += neighbors_read;
            }
            let scoring_started = PROFILE.then(Instant::now);
            let count = self.score_fresh::<BATCH_POOL, QUERY_X4>(g, k, patience, &mut stale, w);
            evals += count;
            if let Some(started) = scoring_started {
                profile.scoring_pool_ns += started.elapsed().as_nanos() as u64;
                profile.traversal_evaluations += count;
            }
            if knobs.prefetch >= 3 {
                // Lookahead: what the next expansion's admission touches for its unvisited
                // neighbours, one expansion early (its list was prefetched above).
                if let Some(next) = self.pool.peek_next() {
                    let visited = &self.visited;
                    g.for_each_neighbour(key_id(next) as usize, |u| {
                        if !visited.seen(u) {
                            if screen {
                                g.prefetch_screen(u);
                            } else {
                                prefetch_bytes(g.code(u));
                                g.prefetch_scale(u);
                            }
                        }
                    });
                }
            }
        }

        let rerank_started = PROFILE.then(Instant::now);
        if PROFILE {
            profile.beam_retained = self.pool.keys.len();
            profile.traversal_evaluations += 1;
        }
        self.keys.clear();
        if params.reservoir > 0 {
            self.keys.extend_from_slice(&self.reservoir.keys);
        } else if blocks && !block_screen {
            self.keys.extend(self.expanded.drain());
            self.keys.sort_unstable();
        } else {
            self.keys.extend_from_slice(&self.pool.keys);
        }
        self.rerank_keys::<PROFILE>(
            g,
            k,
            take,
            params,
            sq,
            evals,
            out,
            profile,
            rerank_started,
            started,
        )
    }

    /// Probe-only search over packed query lists: estimates every row of the `probes` nearest
    /// lists from their contiguous code copies and keeps the `ef` nearest in `self.keys`
    /// (sorted), without the candidate pool or the graph (`fresh` clears the visited marks
    /// first; otherwise the rows already marked are skipped). Returns the rows estimated, or `None`
    /// when fewer than `k` distinct rows were found (the caller then searches the graph).
    #[inline(never)]
    fn scan_packed(
        &mut self,
        g: &VGraph,
        query: &[f32],
        k: usize,
        ef: usize,
        probes: usize,
        fresh: bool,
    ) -> Option<usize> {
        /// Rows ahead of the scan whose codes are prefetched.
        const LOOKAHEAD: usize = 16;
        let lists = g.lists.as_ref()?;
        lists.probe(query, probes, &mut self.probe_query, &mut self.probed);
        if fresh {
            self.visited.next();
        }
        self.keys.clear();
        let (qc, qsum, cb) = (&self.qc[..], self.qsum, g.cbytes);
        let cap = ef;
        let mut tau = u64::MAX;
        let mut scanned = 0usize;
        // Every row met is marked, so a row listed under several probed lists is estimated
        // once and its code is never read again. A key past `tau` (the `cap`-th best at the
        // last compaction) cannot enter the best `cap`.
        let mut offer = |keys: &mut Vec<u64>, id: u32, dot: i64, scale: f32| {
            let k = key(-(scale * dot as f32), id);
            if k < tau {
                keys.push(k);
                if keys.len() >= 2 * cap {
                    keys.select_nth_unstable(cap - 1);
                    keys.truncate(cap);
                    tau = keys[cap - 1];
                }
            }
        };
        for &(_, c) in &self.probed {
            let (ids, codes, scales) = lists.packed_list(c)?;
            let row = |i: usize| &codes[i * cb..(i + 1) * cb];
            let mut batch = [0usize; 4];
            let mut held = 0;
            for (i, &id) in ids.iter().enumerate() {
                if let Some(&ahead) = ids.get(i + LOOKAHEAD) {
                    if !self.visited.seen(ahead) {
                        prefetch_bytes(row(i + LOOKAHEAD));
                    }
                }
                if !self.visited.insert(id) {
                    continue;
                }
                batch[held] = i;
                held += 1;
                if held == 4 {
                    held = 0;
                    scanned += 4;
                    let rows = [row(batch[0]), row(batch[1]), row(batch[2]), row(batch[3])];
                    let dots = dot_i8_query_x4(qc, rows, qsum);
                    for (&at, dot) in batch.iter().zip(dots) {
                        offer(&mut self.keys, ids[at], dot, scales[at]);
                    }
                }
            }
            for &at in &batch[..held] {
                scanned += 1;
                let dot = dot_i8_query(qc, row(at), qsum);
                offer(&mut self.keys, ids[at], dot, scales[at]);
            }
        }
        self.keys.sort_unstable();
        self.keys.truncate(cap);
        (self.keys.len() >= k.min(g.n)).then_some(scanned)
    }

    /// Re-scores the best `take` of `self.keys` (sorted, nearest first) with the residual
    /// codes and writes the `k` nearest original ids to `out`; `evals` is the work so far.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn rerank_keys<const PROFILE: bool>(
        &mut self,
        g: &VGraph,
        k: usize,
        take: usize,
        params: VSearchParams,
        sq: f32,
        mut evals: usize,
        out: &mut Vec<u32>,
        profile: &mut QueryProfile,
        rerank_started: Option<Instant>,
        started: Option<Instant>,
    ) -> Result<usize, WarmError> {
        let knobs = self.knobs;
        if PROFILE {
            profile.retained = self.keys.len();
        }
        self.results.clear();
        let take = take.min(self.keys.len());
        if params.rerank > 0 && knobs.rerank_z > 0.0 && sq > 0.0 && k > 0 && take > k {
            evals += self.rerank_gated(g, k, take, sq, knobs.rerank_z);
            if PROFILE {
                profile.rerank_evaluations = self.results.len();
            }
        } else if params.rerank > 0 {
            if PROFILE {
                profile.rerank_evaluations = take;
            }
            let lead = (8192 / (g.stride + g.res_stride)).clamp(1, 8);
            for &c in &self.keys[..take.min(lead)] {
                g.prefetch_rerank_rows(key_id(c));
            }
            // Ties (duplicate vectors) break by the original id, independent of the layout.
            self.results.reserve(take);
            for (position, &c) in self.keys[..take].iter().enumerate() {
                if position + lead < take {
                    g.prefetch_rerank_rows(key_id(self.keys[position + lead]));
                }
                let id = key_id(c);
                self.results.push((g.fine(&self.qf, id), g.original(id)));
            }
            evals += take;
            self.results
                .sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        } else {
            self.results.extend(
                self.keys[..take]
                    .iter()
                    .map(|&c| (0.0, g.original(key_id(c)))),
            );
        }
        out.clear();
        out.extend(self.results.iter().take(k).map(|r| r.1));
        if let Some(started) = rerank_started {
            profile.rerank_sort_ns = started.elapsed().as_nanos() as u64;
        }
        if let Some(started) = started {
            profile.total_ns = started.elapsed().as_nanos() as u64;
        }
        Ok(evals)
    }
}

#[cfg(feature = "experimental")]
impl VSearcher<'_> {
    /// Profiles the ordinary row-code path; timers perturb latency and are not QPS evidence.
    pub fn profile_search(
        &mut self,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<QueryProfile, WarmError> {
        let g = self.index;
        if g.has_screen()
            || g.has_blocks()
            || self.knobs() != SearchKnobs::OFF
            || params.neighbour_cap != 0
        {
            return Err(invalid("profiling requires the plain row-code path".into()));
        }
        SearchScratch::validate_query(g, query)?;
        let mut profile = QueryProfile::default();
        self.scratch
            .search_inner::<true, { cfg!(feature = "experimental-batch-pool") }, true, false>(
                g,
                query,
                k,
                params,
                out,
                &mut profile,
            )?;
        Ok(profile)
    }
}
