// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Candidates and list distances of one insertion, reused across a thread's insertions.
#[derive(Default)]
pub(super) struct InsertScratch {
    pub(super) cands: Vec<(f32, u32)>,
    pub(super) d: Vec<f32>,
}

/// Buffers of [`Rows::prune_merged_into`], reused across the targets a worker merges.
#[derive(Default)]
pub(super) struct MergeScratch {
    pub(super) d: Vec<f32>,
    pub(super) cands: Vec<(f32, u32, bool)>,
    pub(super) open: Vec<u32>,
    pub(super) out: Vec<u32>,
}

/// Buffers of [`Rows::fill_one`], reused across the vertices a thread fills.
#[derive(Default)]
pub(super) struct FillScratch {
    pub(super) ids: Vec<u32>,
    pub(super) skip: Vec<u32>,
    pub(super) bounds: Vec<(f32, f32)>,
    pub(super) upper: Vec<f32>,
    pub(super) d: Vec<f32>,
    pub(super) near: Vec<(f32, u32)>,
}

thread_local! {
    static BUILD_VISITED: RefCell<BuildVisited> = RefCell::new(BuildVisited::default());
    static INSERT_SCRATCH: RefCell<InsertScratch> = RefCell::default();
    static FILL_SCRATCH: RefCell<FillScratch> = RefCell::default();
}

pub(super) struct Rows<'a> {
    pub(super) data: &'a [f32],
    pub(super) dim: usize,
    /// Squared norm of every row (Euclidean); `None` for unit rows (angular).
    pub(super) norms: Option<&'a [f32]>,
    /// Bounds that let construction skip exact distances whose outcome they already decide,
    /// or, with `quantized`, the estimates that order the construction search's pool.
    pub(super) codes: Option<&'a BuildCodes>,
    pub(super) quantized: bool,
    /// `(rank, slack)`: [`Rows::greedy_into`] stops expanding at a candidate farther than
    /// `slack` times the `rank`-th best of the pool.
    pub(super) bound: Option<(usize, f32)>,
}

/// One 8-bit code per row (`x = scale q + e` with `|e| <= err`), giving bounds on the
/// computed `d2` of two rows. The construction search, the prune and the fill use them only to
/// settle comparisons the bounds decide; the rest are scored exactly, so the graph is the one
/// exact scoring alone would build.
pub(super) struct BuildCodes {
    pub(super) width: usize,
    pub(super) q: Vec<i8>,
    /// Per-row factors, side by side so that one cache line load serves a row.
    pub(super) meta: Vec<CodeMeta>,
    /// Per-row sum of the code, the correction the VNNI kernels take for a row scored as the
    /// probe ([`vnni::query_sum`]).
    pub(super) sums: Vec<i32>,
    /// Euclidean rows: `meta` carries the squared norm the exact distance uses and the norm.
    pub(super) euclidean: bool,
    /// Euclidean rows: relative f32 rounding slack of the computed distance.
    pub(super) rel_slack: f32,
}

/// The factors of one [`BuildCodes`] row; the norms are 0 for unit rows.
#[derive(Clone, Copy, Default)]
#[repr(C, align(16))]
pub(super) struct CodeMeta {
    pub(super) scale: f32,
    pub(super) err: f32,
    pub(super) n2: f32,
    pub(super) n: f32,
}

impl BuildCodes {
    pub(super) fn new(data: &[f32], dim: usize, norms: Option<&[f32]>) -> Self {
        let n = data.len() / dim;
        let width = dim.next_multiple_of(16);
        let mut q = vec![0i8; n * width];
        let mut meta = vec![CodeMeta::default(); n];
        q.par_chunks_mut(width)
            .zip(meta.par_iter_mut())
            .zip(data.par_chunks(dim))
            .for_each(|((q, m), x)| {
                let (s, e) = (&mut m.scale, &mut m.err);
                let max = x.iter().fold(0f32, |m, v| m.max(v.abs()));
                *s = if max > 0.0 { max / 127.0 } else { 0.0 };
                let mut e2 = 0f64;
                for (o, &v) in q.iter_mut().zip(x) {
                    let c = if *s > 0.0 {
                        (v / *s).round().clamp(-127.0, 127.0)
                    } else {
                        0.0
                    };
                    *o = c as i8;
                    let r = f64::from(v) - f64::from(*s) * f64::from(c);
                    e2 += r * r;
                }
                // Rounded up so that `err` bounds the true residual norm.
                *e = (e2.sqrt() * (1.0 + 1e-6)) as f32 + 1e-7;
            });
        if let Some(norms) = norms {
            for (m, &n2) in meta.iter_mut().zip(norms) {
                (m.n2, m.n) = (n2, n2.sqrt());
            }
        }
        let sums = q.par_chunks(width).map(vnni::query_sum).collect();
        Self {
            width,
            q,
            meta,
            sums,
            euclidean: norms.is_some(),
            // The computed dot product and squared norms are each within `dim * 2^-24` of the
            // magnitudes summed; twice that, and at least the unit-row slack, covers them.
            rel_slack: 1e-4f32.max((dim as f32 + 4.0) * 2.0 * f32::EPSILON),
        }
    }

    pub(super) fn code(&self, i: u32) -> &[i8] {
        let i = i as usize;
        &self.q[i * self.width..(i + 1) * self.width]
    }

    /// Hint the cache to load the code and the factors of row `i`.
    #[inline]
    pub(super) fn prefetch(&self, i: u32) {
        build_kernels::prefetch_code(self.code(i));
        build_kernels::prefetch_meta(&self.meta[i as usize]);
    }

    /// A lower bound on the computed `d2` of the unit rows `p` and `u`. With
    /// `x = s_x q_x + e_x`: `<p, u> <= s_p s_u <q_p, q_u> + |e_p| + |s_p q_p| |e_u|`, and
    /// `|s_p q_p| <= 1 + |e_p|`. `SLACK` covers the f32 rounding of this bound and of the exact
    /// dot product (at most `dim * 2^-24` for unit vectors).
    #[inline]
    pub(super) fn d2_lower(&self, p: u32, u: u32) -> f32 {
        self.d2_bounds(p, u).0
    }

    /// The integer dot product of the codes of rows `p` and `u`.
    #[inline]
    pub(super) fn raw(&self, p: u32, u: u32) -> i64 {
        dot_i8_query(self.code(p), self.code(u), self.sums[p as usize])
    }

    /// The 8-bit estimate of the computed `d2` of the rows `p` and `u`.
    #[inline]
    pub(super) fn d2_est(&self, p: u32, u: u32) -> f32 {
        self.est_from_raw(p, u, self.raw(p, u))
    }

    /// [`Self::d2_est`] of row `p` and four rows, from one four-row kernel call; each value
    /// equals the single call's.
    #[inline]
    pub(super) fn d2_est4(&self, p: u32, u: [u32; 4]) -> [f32; 4] {
        let raw = dot_i8_query_x4(self.code(p), u.map(|u| self.code(u)), self.sums[p as usize]);
        [0, 1, 2, 3].map(|i| self.est_from_raw(p, u[i], raw[i]))
    }

    #[inline]
    pub(super) fn est_from_raw(&self, p: u32, u: u32, raw: i64) -> f32 {
        let raw = raw as f32;
        let (mp, mu) = (&self.meta[p as usize], &self.meta[u as usize]);
        if self.euclidean {
            (mp.n2 + mu.n2 - 2.0 * mp.scale * mu.scale * raw).max(0.0)
        } else {
            (2.0 - 2.0 * mp.scale * mu.scale * raw).max(0.0)
        }
    }

    /// `(lower, upper)` bounds on the computed `d2` of the unit rows `p` and `u`, from the
    /// two-sided form of the inequality in [`Self::d2_lower`].
    /// For rows of any norm the same inequality reads
    /// `|<p, u> - s_p s_u <q_p, q_u>| <= |e_p| |u| + (|p| + |e_p|) |e_u|`, and the computed
    /// distance `|p|^2 + |u|^2 - 2 <p, u>` gets a slack relative to the magnitudes it sums.
    /// Inlined so that [`Self::d2_lower`] drops the unused upper bound.
    #[inline]
    pub(super) fn d2_bounds(&self, p: u32, u: u32) -> (f32, f32) {
        const SLACK: f32 = 1e-4;
        let (mp, mu) = (&self.meta[p as usize], &self.meta[u as usize]);
        let raw = self.raw(p, u) as f32;
        let ep = mp.err;
        let est = mp.scale * mu.scale * raw;
        if !self.euclidean {
            let margin = ep + (1.0 + ep) * mu.err + SLACK;
            return (2.0 - 2.0 * (est + margin), 2.0 - 2.0 * (est - margin));
        }
        let ((np2, np), (nu2, nu)) = ((mp.n2, mp.n), (mu.n2, mu.n));
        let margin = ep * nu + (np + ep) * mu.err;
        let slack = self.rel_slack * (np2 + nu2 + 2.0 * np * nu);
        let base = np2 + nu2;
        (
            base - 2.0 * (est + margin) - slack,
            base - 2.0 * (est - margin) + slack,
        )
    }
}

impl Rows<'_> {
    pub(super) fn row(&self, i: u32) -> &[f32] {
        let i = i as usize;
        &self.data[i * self.dim..(i + 1) * self.dim]
    }

    /// Hint the cache to load row `u` and, for Euclidean rows, its squared norm.
    #[inline]
    pub(super) fn prefetch(&self, u: u32) {
        build_kernels::prefetch_row(self.row(u));
        if let Some(n) = self.norms {
            build_kernels::prefetch_meta(&n[u as usize]);
        }
    }

    /// Squared distance of rows `a` and `b` from their computed dot product: `2 - 2 dot` for
    /// unit rows, `|a|^2 + |b|^2 - 2 dot` with the stored squared norms otherwise.
    #[inline]
    pub(super) fn dist(&self, a: u32, b: u32, dot: f32) -> f32 {
        match self.norms {
            None => (2.0 - 2.0 * dot).max(0.0),
            Some(n) => (n[a as usize] + n[b as usize] - 2.0 * dot).max(0.0),
        }
    }

    /// Squared distance of rows `a` and `b`.
    pub(super) fn d2(&self, a: u32, b: u32) -> f32 {
        self.dist(a, b, dot(self.row(a), self.row(b)))
    }

    /// `d2(p, u)` for every `u` in `ids`, written to `out`.
    pub(super) fn d2_many(&self, p: u32, ids: &[u32], out: &mut Vec<f32>) {
        out.clear();
        let target = self.row(p);
        let (quads, rest) = ids.as_chunks::<4>();
        for q in quads {
            let d = build_kernels::dot4(target, q.map(|u| self.row(u)));
            out.extend(d.iter().zip(q).map(|(&x, &u)| self.dist(p, u, x)));
        }
        out.extend(rest.iter().map(|&u| self.d2(p, u)));
    }

    /// `d2_many` over a long list of rows not yet in cache, prefetching a few rows ahead.
    pub(super) fn d2_stream(&self, p: u32, ids: &[u32], out: &mut Vec<f32>) {
        const AHEAD: usize = 16;
        out.clear();
        let target = self.row(p);
        for &u in ids.iter().take(AHEAD) {
            self.prefetch(u);
        }
        let (quads, rest) = ids.as_chunks::<4>();
        for (i, q) in quads.iter().enumerate() {
            for &u in ids.iter().skip(AHEAD + 4 * i).take(4) {
                self.prefetch(u);
            }
            let d = build_kernels::dot4(target, q.map(|u| self.row(u)));
            out.extend(d.iter().zip(q).map(|(&x, &u)| self.dist(p, u, x)));
        }
        out.extend(rest.iter().map(|&u| self.d2(p, u)));
    }

    /// True if some `s` in `kept` has `a2 d2(s, c) <= dc`.
    pub(super) fn occluded(&self, c: u32, kept: &[u32], dc: f32, a2: f32) -> bool {
        let rc = self.row(c);
        if let Some(codes) = self.codes {
            // The codes settle most pairs; the rest are scored exactly, four at a time, so the
            // answer is the one exact scoring alone gives (it does not depend on the order).
            let mut open = [0u32; 4];
            let mut n_open = 0;
            for &s in kept {
                let (lo, hi) = codes.d2_bounds(c, s);
                if a2 * hi <= dc {
                    return true;
                }
                if a2 * lo > dc {
                    continue;
                }
                open[n_open] = s;
                n_open += 1;
                if n_open == 4 {
                    n_open = 0;
                    let d = build_kernels::dot4(rc, open.map(|u| self.row(u)));
                    if d.iter()
                        .zip(&open)
                        .any(|(&x, &s)| a2 * self.dist(c, s, x) <= dc)
                    {
                        return true;
                    }
                }
            }
            return open[..n_open]
                .iter()
                .any(|&s| a2 * self.dist(s, c, dot(self.row(s), rc)) <= dc);
        }
        let (quads, rest) = kept.as_chunks::<4>();
        for q in quads {
            let d = build_kernels::dot4(rc, q.map(|u| self.row(u)));
            if d.iter()
                .zip(q)
                .any(|(&x, &s)| a2 * self.dist(c, s, x) <= dc)
            {
                return true;
            }
        }
        rest.iter()
            .any(|&s| a2 * self.dist(s, c, dot(self.row(s), rc)) <= dc)
    }

    /// Greedy search from `start`; returns every expanded vertex with its distance to row `p`.
    pub(super) fn greedy(
        &self,
        graph: &Adjacency,
        start: u32,
        p: u32,
        l: usize,
        visited: &mut BuildVisited,
    ) -> Vec<(f32, u32)> {
        let mut expanded = Vec::new();
        self.greedy_into(graph, start, p, l, visited, &mut expanded);
        expanded
    }

    /// [`Self::greedy`] into `expanded` (cleared first), so a caller reuses one buffer.
    pub(super) fn greedy_into(
        &self,
        graph: &Adjacency,
        start: u32,
        p: u32,
        l: usize,
        visited: &mut BuildVisited,
        expanded: &mut Vec<(f32, u32)>,
    ) {
        let (mut n_fresh, mut n_exact) = (0u64, 0u64);
        visited.reset(graph.len());
        visited.insert(start);
        // The thread's buffers, taken for this call and returned at its end.
        let mut pool = std::mem::take(&mut visited.pool);
        let mut fresh = std::mem::take(&mut visited.fresh);
        let mut dists = std::mem::take(&mut visited.dists);
        pool.clear();
        pool.push(Candidate {
            dist: self.d2(p, start),
            id: start,
            done: false,
        });
        expanded.clear();
        let mut cur = 0;
        while cur < pool.len() {
            if let Some((rank, slack)) = self.bound {
                if pool.len() >= rank && pool[cur].dist > slack * pool[rank - 1].dist {
                    break;
                }
            }
            pool[cur].done = true;
            let v = pool[cur].id;
            expanded.push((pool[cur].dist, v));
            // Mark and prefetch every new neighbour first so their rows load concurrently.
            fresh.clear();
            // With a full pool, a neighbour at or beyond the pool bound is discarded below; the
            // codes rule most of them out without loading their rows.
            match self.codes.filter(|_| self.quantized || pool.len() >= l) {
                Some(codes) if self.quantized => {
                    for &u in graph.get(v as usize) {
                        if visited.insert(u) {
                            codes.prefetch(u);
                            fresh.push(u);
                        }
                    }
                    n_fresh += fresh.len() as u64;
                }
                Some(codes) => {
                    for &u in graph.get(v as usize) {
                        if visited.insert(u) {
                            codes.prefetch(u);
                            fresh.push(u);
                        }
                    }
                    let bound = pool[l - 1].dist;
                    n_fresh += fresh.len() as u64;
                    fresh.retain(|&u| codes.d2_lower(p, u) < bound);
                    for &u in &fresh {
                        build_kernels::prefetch_row(self.row(u));
                    }
                }
                None => {
                    for &u in graph.get(v as usize) {
                        if visited.insert(u) {
                            build_kernels::prefetch_row(self.row(u));
                            fresh.push(u);
                        }
                    }
                    n_fresh += fresh.len() as u64;
                }
            }
            match self.codes.filter(|_| self.quantized) {
                Some(codes) => {
                    dists.clear();
                    let (quads, rest) = fresh.as_chunks::<4>();
                    for &q in quads {
                        dists.extend(codes.d2_est4(p, q));
                    }
                    dists.extend(rest.iter().map(|&u| codes.d2_est(p, u)));
                }
                None => {
                    n_exact += fresh.len() as u64;
                    self.d2_many(p, &fresh, &mut dists);
                }
            }
            let mut best = usize::MAX;
            for (&u, &du) in fresh.iter().zip(&dists) {
                #[cfg(feature = "experimental-build-search")]
                let Some(pos) = insert_build_candidate(
                    &mut pool,
                    Candidate {
                        dist: du,
                        id: u,
                        done: false,
                    },
                    l,
                ) else {
                    continue;
                };
                #[cfg(not(feature = "experimental-build-search"))]
                let pos = {
                    if pool.len() >= l && du >= pool[l - 1].dist {
                        continue;
                    }
                    let pos = pool.partition_point(|c| c.dist <= du);
                    pool.insert(
                        pos,
                        Candidate {
                            dist: du,
                            id: u,
                            done: false,
                        },
                    );
                    pool.truncate(l);
                    pos
                };
                best = best.min(pos);
                // The list header of a pooled vertex is read when it is expanded.
                build_kernels::prefetch_meta(graph.len_word(u as usize));
            }
            cur = best.min(cur + 1);
            while cur < pool.len() && pool[cur].done {
                cur += 1;
            }
            if let Some(c) = pool.get(cur) {
                build_kernels::prefetch_ids(graph.get(c.id as usize));
            }
        }
        if self.quantized {
            // Exact distances of the expanded set, which the prune scores exactly.
            fresh.clear();
            fresh.extend(expanded.iter().map(|e| e.1));
            self.d2_stream(p, &fresh, &mut dists);
            n_exact += fresh.len() as u64;
            for (e, &d) in expanded.iter_mut().zip(&dists) {
                e.0 = d;
            }
        }
        (visited.pool, visited.fresh, visited.dists) = (pool, fresh, dists);
        prof::count(&prof::FRESH, n_fresh);
        prof::count(&prof::EXACT, n_exact);
    }

    /// Vamana robust prune: keep `c` unless an already kept `s` has `alpha^2 d2(s, c) <= d2(p, c)`.
    pub(super) fn prune(
        &self,
        p: u32,
        mut cands: Vec<(f32, u32)>,
        alpha: f32,
        r: usize,
    ) -> Vec<u32> {
        let mut out = Vec::with_capacity(r);
        self.prune_into(p, &mut cands, alpha, r, &mut out);
        out
    }

    /// [`Self::prune`] into `out` (cleared first); `cands` is sorted and deduplicated in place.
    pub(super) fn prune_into(
        &self,
        p: u32,
        cands: &mut Vec<(f32, u32)>,
        alpha: f32,
        r: usize,
        out: &mut Vec<u32>,
    ) {
        cands.retain(|c| c.1 != p);
        cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        cands.dedup_by_key(|c| c.1);
        let a2 = alpha * alpha;
        out.clear();
        for &(dc, c) in cands.iter() {
            if out.len() == r {
                break;
            }
            if !self.occluded(c, out, dc, a2) {
                out.push(c);
            }
        }
    }

    /// `prune` of `list` (distinct ids, not `t`) when its first `closed` entries are an
    /// unchanged `prune` output for `t` under the same `alpha`. Such an entry was not occluded
    /// by the closed entries nearer than it, so it is checked only against kept entries from
    /// the rest of the list; the result equals `prune` of the whole list.
    #[cfg(test)]
    pub(super) fn prune_merged(
        &self,
        t: u32,
        list: &[u32],
        closed: usize,
        alpha: f32,
        r: usize,
    ) -> Vec<u32> {
        let mut scratch = MergeScratch::default();
        self.prune_merged_into(t, list, closed, alpha, r, &mut scratch);
        scratch.out
    }

    /// [`Self::prune_merged`] into `scratch.out`, reusing the scratch buffers.
    pub(super) fn prune_merged_into(
        &self,
        t: u32,
        list: &[u32],
        closed: usize,
        alpha: f32,
        r: usize,
        scratch: &mut MergeScratch,
    ) {
        let MergeScratch {
            d,
            cands,
            open,
            out,
        } = scratch;
        self.d2_stream(t, list, d);
        cands.clear();
        cands.extend(
            d.iter()
                .zip(list)
                .enumerate()
                .map(|(i, (&d, &u))| (d, u, i < closed)),
        );
        cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let a2 = alpha * alpha;
        out.clear();
        open.clear();
        for &(dc, c, was_kept) in cands.iter() {
            if out.len() == r {
                break;
            }
            if !self.occluded(c, if was_kept { open } else { out }, dc, a2) {
                out.push(c);
                if !was_kept {
                    open.push(c);
                }
            }
        }
    }

    /// The position of every row in an order that keeps near rows together: the rows are
    /// grouped by their nearest of `cells` sampled rows, and the groups follow a nearest-
    /// neighbour tour of the samples. Empty when the build is too small to gain from it.
    pub(super) fn locality_rank(&self, order: &[u32]) -> Vec<u32> {
        let n = order.len();
        if n < 4 * LOCALITY_MIN_BATCH {
            return Vec::new();
        }
        let cells =
            ((n as f64).sqrt() as usize / 4).clamp(*LOCALITY_CELLS.start(), *LOCALITY_CELLS.end());
        let centers = &order[..cells];
        let cell_of: Vec<u32> = (0..n as u32)
            .into_par_iter()
            .map_init(Vec::new, |d, p| {
                self.d2_many(p, centers, d);
                let mut best = 0;
                for (i, &x) in d.iter().enumerate() {
                    if x < d[best] {
                        best = i;
                    }
                }
                best as u32
            })
            .collect();
        // Nearest-neighbour tour of the cell centres.
        let mut tour = Vec::with_capacity(cells);
        let mut left: Vec<u32> = (1..cells as u32).collect();
        let mut at = 0u32;
        tour.push(0u32);
        let mut d = Vec::new();
        while !left.is_empty() {
            let ids: Vec<u32> = left.iter().map(|&c| centers[c as usize]).collect();
            self.d2_many(centers[at as usize], &ids, &mut d);
            let mut best = 0;
            for (i, &x) in d.iter().enumerate() {
                if x < d[best] {
                    best = i;
                }
            }
            at = left.swap_remove(best);
            tour.push(at);
        }
        let mut place = vec![0u32; cells];
        for (i, &c) in tour.iter().enumerate() {
            place[c as usize] = i as u32;
        }
        cell_of.iter().map(|&c| place[c as usize]).collect()
    }

    /// One insertion pass in batches of doubling size; each batch searches a frozen graph, so
    /// the result does not depend on thread scheduling.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn insert_pass(
        &self,
        graph: &mut Adjacency,
        order: &[u32],
        entry: u32,
        l: usize,
        alpha: f32,
        r: usize,
        rank: &[u32],
    ) {
        let n = graph.len();
        // Length of the prefix of each list that is an unchanged output of `prune` in this pass.
        let mut closed = vec![0u32; n];
        let max_batch = (n / 50).max(64);
        let mut outs = Adjacency::new(max_batch.min(order.len()), r);
        let (mut start, mut size) = (0, 1);
        while start < order.len() {
            let end = (start + size).min(order.len());
            // Each batch searches a frozen graph and its lists are merged in sorted order, so
            // the order the points run in cannot change the result. Running neighbours of the
            // space together keeps their rows and lists in the cache.
            let mut local;
            let batch = if !rank.is_empty() && end - start >= LOCALITY_MIN_BATCH {
                local = order[start..end].to_vec();
                local.sort_unstable_by_key(|&p| rank[p as usize]);
                &local[..]
            } else {
                &order[start..end]
            };
            let g: &Adjacency = graph;
            outs.par_set(batch.len(), |i, out| {
                let p = batch[i];
                INSERT_SCRATCH.with(|cell| {
                    let InsertScratch { cands, d } = &mut *cell.borrow_mut();
                    let t0 = prof::start();
                    BUILD_VISITED.with(|vis| {
                        self.greedy_into(g, entry, p, l, &mut vis.borrow_mut(), cands);
                    });
                    prof::add(&prof::GREEDY, t0);
                    let t0 = prof::start();
                    let adj = g.get(p as usize);
                    self.d2_stream(p, adj, d);
                    cands.extend(d.iter().copied().zip(adj.iter().copied()));
                    self.prune_into(p, cands, alpha, r, out);
                    prof::add(&prof::PRUNE, t0);
                });
            });
            let trev = prof::start();
            let mut rev: Vec<(u32, u32)> = batch
                .iter()
                .enumerate()
                .flat_map(|(i, &p)| outs.get(i).iter().map(move |&u| (u, p)))
                .collect();
            for (i, &p) in batch.iter().enumerate() {
                let out = outs.get(i);
                closed[p as usize] = out.len() as u32;
                graph.set(p as usize, out);
            }
            rev.par_sort_unstable();
            // Each group updates only its own target's list, so the groups run in place.
            let groups: Vec<_> = rev.chunk_by(|a, b| a.0 == b.0).collect();
            let targets = || groups.iter().map(|grp| grp[0].0 as usize);
            let closed_of = pick_mut(&mut closed, targets());
            let jobs: Vec<_> = graph
                .rows_mut(targets())
                .zip(closed_of)
                .zip(&groups)
                .collect();
            jobs.into_par_iter().for_each_init(
                <(Vec<u32>, MergeScratch)>::default,
                |(merged, scratch), (((row, len), closed), grp)| {
                    let t = grp[0].0;
                    merged.clear();
                    merged.extend_from_slice(&row[..*len as usize]);
                    for &(_, s) in grp.iter() {
                        if !merged.contains(&s) {
                            merged.push(s);
                        }
                    }
                    let list = if merged.len() > r {
                        self.prune_merged_into(t, merged, *closed as usize, alpha, r, scratch);
                        *closed = scratch.out.len() as u32;
                        &scratch.out
                    } else {
                        &*merged
                    };
                    row[..list.len()].copy_from_slice(list);
                    *len = list.len() as u32;
                },
            );
            prof::add(&prof::REV, trev);
            start = end;
            size = (size * 2).min(max_batch);
        }
    }

    /// Fill every adjacency list up to `r` with the nearest two-hop neighbours; the
    /// search estimates a full batch per expansion anyway, so the extra edges are free.
    pub(super) fn fill(&self, graph: &mut Adjacency, r: usize) {
        graph.par_append(|g, v, out| {
            let have = g.get(v);
            if have.len() >= r {
                return;
            }
            FILL_SCRATCH.with(|cell| self.fill_one(g, v, r, &mut cell.borrow_mut(), out));
        });
    }

    /// The fill of vertex `v` (whose list is shorter than `r`), appended to `out`.
    pub(super) fn fill_one(
        &self,
        g: &Adjacency,
        v: usize,
        r: usize,
        s: &mut FillScratch,
        out: &mut Vec<u32>,
    ) {
        let FillScratch {
            ids,
            skip,
            bounds,
            upper,
            d,
            near,
        } = s;
        let have = g.get(v);
        ids.clear();
        ids.extend(have.iter().flat_map(|&u| g.get(u as usize).iter().copied()));
        ids.sort_unstable();
        ids.dedup();
        // `ids` is sorted, so one merge with the sorted list drops the ids in it.
        skip.clear();
        skip.extend_from_slice(have);
        skip.push(v as u32);
        skip.sort_unstable();
        let mut at = 0;
        ids.retain(|&u| {
            while at < skip.len() && skip[at] < u {
                at += 1;
            }
            skip.get(at) != Some(&u)
        });
        let k = r - have.len();
        if let Some(codes) = self.codes.filter(|_| ids.len() > k) {
            // At least `k` ids score at most the k-th smallest upper bound, so an id
            // whose lower bound exceeds it cannot be among the `k` nearest.
            const AHEAD: usize = 8;
            ids.iter().take(AHEAD).for_each(|&u| codes.prefetch(u));
            bounds.clear();
            bounds.extend(ids.iter().enumerate().map(|(i, &u)| {
                if let Some(&w) = ids.get(i + AHEAD) {
                    codes.prefetch(w);
                }
                codes.d2_bounds(v as u32, u)
            }));
            upper.clear();
            upper.extend(bounds.iter().map(|b| b.1));
            let cut = *upper.select_nth_unstable_by(k - 1, f32::total_cmp).1;
            let mut b = bounds.iter();
            ids.retain(|_| b.next().is_some_and(|b| b.0 <= cut));
        }
        self.d2_stream(v as u32, ids, d);
        near.clear();
        near.extend(d.iter().copied().zip(ids.iter().copied()));
        // The ids are distinct, so the order is total and the `k` nearest, sorted, are
        // the first `k` of a full sort.
        let order = |a: &(f32, u32), b: &(f32, u32)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
        if near.len() > k {
            near.select_nth_unstable_by(k - 1, order);
            near.truncate(k);
        }
        near.sort_by(order);
        out.extend(near.iter().map(|x| x.1));
    }
}

/// The entries of `values` at `ids` (strictly increasing), as disjoint mutable references.
pub(super) fn pick_mut<T>(
    values: &mut [T],
    ids: impl Iterator<Item = usize>,
) -> impl Iterator<Item = &mut T> {
    let mut rest = values;
    let mut next = 0;
    ids.map(move |v| {
        assert!(v >= next, "ids must be strictly increasing");
        let (x, tail) = std::mem::take(&mut rest)[v - next..]
            .split_first_mut()
            .expect("id in range");
        rest = tail;
        next = v + 1;
        x
    })
}

/// Per-edge code and factors for the edge v -> u (rotated unit vectors).
/// Returns `(K, M, popcount)` with `K = <v, w> - M <s, v>`.
pub(super) fn edge_factors(pv: &[f32], pu: &[f32], code: &mut [u64]) -> (f32, f32, f32) {
    let (mut r2, mut l1, mut sv, mut vw) = (0f32, 0f32, 0f32, 0f32);
    // The padded length is a multiple of 64, so every code word is written.
    for ((a64, b64), word) in pv
        .as_chunks::<64>()
        .0
        .iter()
        .zip(pu.as_chunks::<64>().0)
        .zip(code.iter_mut())
    {
        let mut bits = 0u64;
        for (i, (&a, &b)) in a64.iter().zip(b64).enumerate() {
            let w = b - a;
            r2 += w * w;
            l1 += w.abs();
            vw += a * w;
            let up = w > 0.0;
            sv += if up { a } else { -a };
            bits |= u64::from(up) << i;
        }
        *word = bits;
    }
    let m = if l1 > 0.0 { r2 / l1 } else { 0.0 };
    let pop = code.iter().map(|c| c.count_ones()).sum::<u32>() as f32;
    (vw - m * sv, m, pop)
}
