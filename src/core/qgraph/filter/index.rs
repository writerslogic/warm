// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl FilteredIndex {
    /// Builds the index over `n = data.len() / dim` rows of `data` (row-major u8) and their
    /// labels as CSR (`label_indptr` with `n + 1` entries into `label_ids`). `dim` must be a
    /// in `1..=MAX_DIMENSIONS`, label ids below `labels` (0 infers it from the data),
    /// with at most `MAX_LABELS` labels. Rows are padded internally for the kernels.
    pub fn build(
        data: &[u8],
        dim: usize,
        label_indptr: &[u64],
        label_ids: &[u32],
        labels: usize,
        params: &FilterBuildParams,
    ) -> Result<Self, FilterError> {
        if !(1..=MAX_DIMENSIONS).contains(&dim) {
            return Err(FilterError::Invalid(format!(
                "dim must be in 1..={MAX_DIMENSIONS}"
            )));
        }
        if !data.len().is_multiple_of(dim) {
            return Err(FilterError::Invalid(
                "data is not a whole number of rows".into(),
            ));
        }
        let n = data.len() / dim;
        if n > u32::MAX as usize - 1 {
            return Err(FilterError::Invalid(format!("{n} rows exceed u32 ids")));
        }
        if label_indptr.len() != n + 1
            || label_indptr[0] != 0
            || label_indptr.windows(2).any(|w| w[0] > w[1])
            || label_indptr[n] != label_ids.len() as u64
        {
            return Err(FilterError::Invalid(
                "label_indptr is not a monotone CSR index over label_ids with n + 1 entries".into(),
            ));
        }
        let max_label = label_ids
            .iter()
            .copied()
            .max()
            .map_or(0, |m| m as usize + 1);
        let labels = if labels == 0 { max_label } else { labels };
        if labels > MAX_LABELS {
            return Err(FilterError::Invalid(format!("labels exceed {MAX_LABELS}")));
        }
        if max_label > labels {
            return Err(FilterError::Invalid(format!(
                "label id {} is out of range for {labels} labels",
                max_label - 1
            )));
        }
        if params.pq_m != 0 && params.pq_sample == 0 {
            return Err(FilterError::Invalid("pq_sample must be positive".into()));
        }
        if params.cluster_size == 0 || params.max_lists == 0 || params.train_per_list == 0 {
            return Err(FilterError::Invalid(
                "cluster_size, max_lists and train_per_list must be positive".into(),
            ));
        }

        // Rows are zero-padded (in the shifted domain) to a multiple of 16 for the kernels.
        let stride = dim.next_multiple_of(16);
        let mut rows = allocation(product(n, stride)?, 0i8)?;
        let mut norms = allocation(n, 0i32)?;
        rows.par_chunks_mut(stride)
            .zip(data.par_chunks(dim))
            .zip(norms.par_iter_mut())
            .for_each(|((out, x), norm)| {
                let mut s = 0i32;
                for (o, &v) in out.iter_mut().zip(x) {
                    *o = shift(v);
                    s += i32::from(*o) * i32::from(*o);
                }
                *norm = s;
            });

        // Transpose the point -> labels CSR with a counting sort: ids come out ascending.
        let mut counts = allocation(labels + 1, 0u64)?;
        for &l in label_ids {
            counts[l as usize + 1] += 1;
        }
        for i in 0..labels {
            counts[i + 1] += counts[i];
        }
        let label_offsets = counts.clone();
        let mut fill = counts;
        let mut postings = allocation(label_ids.len(), 0u32)?;
        for p in 0..n {
            let (a, b) = (label_indptr[p] as usize, label_indptr[p + 1] as usize);
            for &l in &label_ids[a..b] {
                let at = &mut fill[l as usize];
                postings[*at as usize] = p as u32;
                *at += 1;
            }
        }
        // A point listing a label twice would appear twice; collapse adjacent repeats.
        let mut dedup_offsets = allocation(labels + 1, 0u64)?;
        let mut write = 0usize;
        for l in 0..labels {
            let (a, b) = (label_offsets[l] as usize, label_offsets[l + 1] as usize);
            let mut prev = u32::MAX;
            for r in a..b {
                let id = postings[r];
                if id != prev {
                    postings[write] = id;
                    write += 1;
                    prev = id;
                }
            }
            dedup_offsets[l + 1] = write as u64;
        }
        postings.truncate(write);
        let label_offsets = dedup_offsets;

        let words = n.div_ceil(64);
        let mut bitset_slot = allocation(labels, u32::MAX)?;
        let dense: Vec<usize> = (0..labels)
            .filter(|&l| {
                let len = (label_offsets[l + 1] - label_offsets[l]) as usize;
                len > 0 && len >= params.bitset_min
            })
            .collect();
        for (slot, &l) in dense.iter().enumerate() {
            bitset_slot[l] = slot as u32;
        }
        let mut bitsets = allocation(product(dense.len(), words)?, 0u64)?;
        bitsets
            .par_chunks_mut(words.max(1))
            .zip(dense.par_iter())
            .for_each(|(set, &l)| {
                let (a, b) = (label_offsets[l] as usize, label_offsets[l + 1] as usize);
                for &id in &postings[a..b] {
                    set[(id >> 6) as usize] |= 1 << (id & 63);
                }
            });

        let mut ivf_slot = allocation(labels, u32::MAX)?;
        let mut big: Vec<usize> = (0..labels)
            .filter(|&l| (label_offsets[l + 1] - label_offsets[l]) as usize > params.ivf_min)
            .collect();
        // Largest first so the long trainings start early.
        big.sort_by_key(|&l| std::cmp::Reverse(label_offsets[l + 1] - label_offsets[l]));
        let pq = if params.pq_m != 0 && !big.is_empty() {
            Some(Pq::train(
                &rows,
                n,
                stride,
                params.pq_m,
                params.pq_sample,
                params.pq_iters,
                params.seed,
            )?)
        } else {
            None
        };
        let point_codes = match &pq {
            Some(pq) => pq.encode_all(&rows, stride, n)?,
            None => Vec::new(),
        };
        let ivfs: Vec<LabelIvf> = big
            .par_iter()
            .map(|&l| {
                let (a, b) = (label_offsets[l] as usize, label_offsets[l + 1] as usize);
                train_label(
                    &rows,
                    &norms,
                    stride,
                    &postings[a..b],
                    params,
                    l as u64,
                    pq.as_ref().map(|p| (p, point_codes.as_slice())),
                )
            })
            .collect::<Result<_, _>>()?;
        for (slot, &l) in big.iter().enumerate() {
            ivf_slot[l] = slot as u32;
        }

        Ok(Self {
            dim,
            stride,
            n,
            rows,
            norms,
            label_offsets,
            postings,
            bitset_slot,
            bitsets,
            ivf_slot,
            ivfs,
            pq,
            deleted: (0..words).map(|_| AtomicU64::new(0)).collect(),
            live: n,
            tuning: FilterTuning::default(),
            brute_queries: AtomicU64::new(0),
            ivf_queries: AtomicU64::new(0),
            scored: AtomicU64::new(0),
        })
    }

    /// Sets the execution choices of the searches that follow.
    pub fn set_tuning(&mut self, tuning: FilterTuning) {
        self.tuning = tuning;
    }

    /// Row width.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Rows indexed, deleted ones included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    /// Whether no rows are indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Rows not deleted.
    #[must_use]
    pub fn live(&self) -> usize {
        self.live
    }

    /// Number of labels.
    #[must_use]
    pub fn labels(&self) -> usize {
        self.label_offsets.len() - 1
    }

    /// Members of `label` (deleted ones included).
    #[must_use]
    pub fn label_len(&self, label: u32) -> usize {
        let l = label as usize;
        if l >= self.labels() {
            return 0;
        }
        (self.label_offsets[l + 1] - self.label_offsets[l]) as usize
    }

    /// Labels that have an inverted file.
    #[must_use]
    pub fn ivf_labels(&self) -> usize {
        self.ivfs.len()
    }

    /// Bytes held by the index structures (rows, postings, bitsets, inverted files).
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.rows.len()
            + self.norms.len() * 4
            + self.label_offsets.len() * 8
            + self.postings.len() * 4
            + self.bitsets.len() * 8
            + self.deleted.len() * 8
            + self
                .ivfs
                .iter()
                .map(|v| {
                    v.centroids.len()
                        + v.centroid_norms.len() * 4
                        + v.offsets.len() * 4
                        + v.members.len() * 4
                        + v.rows.len()
                        + v.norms.len() * 4
                        + v.codes.len()
                        + v.bids.len() * 4
                        + v.blocks.len() * 4
                })
                .sum::<usize>()
            + self.pq.as_ref().map_or(0, Pq::bytes)
    }

    /// Marks `id` deleted; it is skipped by every search from now on. Returns whether it was
    /// live.
    pub fn delete(&mut self, id: u32) -> bool {
        if id as usize >= self.n {
            return false;
        }
        let w = &self.deleted[(id >> 6) as usize];
        let mask = 1u64 << (id & 63);
        let was = w.fetch_or(mask, Ordering::Relaxed) & mask == 0;
        if was {
            self.live -= 1;
        }
        was
    }

    #[inline]
    pub(super) fn is_deleted(&self, id: u32) -> bool {
        if self.tuning.skip_tombstones && self.live == self.n {
            return false;
        }
        self.deleted[(id >> 6) as usize].load(Ordering::Relaxed) >> (id & 63) & 1 == 1
    }

    pub(super) fn posting(&self, label: u32) -> &[u32] {
        let l = label as usize;
        &self.postings[self.label_offsets[l] as usize..self.label_offsets[l + 1] as usize]
    }

    pub(super) fn bitset(&self, label: u32) -> Option<&[u64]> {
        let slot = self.bitset_slot[label as usize];
        (slot != u32::MAX).then(|| {
            let words = self.n.div_ceil(64);
            &self.bitsets[slot as usize * words..(slot as usize + 1) * words]
        })
    }

    #[inline]
    pub(super) fn row(&self, id: u32) -> &[i8] {
        let at = id as usize * self.stride;
        &self.rows[at..at + self.stride]
    }

    /// Path counters since the last [`Self::reset_stats`].
    #[must_use]
    pub fn stats(&self) -> FilterStats {
        FilterStats {
            brute: self.brute_queries.load(Ordering::Relaxed),
            ivf: self.ivf_queries.load(Ordering::Relaxed),
            scored: self.scored.load(Ordering::Relaxed),
        }
    }

    /// Clears the path counters.
    pub fn reset_stats(&self) {
        self.brute_queries.store(0, Ordering::Relaxed);
        self.ivf_queries.store(0, Ordering::Relaxed);
        self.scored.store(0, Ordering::Relaxed);
    }

    /// The `k` nearest live points holding every label in `labels`, nearest first, written to
    /// `out` (cleared first) as `(squared distance, id)`. An empty `labels` matches nothing;
    /// a label id out of range matches nothing.
    pub fn search(
        &self,
        query: &[u8],
        labels: &[u32],
        k: usize,
        params: &FilterSearchParams,
        out: &mut Vec<(u32, u32)>,
    ) -> Result<(), FilterError> {
        self.search_with_scratch(query, labels, k, params, out, &mut SearchScratch::default())
    }

    pub(super) fn search_with_scratch(
        &self,
        query: &[u8],
        labels: &[u32],
        k: usize,
        params: &FilterSearchParams,
        out: &mut Vec<(u32, u32)>,
        scratch: &mut SearchScratch,
    ) -> Result<(), FilterError> {
        out.clear();
        if query.len() != self.dim {
            return Err(FilterError::Invalid(format!(
                "query has {} coordinates, index {}",
                query.len(),
                self.dim
            )));
        }
        if k == 0 || labels.is_empty() || labels.iter().any(|&l| l as usize >= self.labels()) {
            return Ok(());
        }
        let SearchScratch {
            labels: order,
            query: q,
            top,
            matches,
            matches_for,
            cursors,
            ivf: ivf_scratch,
            fs: fs_scratch,
        } = scratch;
        order.clear();
        order.extend_from_slice(labels);
        order.sort_by_key(|&l| (self.label_len(l), l));
        order.dedup();
        let rarest = order[0];
        let others = &order[1..];
        let k = k.min(self.live).min(self.label_len(rarest));

        q.resize(self.stride, 0);
        q[self.dim..].fill(0);
        for (o, &v) in q.iter_mut().zip(query) {
            *o = shift(v);
        }
        let qsum: i32 = q.iter().map(|&v| i32::from(v)).sum();
        let qnorm: i64 = q.iter().map(|&v| i64::from(v) * i64::from(v)).sum();
        top.k = k;
        top.items.clear();
        top.items.try_reserve(k + 1).map_err(|_| {
            FilterError::Invalid("filtered result allocation exceeds capacity".into())
        })?;

        let n_rare = self.label_len(rarest);
        let ivf = self.ivf_slot[rarest as usize];
        let scored = if n_rare <= params.brute_max || ivf == u32::MAX {
            self.brute_queries.fetch_add(1, Ordering::Relaxed);
            if self.tuning.cache_intersections && !others.is_empty() {
                self.cached_intersect(order, matches, matches_for, cursors);
                self.score_ids(q, qsum, matches, top);
                matches.len()
            } else {
                self.score_intersection((q, qsum), rarest, others, top, cursors)
            }
        } else if let Some(scored) = self.fs_route(
            &self.ivfs[ivf as usize],
            (q, qsum),
            others,
            (params, n_rare),
            top,
            fs_scratch,
        ) {
            self.ivf_queries.fetch_add(1, Ordering::Relaxed);
            scored
        } else {
            let intersected = !others.is_empty() && n_rare <= params.intersect_max;
            if intersected {
                if self.tuning.cache_intersections {
                    self.cached_intersect(order, matches, matches_for, cursors);
                } else {
                    self.intersect(rarest, others, matches, cursors);
                }
            }
            if intersected
                && (matches.len() <= params.brute_max || self.ivfs[ivf as usize].members.is_empty())
            {
                self.brute_queries.fetch_add(1, Ordering::Relaxed);
                self.score_ids(q, qsum, matches, top);
                matches.len()
            } else {
                self.ivf_queries.fetch_add(1, Ordering::Relaxed);
                let params = &self.classed(params, n_rare, !others.is_empty());
                self.search_ivf(
                    &self.ivfs[ivf as usize],
                    (q, qsum),
                    others,
                    params,
                    top,
                    ivf_scratch,
                )
            }
        };
        self.scored.fetch_add(scored as u64, Ordering::Relaxed);
        out.extend(top.items.iter().map(|&(key, id)| {
            (
                u32::try_from(key + qnorm).expect("validated filtered distance exceeds u32"),
                id,
            )
        }));
        Ok(())
    }

    /// Whether `id` holds every label in `others` (bitsets first, then sorted postings).
    #[inline]
    pub(super) fn holds(&self, id: u32, others: &[u32], cursors: &mut [usize]) -> bool {
        for (j, &l) in others.iter().enumerate() {
            if let Some(set) = self.bitset(l) {
                if !bit(set, id) {
                    return false;
                }
            } else {
                // Ids arrive ascending, so a forward galloping cursor per label suffices.
                let p = self.posting(l);
                let c = &mut cursors[j];
                if *c < p.len() && p[*c] < id {
                    // Invariant: p[lo] < id.
                    let (mut lo, mut step) = (*c, 1);
                    while lo + step < p.len() && p[lo + step] < id {
                        lo += step;
                        step *= 2;
                    }
                    let hi = (lo + step + 1).min(p.len());
                    *c = lo + 1 + p[lo + 1..hi].partition_point(|&v| v < id);
                }
                if *c >= p.len() || p[*c] != id {
                    return false;
                }
            }
        }
        true
    }

    /// Appends to `out` the entries of `ids` (ascending) that are live and hold every label
    /// of `others`: the id itself, or `base + offset` when `positions` is set. Labels with a
    /// bitset are tested for 64 ids at a time without branching, the words of the next block
    /// prefetched; the rest go through [`Self::holds`] for the survivors.
    pub(super) fn match_block(
        &self,
        ids: &[u32],
        base: u32,
        positions: bool,
        others: &[u32],
        cursors: &mut [usize],
        out: &mut Vec<u32>,
    ) {
        const MAX_SETS: usize = 4;
        let mut sets: [&[u64]; MAX_SETS] = [&[]; MAX_SETS];
        let mut in_set = [false; MAX_SETS];
        let mut used = 0;
        let mut rest = false;
        if others.len() <= MAX_SETS {
            for (j, &l) in others.iter().enumerate() {
                if let Some(set) = self.bitset(l) {
                    sets[used] = set;
                    used += 1;
                    in_set[j] = true;
                } else {
                    rest = true;
                }
            }
        }
        if used == 0 || others.len() > MAX_SETS {
            for (j, &id) in ids.iter().enumerate() {
                if !self.is_deleted(id) && self.holds(id, others, cursors) {
                    out.push(if positions { base + j as u32 } else { id });
                }
            }
            return;
        }
        let check_deleted = !(self.tuning.skip_tombstones && self.live == self.n);
        let mut offset = 0;
        for block in ids.chunks(64) {
            let mut mask = u64::MAX >> (64 - block.len());
            for set in &sets[..used] {
                let mut m = 0u64;
                for (j, &id) in block.iter().enumerate() {
                    m |= u64::from(bit(set, id)) << j;
                }
                mask &= m;
            }
            if let Some(next) = ids.get(offset + 64..) {
                for set in &sets[..used] {
                    for &id in next.iter().take(64) {
                        prefetch_word(&set[(id >> 6) as usize]);
                    }
                }
            }
            while mask != 0 {
                let j = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let id = block[j];
                if check_deleted && self.is_deleted(id) {
                    continue;
                }
                if rest {
                    // Cursors advance with ids, so only passing ids may reach `holds`.
                    let mut pass = true;
                    for (k, &l) in others.iter().enumerate() {
                        if !in_set[k]
                            && !self.holds(id, std::slice::from_ref(&l), &mut cursors[k..=k])
                        {
                            pass = false;
                            break;
                        }
                    }
                    if !pass {
                        continue;
                    }
                }
                out.push(if positions {
                    base + (offset + j) as u32
                } else {
                    id
                });
            }
            offset += 64;
        }
    }

    /// [`Self::intersect`] of `labels` (sorted by size, the rarest first), skipped when
    /// `matches` already holds exactly that intersection.
    pub(super) fn cached_intersect(
        &self,
        labels: &[u32],
        matches: &mut Vec<u32>,
        matches_for: &mut Vec<u32>,
        cursors: &mut Vec<usize>,
    ) {
        if matches_for.as_slice() == labels {
            return;
        }
        matches_for.clear();
        self.intersect(labels[0], &labels[1..], matches, cursors);
        matches_for.extend_from_slice(labels);
    }

    /// The live members of `rarest` that hold every label in `others`, ascending.
    pub(super) fn intersect(
        &self,
        rarest: u32,
        others: &[u32],
        matches: &mut Vec<u32>,
        cursors: &mut Vec<usize>,
    ) {
        matches.clear();
        cursors.resize(others.len(), 0);
        cursors.fill(0);
        if self.tuning.block_filter {
            self.match_block(self.posting(rarest), 0, false, others, cursors, matches);
            return;
        }
        matches.extend(
            self.posting(rarest)
                .iter()
                .copied()
                .filter(|&id| !self.is_deleted(id) && self.holds(id, others, cursors)),
        );
    }

    pub(super) fn score_intersection(
        &self,
        query: (&[i8], i32),
        rarest: u32,
        others: &[u32],
        top: &mut TopK,
        cursors: &mut Vec<usize>,
    ) -> usize {
        let (q, qsum) = query;
        let posting = self.posting(rarest);
        if others.is_empty() && self.live == self.n {
            self.score_ids(q, qsum, posting, top);
            return posting.len();
        }
        cursors.clear();
        cursors.resize(others.len(), 0);
        if self.tuning.block_filter {
            let mut batch = Vec::with_capacity(256);
            let mut scored = 0;
            for chunk in posting.chunks(128) {
                self.match_block(chunk, 0, false, others, cursors, &mut batch);
                if batch.len() >= 128 {
                    self.score_ids(q, qsum, &batch, top);
                    scored += batch.len();
                    batch.clear();
                }
            }
            self.score_ids(q, qsum, &batch, top);
            return scored + batch.len();
        }
        let mut batch = [0u32; 128];
        let mut used = 0;
        let mut scored = 0;
        for &id in posting {
            if !self.is_deleted(id) && self.holds(id, others, cursors) {
                batch[used] = id;
                used += 1;
                if used == batch.len() {
                    self.score_ids(q, qsum, &batch, top);
                    scored += used;
                    used = 0;
                }
            }
        }
        self.score_ids(q, qsum, &batch[..used], top);
        scored + used
    }

    /// Scores every id of `ids` (all live and matching) into `top`.
    pub(super) fn score_ids(&self, q: &[i8], qsum: i32, ids: &[u32], top: &mut TopK) {
        let ahead = self.tuning.prefetch;
        let wide = self.tuning.wide;
        let (chunks, tail) = ids.as_chunks::<4>();
        for (c, ch) in chunks.iter().enumerate() {
            for &p in ids
                .iter()
                .skip(c * 4 + ahead)
                .take(if ahead == 0 { 0 } else { 4 })
            {
                prefetch_bytes(self.row(p));
                prefetch_norm(&self.norms[p as usize]);
            }
            let rows = [
                self.row(ch[0]),
                self.row(ch[1]),
                self.row(ch[2]),
                self.row(ch[3]),
            ];
            let d = if wide {
                dot_i8_query_x4_wide(q, rows, qsum)
            } else {
                dot_i8_query_x4(q, rows, qsum)
            };
            for j in 0..4 {
                let key = i64::from(self.norms[ch[j] as usize]) - 2 * d[j];
                top.push(key, ch[j]);
            }
        }
        for &id in tail {
            let key = i64::from(self.norms[id as usize]) - 2 * dot_i8_query(q, self.row(id), qsum);
            top.push(key, id);
        }
    }

    /// Scores the copied rows at positions `pos` of `ivf` into `top`.
    pub(super) fn score_copied(
        &self,
        ivf: &LabelIvf,
        q: &[i8],
        qsum: i32,
        pos: &[u32],
        top: &mut TopK,
    ) {
        let s = self.stride;
        let row = |p: u32| &ivf.rows[p as usize * s..(p as usize + 1) * s];
        let (chunks, tail) = pos.as_chunks::<4>();
        for ch in chunks {
            let d = dot_i8_query_x4(q, [row(ch[0]), row(ch[1]), row(ch[2]), row(ch[3])], qsum);
            for (&p, &dot) in ch.iter().zip(&d) {
                let key = i64::from(ivf.norms[p as usize]) - 2 * dot;
                top.push(key, ivf.members[p as usize]);
            }
        }
        for &p in tail {
            let key = i64::from(ivf.norms[p as usize]) - 2 * dot_i8_query(q, row(p), qsum);
            top.push(key, ivf.members[p as usize]);
        }
    }

    /// Appends to `order` every list of `ivf` as `(centroid key, list)`, unsorted.
    pub(super) fn rank_lists(
        &self,
        ivf: &LabelIvf,
        q: &[i8],
        qsum: i32,
        order: &mut Vec<(i64, u32)>,
    ) {
        let lists = ivf.centroid_norms.len();
        order.clear();
        let centroid = |c: usize| &ivf.centroids[c * self.stride..(c + 1) * self.stride];
        let grouped = lists / 4 * 4;
        for c in (0..grouped).step_by(4) {
            let dots = dot_i8_query_x4(
                q,
                [
                    centroid(c),
                    centroid(c + 1),
                    centroid(c + 2),
                    centroid(c + 3),
                ],
                qsum,
            );
            for (j, dot) in dots.into_iter().enumerate() {
                let key = i64::from(ivf.centroid_norms[c + j]) - 2 * dot;
                order.push((key, (c + j) as u32));
            }
        }
        for (c, &norm) in ivf.centroid_norms[..lists].iter().enumerate().skip(grouped) {
            let key = i64::from(norm) - 2 * dot_i8_query(q, centroid(c), qsum);
            order.push((key, c as u32));
        }
    }

    /// `params` with the `(target, rerank)` of the query's class.
    pub(super) fn classed(
        &self,
        params: &FilterSearchParams,
        n_rare: usize,
        multi: bool,
    ) -> FilterSearchParams {
        let t = &self.tuning;
        let band = t.class_edges.iter().filter(|&&e| n_rare > e).count();
        let (target, rerank) = t.class_params[usize::from(multi)][band];
        FilterSearchParams {
            target: if target == 0 { params.target } else { target },
            rerank: if rerank == 0 { params.rerank } else { rerank },
            ..*params
        }
    }

    /// The 4-bit screen route of a label with codes, when `params.rerank` asks for it.
    pub(super) fn fs_route(
        &self,
        ivf: &LabelIvf,
        query: (&[i8], i32),
        others: &[u32],
        (params, n_rare): (&FilterSearchParams, usize),
        top: &mut TopK,
        scratch: &mut FsScratch,
    ) -> Option<usize> {
        if params.rerank == 0 || ivf.codes.is_empty() {
            return None;
        }
        let params = &self.classed(params, n_rare, !others.is_empty());
        self.search_fs(ivf, query, others, params, n_rare, top, scratch)
    }

    pub(super) fn search_ivf(
        &self,
        ivf: &LabelIvf,
        query: (&[i8], i32),
        others: &[u32],
        params: &FilterSearchParams,
        top: &mut TopK,
        scratch: &mut IvfScratch,
    ) -> usize {
        let (q, qsum) = query;
        let IvfScratch {
            order,
            batch,
            cursors,
        } = scratch;
        self.rank_lists(ivf, q, qsum, order);
        order.sort_unstable();
        let mut scored = 0usize;
        if batch.capacity() == 0 {
            batch.reserve(4096);
        }
        cursors.resize(others.len(), 0);
        for (probed, &(_, c)) in order.iter().enumerate() {
            if scored >= params.target && probed >= params.min_lists {
                break;
            }
            let (a, b) = (
                ivf.offsets[c as usize] as usize,
                ivf.offsets[c as usize + 1] as usize,
            );
            let members = &ivf.members[a..b];
            cursors.fill(0);
            if self.tuning.block_filter && !others.is_empty() {
                batch.clear();
                self.match_block(
                    members,
                    a as u32,
                    !ivf.rows.is_empty(),
                    others,
                    cursors,
                    batch,
                );
                if ivf.rows.is_empty() {
                    self.score_ids(q, qsum, batch, top);
                } else {
                    self.score_copied(ivf, q, qsum, batch, top);
                }
                scored += batch.len();
            } else if ivf.rows.is_empty() {
                batch.clear();
                batch.extend(
                    members
                        .iter()
                        .copied()
                        .filter(|&id| !self.is_deleted(id) && self.holds(id, others, cursors)),
                );
                self.score_ids(q, qsum, batch, top);
                scored += batch.len();
            } else {
                // Positions (into the copied rows) of the passing members.
                batch.clear();
                batch.extend((a as u32..b as u32).filter(|&pos| {
                    let id = ivf.members[pos as usize];
                    !self.is_deleted(id) && self.holds(id, others, cursors)
                }));
                self.score_copied(ivf, q, qsum, batch, top);
                scored += batch.len();
            }
        }
        scored
    }

    /// [`Self::search`] for `nq` queries in parallel: `queries` row-major `nq * dim`, query
    /// labels as CSR (`label_indptr` with `nq + 1` entries). Writes ids to `out` (`nq * k`,
    /// -1 where fewer than `k` points match).
    pub fn search_batch(
        &self,
        queries: &[u8],
        label_indptr: &[u64],
        label_ids: &[u32],
        k: usize,
        params: &FilterSearchParams,
        out: &mut [i64],
    ) -> Result<(), FilterError> {
        if !queries.len().is_multiple_of(self.dim) {
            return Err(FilterError::Invalid("queries are not whole rows".into()));
        }
        let nq = queries.len() / self.dim;
        if label_indptr.len() != nq + 1
            || label_indptr[0] != 0
            || label_indptr.windows(2).any(|w| w[0] > w[1])
            || label_indptr[nq] != label_ids.len() as u64
        {
            return Err(FilterError::Invalid(
                "query label CSR does not match the queries".into(),
            ));
        }
        let expected = product(nq, k)?;
        if out.len() != expected {
            return Err(FilterError::Invalid(format!(
                "out has {} slots, need {}",
                out.len(),
                expected
            )));
        }
        if k == 0 {
            return Ok(());
        }
        if self.tuning.sort_queries && nq > 1 {
            return self.search_batch_grouped(queries, label_indptr, label_ids, k, params, out);
        }
        out.par_chunks_mut(k)
            .zip(queries.par_chunks(self.dim))
            .enumerate()
            .try_for_each_init(
                || (SearchScratch::default(), Vec::new()),
                |(scratch, found), (i, (row, q))| {
                    let labels = &label_ids[label_indptr[i] as usize..label_indptr[i + 1] as usize];
                    self.search_with_scratch(q, labels, k, params, found, scratch)?;
                    row.fill(-1);
                    for (slot, &(_, id)) in row.iter_mut().zip(found.iter()) {
                        *slot = i64::from(id);
                    }
                    Ok(())
                },
            )
    }
}
