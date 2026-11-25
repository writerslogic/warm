// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl PartitionTier {
    pub fn new(dim: usize, params: PartitionParams) -> Result<Self, WarmError> {
        if dim == 0 || params.pq_m == 0 || !dim.is_multiple_of(params.pq_m) {
            return Err(invalid(format!(
                "partition tier: pq_m {} must divide dim {dim}",
                params.pq_m
            )));
        }
        if params.lists < 2 || params.spill_k < 2 || params.reorder == 0 || params.nprobe == 0 {
            return Err(invalid(
                "partition tier: lists, spill_k >= 2; reorder, nprobe >= 1".into(),
            ));
        }
        let shape = Shape::new(params.pq_m);
        Ok(Self {
            dim,
            params,
            rows: Vec::new(),
            slot_id: Vec::new(),
            id_slot: HashMap::default(),
            free: Vec::new(),
            trained: None,
            shape,
            profile: std::sync::Mutex::new(SearchProfile::default()),
            trained_live: 0,
            retrain_next: 0,
            inserts_since_train: 0,
            train_phases: [0.0; 3],
        })
    }

    pub fn len(&self) -> usize {
        self.id_slot.len()
    }

    pub fn is_empty(&self) -> bool {
        self.id_slot.is_empty()
    }

    /// Phase timings of the last partition (post-training) search.
    pub fn last_search_profile(&self) -> SearchProfile {
        *self.profile.lock().unwrap()
    }

    /// Changes the number of probed lists for later searches.
    pub fn set_nprobe(&mut self, nprobe: usize) {
        self.params.nprobe = nprobe.max(1);
    }

    /// Candidates re-ranked exactly per query for later searches.
    pub fn set_reorder(&mut self, reorder: usize) {
        self.params.reorder = reorder.max(1);
    }

    /// Sets the adaptive rule and its floor (the cap is `nprobe`, see `set_nprobe`).
    pub fn set_probe_rule(&mut self, rule: ProbeRule, nprobe_min: usize) {
        self.params.probe_rule = rule;
        self.params.nprobe_min = nprobe_min.max(1);
    }

    /// Trained centres, `lists x dim` row-major (None before training).
    pub fn centres(&self) -> Option<&[f32]> {
        self.trained.as_ref().map(|t| t.centres.as_slice())
    }

    /// Primary and spill list of a live id (`u32::MAX` = none); None for an unknown id or before
    /// training. Read-only diagnostic for the miss audit.
    pub fn lists_of(&self, id: u32) -> Option<[u32; 2]> {
        let slot = *self.id_slot.get(&id)?;
        let e = self.trained.as_ref()?.loc.get(slot as usize)?;
        Some([e[0].0, e[1].0])
    }

    pub fn is_trained(&self) -> bool {
        self.trained.is_some()
    }

    pub(super) fn row(&self, slot: u32) -> &[f32] {
        &self.rows[slot as usize * self.dim..(slot as usize + 1) * self.dim]
    }

    /// Inserts rows (`vectors` = `ids.len() x dim`). Re-inserting a live id is an error.
    pub fn insert(&mut self, ids: &[u32], vectors: &[f32]) -> Result<(), WarmError> {
        if vectors.len() != ids.len() * self.dim {
            return Err(invalid(
                "partition tier: vectors must be ids.len() x dim".into(),
            ));
        }
        let mut slots = Vec::with_capacity(ids.len());
        for (i, &id) in ids.iter().enumerate() {
            if self.id_slot.contains_key(&id) {
                return Err(invalid(format!("partition tier: id {id} is already live")));
            }
            let v = &vectors[i * self.dim..(i + 1) * self.dim];
            let slot = if let Some(s) = self.free.pop() {
                self.rows[s as usize * self.dim..(s as usize + 1) * self.dim].copy_from_slice(v);
                self.slot_id[s as usize] = id;
                s
            } else {
                let s =
                    u32::try_from(self.slot_id.len()).map_err(|_| WarmError::CapacityExceeded {
                        limit: "partition tier slots".into(),
                    })?;
                self.rows.extend_from_slice(v);
                self.slot_id.push(id);
                s
            };
            self.id_slot.insert(id, slot);
            slots.push(slot);
        }
        if self.trained.is_some() {
            self.inserts_since_train += ids.len();
            self.place(&slots, vectors);
        }
        Ok(())
    }

    /// Assigns, encodes and appends the given slots (rows in `vectors`, same order).
    pub(super) fn place(&mut self, slots: &[u32], vectors: &[f32]) {
        let (dim, m, sh) = (self.dim, self.params.pq_m, self.shape);
        let t = self.trained.as_mut().expect("trained");
        let asg = t.assign(
            vectors,
            dim,
            self.params.spill_k,
            self.params.soar_lambda,
            self.params.spill_eps,
        );
        let codes: Vec<u8> = {
            let t = &*t;
            asg.par_iter()
                .enumerate()
                .flat_map_iter(|(i, &(a1, a2))| {
                    let x = &vectors[i * dim..(i + 1) * dim];
                    let mut c = vec![0u8; 2 * m];
                    t.encode(x, a1 as usize, dim, m, &mut c[..m]);
                    if a2 != u32::MAX {
                        t.encode(x, a2 as usize, dim, m, &mut c[m..]);
                    }
                    c
                })
                .collect()
        };
        let dists: Vec<[f32; 2]> = {
            let t = &*t;
            asg.par_iter()
                .enumerate()
                .map(|(i, &(a1, a2))| {
                    let x = &vectors[i * dim..(i + 1) * dim];
                    let d = |a: u32| {
                        let a = a as usize;
                        sq(x, &t.centres[a * dim..(a + 1) * dim]).sqrt()
                    };
                    [d(a1), if a2 != u32::MAX { d(a2) } else { 0.0 }]
                })
                .collect()
        };
        if t.loc.len() < self.slot_id.len() {
            t.loc.resize(self.slot_id.len(), [(u32::MAX, u32::MAX); 2]);
        }
        for (i, (&slot, &(a1, a2))) in slots.iter().zip(&asg).enumerate() {
            let p1 = t.lists[a1 as usize].append(sh, &codes[2 * i * m..(2 * i + 1) * m], slot);
            let mut p2 = u32::MAX;
            if a2 != u32::MAX {
                p2 =
                    t.lists[a2 as usize].append(sh, &codes[(2 * i + 1) * m..(2 * i + 2) * m], slot);
            }
            for (a, d) in [a1, a2].into_iter().zip(dists[i]) {
                if a != u32::MAX {
                    t.lists[a as usize].track(d, 1.0);
                }
            }
            t.loc[slot as usize] = [(a1, p1), (a2, p2)];
        }
    }

    /// Deletes ids; unknown ids are ignored. Returns how many were removed.
    pub fn delete(&mut self, ids: &[u32]) -> usize {
        let (sh, dim) = (self.shape, self.dim);
        let mut removed = 0;
        for id in ids {
            let Some(slot) = self.id_slot.remove(id) else {
                continue;
            };
            removed += 1;
            if let Some(t) = self.trained.as_mut() {
                for k in 0..2 {
                    let (list, pos) = t.loc[slot as usize][k];
                    if list == u32::MAX {
                        continue;
                    }
                    let c = list as usize;
                    let x = &self.rows[slot as usize * dim..(slot as usize + 1) * dim];
                    let d = sq(x, &t.centres[c * dim..(c + 1) * dim]).sqrt();
                    t.lists[c].track(d, -1.0);
                    if let Some(moved) = t.lists[list as usize].remove(sh, pos as usize) {
                        // The moved code came from this list's old last position, which is
                        // the list's new length; a slot is in a list at most once.
                        let last = t.lists[list as usize].slots.len() as u32;
                        let e = &mut t.loc[moved as usize];
                        let idx = usize::from(e[0] != (list, last));
                        debug_assert_eq!(e[idx], (list, last));
                        e[idx].1 = pos;
                    }
                }
                t.loc[slot as usize] = [(u32::MAX, u32::MAX); 2];
            }
            self.free.push(slot);
        }
        removed
    }

    /// Trains centres (or uses `centres` when given), the PQ codebook, and places every live row.
    pub fn train(&mut self, centres: Option<Vec<f32>>) -> Result<(), WarmError> {
        let (dim, p) = (self.dim, self.params.clone());
        let live: Vec<u32> = {
            let mut v: Vec<u32> = self.id_slot.values().copied().collect();
            v.sort_unstable();
            v
        };
        if live.len() < p.lists {
            return Err(invalid("partition tier: fewer live rows than lists".into()));
        }
        let mut rng = SplitMix(p.seed);
        let t_start = std::time::Instant::now();
        let gather = |idx: &[usize]| -> Vec<f32> {
            let mut s = Vec::with_capacity(idx.len() * dim);
            for &i in idx {
                s.extend_from_slice(self.row(live[i]));
            }
            s
        };
        let centres = match centres {
            Some(c) if c.len() == p.lists * dim => c,
            Some(_) => {
                return Err(invalid(
                    "partition tier: centres must be lists x dim".into(),
                ))
            }
            None => match self.trained.as_ref() {
                // A retrain continues from the current centres: a few iterations restore
                // balance at a fraction of a cold k-means.
                Some(old) if p.retrain_warm_iters > 0 && old.centres.len() == p.lists * dim => {
                    let sample = gather(&sample_indices(live.len(), p.retrain_sample, &mut rng));
                    kmeans_from(
                        &sample,
                        dim,
                        old.centres.clone(),
                        p.retrain_warm_iters,
                        p.kmeans_rebalance,
                        &mut rng,
                    )
                }
                _ => {
                    let sample = gather(&sample_indices(live.len(), p.train_sample, &mut rng));
                    kmeans_with(
                        &sample,
                        dim,
                        p.lists,
                        p.kmeans_iters,
                        p.kmeans_rebalance,
                        &mut rng,
                    )
                }
            },
        };
        let t_centres = t_start.elapsed().as_secs_f64();
        let l = p.lists;
        let c2: Vec<f32> = centres.chunks(dim).map(|r| dot(r, r)).collect();
        let mut gram = vec![0f32; l * l];
        gram.par_chunks_mut(ROWS * l)
            .zip(centres.par_chunks(ROWS * dim))
            .for_each(|(g, cb)| gemm_nt(cb, cb.len() / dim, &centres, l, dim, g));
        // PQ codebook on residuals to the nearest centre of a sample.
        let m = p.pq_m;
        let sub = dim / m;
        let ps = gather(&sample_indices(live.len(), p.pq_sample, &mut rng));
        let pa = nearest_all(&ps, dim, &centres, &c2);
        let mut resid = ps.clone();
        for (i, &a) in pa.iter().enumerate() {
            for t in 0..dim {
                resid[i * dim + t] -= centres[a as usize * dim + t];
            }
        }
        let n_ps = pa.len();
        let mut codebook = vec![0f32; m * 16 * sub];
        for b in 0..m {
            let mut x = Vec::with_capacity(n_ps * sub);
            for i in 0..n_ps {
                x.extend_from_slice(&resid[i * dim + b * sub..i * dim + (b + 1) * sub]);
            }
            let cb = kmeans(&x, sub, 16, p.pq_iters, &mut rng);
            codebook[b * 16 * sub..(b + 1) * 16 * sub].copy_from_slice(&cb);
        }
        let mut list_tab = vec![0f32; l * 16 * m];
        list_tab
            .par_chunks_mut(16 * m)
            .enumerate()
            .for_each(|(li, tab)| {
                let c = &centres[li * dim..(li + 1) * dim];
                for b in 0..m {
                    for v in 0..16 {
                        let cb = &codebook[(b * 16 + v) * sub..(b * 16 + v + 1) * sub];
                        tab[b * 16 + v] = dot(cb, cb) + 2.0 * dot(&c[b * sub..(b + 1) * sub], cb);
                    }
                }
            });
        let t_tables = t_start.elapsed().as_secs_f64();
        self.trained = Some(Trained {
            centres,
            c2,
            gram,
            codebook,
            list_tab,
            lists: (0..l).map(|_| List::default()).collect(),
            loc: vec![[(u32::MAX, u32::MAX); 2]; self.slot_id.len()],
        });
        self.trained_live = live.len();
        self.inserts_since_train = 0;
        // Placed in chunks so a retrain at 10M rows does not hold a second copy of the rows.
        for chunk in live.chunks(PLACE_CHUNK) {
            let mut vectors = Vec::with_capacity(chunk.len() * dim);
            for &slot in chunk {
                vectors.extend_from_slice(self.row(slot));
            }
            self.place(chunk, &vectors);
        }
        let t_end = t_start.elapsed().as_secs_f64();
        self.train_phases = [t_centres, t_tables - t_centres, t_end - t_tables];
        Ok(())
    }

    /// Retrains in place; `WARM_LOG_RETRAIN` prints one line per retrain to stderr.
    pub(super) fn retrain(&mut self, why: &str) -> Result<(), WarmError> {
        let (t0, imb, live) = (std::time::Instant::now(), self.list_imbalance(), self.len());
        self.train(None)?;
        if crate::compatibility::var("WARM_LOG_RETRAIN").is_ok() {
            eprintln!(
                "[warm] retrain ({why}) live={live} imbalance {imb:.3} -> {:.3} in {:.1}s \
                 (centres {:.1}s, tables {:.1}s, place {:.1}s)",
                self.list_imbalance(),
                t0.elapsed().as_secs_f64(),
                self.train_phases[0],
                self.train_phases[1],
                self.train_phases[2]
            );
        }
        Ok(())
    }

    /// `nprobe` scaled by `nprobe_by_live` at the current live size.
    pub(super) fn effective_nprobe(&self) -> usize {
        let pts = &self.params.nprobe_by_live;
        let (n, base) = (self.len(), self.params.nprobe);
        let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
            return base;
        };
        let f = if n <= first.0 {
            first.1
        } else if n >= last.0 {
            last.1
        } else {
            let i = pts.partition_point(|p| p.0 <= n) - 1;
            let ((x0, f0), (x1, f1)) = (pts[i], pts[i + 1]);
            f0 + (f1 - f0) * (n - x0) as f32 / (x1 - x0) as f32
        };
        ((base as f32 * f).round() as usize).clamp(1, self.params.lists)
    }

    /// `lists * sum(len^2) / sum(len)^2` over the list lengths (1 = perfectly even).
    pub(super) fn list_imbalance(&self) -> f64 {
        let Some(t) = self.trained.as_ref() else {
            return 1.0;
        };
        let (mut sum, mut sq) = (0f64, 0f64);
        for l in &t.lists {
            let n = l.slots.len() as f64;
            sum += n;
            sq += n * n;
        }
        if sum == 0.0 {
            1.0
        } else {
            sq * t.lists.len() as f64 / (sum * sum)
        }
    }

    /// Top-`k` ids per query (`queries` = `nq x dim`), nearest first.
    pub fn search(&mut self, queries: &[f32], k: usize) -> Result<Vec<Vec<u32>>, WarmError> {
        if !queries.len().is_multiple_of(self.dim) || k == 0 {
            return Err(invalid(
                "partition tier: queries must be nq x dim, k >= 1".into(),
            ));
        }
        if self.trained.is_none() && self.len() >= self.params.train_at.max(self.params.lists) {
            self.train(None)?;
        } else if self.trained.is_some() {
            while self
                .params
                .retrain_at
                .get(self.retrain_next)
                .is_some_and(|&at| at <= self.trained_live)
            {
                self.retrain_next += 1;
            }
            if self
                .params
                .retrain_at
                .get(self.retrain_next)
                .is_some_and(|&at| self.len() >= at)
            {
                self.retrain_next += 1;
                self.retrain("schedule")?;
            } else if self.params.retrain_imb > 0.0
                && self.inserts_since_train >= self.params.retrain_min_inserts
                && self.list_imbalance() > f64::from(self.params.retrain_imb)
            {
                self.retrain("imbalance")?;
            }
        }
        Ok(if self.trained.is_some() {
            let base = self.params.nprobe;
            self.params.nprobe = self.effective_nprobe();
            let out = self.search_partition(queries, k);
            self.params.nprobe = base;
            out
        } else {
            self.search_exact(queries, k)
        })
    }

    /// Exact brute force over live rows: query blocks in parallel, row blocks through one
    /// product each, a running k-th-best filter per query.
    pub fn search_exact(&self, queries: &[f32], k: usize) -> Vec<Vec<u32>> {
        let dim = self.dim;
        let live: Vec<u32> = {
            let mut v: Vec<u32> = self.id_slot.values().copied().collect();
            v.sort_unstable();
            v
        };
        let mut x = Vec::with_capacity(live.len() * dim);
        for &s in &live {
            x.extend_from_slice(self.row(s));
        }
        let x2: Vec<f32> = x.chunks(dim).map(|r| dot(r, r)).collect();
        let nq = queries.len() / dim;
        let mut out: Vec<Vec<u32>> = vec![Vec::new(); nq];
        out.par_chunks_mut(BF_QUERIES)
            .zip(queries.par_chunks(BF_QUERIES * dim))
            .for_each(|(o, q)| {
                let nb = o.len();
                let mut best: Vec<Vec<(f32, u32)>> = vec![Vec::with_capacity(k + 1); nb];
                let mut prod = vec![0f32; nb * BF_ROWS];
                for (r0, xr) in x.chunks(BF_ROWS * dim).enumerate() {
                    let rn = xr.len() / dim;
                    gemm_nt(q, nb, xr, rn, dim, &mut prod);
                    for (qi, bq) in best.iter_mut().enumerate() {
                        let mut worst = if bq.len() == k {
                            bq[k - 1].0
                        } else {
                            f32::INFINITY
                        };
                        for j in 0..rn {
                            let d = x2[r0 * BF_ROWS + j] - 2.0 * prod[qi * rn + j];
                            if d < worst {
                                let pos = bq.partition_point(|e| e.0 <= d);
                                bq.insert(pos, (d, live[r0 * BF_ROWS + j]));
                                bq.truncate(k);
                                if bq.len() == k {
                                    worst = bq[k - 1].0;
                                }
                            }
                        }
                    }
                }
                for (oi, bq) in o.iter_mut().zip(best) {
                    *oi = bq
                        .into_iter()
                        .map(|(_, s)| self.slot_id[s as usize])
                        .collect();
                }
            });
        out
    }

    pub(super) fn search_partition(&self, queries: &[f32], k: usize) -> Vec<Vec<u32>> {
        let t = self.trained.as_ref().expect("trained");
        let (dim, m) = (self.dim, self.params.pq_m);
        let (l, e, sub) = (t.lists_len(), 16 * m, dim / m);
        let sh = self.shape;
        let lstride = sh.stride();
        let nq = queries.len() / dim;
        let nprobe = self.params.nprobe.min(l);
        let (rule, nmin) = (self.params.probe_rule, self.params.nprobe_min);
        let radii: Vec<f32> = match rule {
            ProbeRule::Radius { .. } => t.lists.iter().map(List::radius).collect(),
            _ => Vec::new(),
        };
        let reorder = self.params.reorder.max(k);
        let t0 = std::time::Instant::now();
        let mut qc = vec![0f32; nq * l];
        qc.par_chunks_mut(ROWS * l)
            .zip(queries.par_chunks(ROWS * dim))
            .for_each(|(o, q)| gemm_nt(q, q.len() / dim, &t.centres, l, dim, o));
        let probe: Vec<Vec<u32>> = qc
            .par_chunks_mut(l)
            .zip(queries.par_chunks(dim))
            .map(|(row, q)| {
                let q2 = dot(q, q);
                for (v, &cc) in row.iter_mut().zip(&t.c2) {
                    *v = (q2 - 2.0 * *v + cc).max(0.0);
                }
                if rule != ProbeRule::Fixed {
                    return select_probes(row, &radii, rule, nmin, nprobe);
                }
                // Distances are non-negative (`+ 0.0` turns -0.0 into 0.0), so their bits order
                // like the floats; packing the list index beside them selects on integers.
                let mut keys: Vec<u64> = row
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (u64::from((v + 0.0).to_bits()) << 32) | i as u64)
                    .collect();
                keys.select_nth_unstable(nprobe - 1);
                keys.truncate(nprobe);
                keys.into_iter().map(|k| k as u32).collect()
            })
            .collect();
        let probes: u64 = probe.iter().map(|p| p.len() as u64).sum();
        let centres_s = t0.elapsed().as_secs_f64();
        // Per list, the probing queries with their centre distance, in query order. Built in
        // parallel over ranges of lists, each pass reading every probe.
        let span = l.div_ceil(2 * rayon::current_num_threads().max(1)).max(1);
        let by_list: Vec<Vec<(u32, f32)>> = (0..l)
            .step_by(span)
            .collect::<Vec<_>>()
            .into_par_iter()
            .flat_map_iter(|lo| {
                let hi = (lo + span).min(l);
                let mut part: Vec<Vec<(u32, f32)>> = vec![Vec::new(); hi - lo];
                for (q, ls) in probe.iter().enumerate() {
                    for &li in ls {
                        let li = li as usize;
                        if (lo..hi).contains(&li) {
                            part[li - lo].push((q as u32, qc[q * l + li]));
                        }
                    }
                }
                part
            })
            .collect();
        drop(qc);
        let qtab: Vec<f32> = queries
            .par_chunks(dim)
            .flat_map_iter(|q| {
                (0..e).map(move |i| {
                    let (b, v) = (i / 16, i % 16);
                    let cb = &t.codebook[(b * 16 + v) * sub..(b * 16 + v + 1) * sub];
                    -2.0 * dot(&q[b * sub..(b + 1) * sub], cb)
                })
            })
            .collect();
        let route_s = t0.elapsed().as_secs_f64() - centres_s;
        let threads = rayon::current_num_threads().max(1);
        let (owner, local_at, members) = balance_queries(&probe, &t.lists, threads);
        let parts: Vec<ThreadPart> = (0..threads)
            .into_par_iter()
            .map(|th| {
                let mine: &[u32] = &members[th];
                let local = |q: u32| local_at[q as usize] as usize;
                let mut bufs: Vec<Vec<u64>> =
                    vec![Vec::with_capacity(2 * reorder + BLOCK); mine.len()];
                let mut thr = vec![f32::INFINITY; mine.len()];
                let (mut ent, mut mins, mut lut) = (vec![0f32; e], vec![0f32; m], vec![0u8; e]);
                let mut luts = vec![0u8; if sh.quad { QB * lstride } else { 0 }];
                let mut outs = [[0u32; BLOCK]; QB];
                let mut own: Vec<(u32, f32)> = Vec::new();
                let mut out = [0u32; BLOCK];
                let (mut lut_s, mut blocks, mut admitted) = (0f64, 0u64, 0u64);
                let ts = std::time::Instant::now();
                for (li, qs) in by_list.iter().enumerate() {
                    let list = &t.lists[li];
                    let n = list.slots.len();
                    if n == 0 {
                        continue;
                    }
                    let lt = &t.list_tab[li * e..(li + 1) * e];
                    if sh.quad {
                        own.clear();
                        own.extend(
                            qs.iter()
                                .copied()
                                .filter(|(q, _)| owner[*q as usize] as usize == th),
                        );
                        for (ci, chunk) in own.chunks(QB).enumerate() {
                            // The next chunk's query tables are read cold; request them now.
                            for &(q, _) in own.iter().skip((ci + 1) * QB).take(QB) {
                                exact_l2::prefetch_l2(&qtab[q as usize * e..(q as usize + 1) * e]);
                            }
                            let kq = chunk.len();
                            let (mut js, mut bases) = ([0usize; QB], [0f32; QB]);
                            let (mut scales, mut cuts) = ([0f32; QB], [0u32; QB]);
                            let tl = std::time::Instant::now();
                            for (k, &(q, dc)) in chunk.iter().enumerate() {
                                let j = local(q);
                                let (base, scale) = build_lut(
                                    &qtab[q as usize * e..(q as usize + 1) * e],
                                    lt,
                                    &mut ent,
                                    &mut mins,
                                    &mut luts[k * lstride..k * lstride + e],
                                );
                                let base = base + dc;
                                (js[k], bases[k], scales[k]) = (j, base, scale);
                                cuts[k] = admit_cut(thr[j], base, scale);
                            }
                            lut_s += tl.elapsed().as_secs_f64();
                            // Members whose threshold fell to this list's base are finished with
                            // it (a threshold only falls), so only the live ones go through the
                            // kernel; the set changes only when a threshold does.
                            let (mut sel, mut cs, mut live) = ([0usize; QB], [0u32; QB], 0);
                            for k in 0..kq {
                                if thr[js[k]] > bases[k] {
                                    (sel[live], cs[live]) = (k, cuts[k]);
                                    live += 1;
                                }
                            }
                            for blk in 0..n.div_ceil(BLOCK) {
                                if live == 0 {
                                    break;
                                }
                                blocks += live as u64;
                                let masks = scan_group(
                                    live,
                                    &list.codes[blk * lstride..(blk + 1) * lstride],
                                    &luts,
                                    lstride,
                                    m,
                                    &sel,
                                    &cs,
                                    &mut outs,
                                );
                                let valid = (n - blk * BLOCK).min(BLOCK);
                                let mut retired = false;
                                let any = masks[..live].iter().fold(0, |a, &m| a | m);
                                for i in 0..if any == 0 { 0 } else { live } {
                                    let mut mask = masks[i];
                                    if mask == 0 {
                                        continue;
                                    }
                                    if valid < BLOCK {
                                        mask &= (1u32 << valid) - 1;
                                    }
                                    let k = sel[i];
                                    let (j, base, scale) = (js[k], bases[k], scales[k]);
                                    let buf = &mut bufs[j];
                                    while mask != 0 {
                                        let c = mask.trailing_zeros() as usize;
                                        mask &= mask - 1;
                                        admitted += 1;
                                        buf.push(score_key(
                                            base + outs[i][c] as f32 * scale,
                                            list.slots[blk * BLOCK + c],
                                        ));
                                    }
                                    if buf.len() >= 2 * reorder {
                                        buf.select_nth_unstable(reorder - 1);
                                        buf.truncate(reorder);
                                        // The selected element is the largest kept score.
                                        thr[j] = key_score(buf[reorder - 1]);
                                        cs[i] = admit_cut(thr[j], base, scale);
                                        if thr[j] <= base {
                                            sel[i] = usize::MAX;
                                            retired = true;
                                        }
                                    }
                                }
                                if retired {
                                    let mut w = 0;
                                    for i in 0..live {
                                        if sel[i] != usize::MAX {
                                            (sel[w], cs[w]) = (sel[i], cs[i]);
                                            w += 1;
                                        }
                                    }
                                    live = w;
                                }
                            }
                        }
                        continue;
                    }
                    for &(q, dc) in qs.iter().filter(|(q, _)| owner[*q as usize] as usize == th) {
                        let j = local(q);
                        let tl = std::time::Instant::now();
                        let (base, scale) = build_lut(
                            &qtab[q as usize * e..(q as usize + 1) * e],
                            lt,
                            &mut ent,
                            &mut mins,
                            &mut lut,
                        );
                        lut_s += tl.elapsed().as_secs_f64();
                        let base = base + dc;
                        let buf = &mut bufs[j];
                        let mut cut = admit_cut(thr[j], base, scale);
                        for blk in 0..n.div_ceil(BLOCK) {
                            if thr[j] <= base {
                                break;
                            }
                            let mut mask = scan32_lt(
                                &list.codes[blk * lstride..(blk + 1) * lstride],
                                &lut,
                                m,
                                cut,
                                &mut out,
                            );
                            blocks += 1;
                            let valid = (n - blk * BLOCK).min(BLOCK);
                            if valid < BLOCK {
                                mask &= (1u32 << valid) - 1;
                            }
                            while mask != 0 {
                                let c = mask.trailing_zeros() as usize;
                                mask &= mask - 1;
                                admitted += 1;
                                buf.push(score_key(
                                    base + out[c] as f32 * scale,
                                    list.slots[blk * BLOCK + c],
                                ));
                            }
                            if buf.len() >= 2 * reorder {
                                buf.select_nth_unstable(reorder - 1);
                                buf.truncate(reorder);
                                thr[j] = key_score(buf[reorder - 1]);
                                cut = admit_cut(thr[j], base, scale);
                            }
                        }
                    }
                }
                let scan_s = ts.elapsed().as_secs_f64();
                let mut seen = SlotSet::new(reorder);
                let mut uniq: Vec<u32> = Vec::with_capacity(reorder);
                let mut dist = vec![0f32; reorder + exact_l2::LANES];
                let res: Vec<(u32, Vec<u32>)> = mine
                    .iter()
                    .zip(bufs)
                    .map(|(&q, mut buf)| {
                        buf.sort_unstable();
                        let qv = &queries[q as usize * dim..(q as usize + 1) * dim];
                        // The best `reorder` distinct slots in score order.
                        seen.clear();
                        uniq.clear();
                        for key in buf {
                            if uniq.len() == reorder {
                                break;
                            }
                            let s = key as u32;
                            if seen.insert(s) {
                                uniq.push(s);
                            }
                        }
                        // Exact distances in batches, the rows of the next batch requested
                        // while this one is computed.
                        for (b, slots) in uniq.chunks(exact_l2::LANES).enumerate() {
                            for &s in uniq
                                .iter()
                                .skip((b + 1) * exact_l2::LANES)
                                .take(exact_l2::LANES)
                            {
                                exact_l2::prefetch_row(&self.rows, dim, s);
                            }
                            exact_l2::sq_batch(
                                qv,
                                &self.rows,
                                dim,
                                slots,
                                &mut dist[b * exact_l2::LANES..],
                            );
                        }
                        let mut exact: Vec<(f32, u32)> =
                            dist.iter().copied().zip(uniq.iter().copied()).collect();
                        exact.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                        (
                            q,
                            exact
                                .into_iter()
                                .take(k)
                                .map(|(_, s)| self.slot_id[s as usize])
                                .collect(),
                        )
                    })
                    .collect();
                (
                    res,
                    scan_s,
                    ts.elapsed().as_secs_f64() - scan_s,
                    lut_s,
                    blocks,
                    admitted,
                )
            })
            .collect();
        let mut res = vec![Vec::new(); nq];
        let (mut scan_s, mut reorder_s, mut scan_sum, mut lut_sum) = (0f64, 0f64, 0f64, 0f64);
        let (mut blocks, mut admitted) = (0u64, 0u64);
        let nparts = parts.len().max(1) as f64;
        for (part, sc, ro, lu, bl, ad) in parts {
            scan_s = scan_s.max(sc);
            reorder_s = reorder_s.max(ro);
            scan_sum += sc;
            lut_sum += lu;
            blocks += bl;
            admitted += ad;
            for (q, ids) in part {
                res[q as usize] = ids;
            }
        }
        *self.profile.lock().unwrap() = SearchProfile {
            centres_s,
            route_s,
            scan_s,
            scan_mean_s: scan_sum / nparts,
            lut_mean_s: lut_sum / nparts,
            probes,
            blocks,
            admitted,
            reorder_s,
            total_s: t0.elapsed().as_secs_f64(),
        };
        res
    }
}
