// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! 4-bit FastScan screen for the inverted files of [`FilteredIndex`]: every list keeps its
//! members' product-quantised codes (16 centroids per sub-space, one codebook for the whole
//! index) in 32-code blocks. A query builds one u8 table, scans the blocks of the nearest lists,
//! applies the other labels only to the codes that beat the running cut, and re-ranks the best
//! `rerank` candidates exactly from the base rows.

use std::collections::BinaryHeap;

use super::super::fastscan::{build_lut, pack32, scan32_lt, BLOCK};
use super::*;

/// Product quantiser over the padded `i8` rows: `m` sub-spaces of `dsub` coordinates, 16
/// centroids each.
pub(super) struct Pq {
    m: usize,
    dsub: usize,
    /// `m * 16 * dsub` centroid coordinates.
    centroids: Vec<f32>,
    /// `m * dsub * 16`: coordinate-major copy of `centroids`, so one query coordinate meets the
    /// 16 centroids of its sub-space in a single vector operation.
    lanes: Vec<f32>,
}

impl Pq {
    pub(super) fn train(
        rows: &[i8],
        n: usize,
        stride: usize,
        m: usize,
        sample: usize,
        iters: usize,
        seed: u64,
    ) -> Result<Self, FilterError> {
        if m == 0 || !stride.is_multiple_of(m) || m > 256 || stride / m > 16 {
            return Err(FilterError::Invalid(format!(
                "pq_m {m} must divide the padded dimension {stride} into at most 16 coordinates and be at most 256"
            )));
        }
        let dsub = stride / m;
        let s = sample.min(n);
        if s < 16 {
            return Err(FilterError::Invalid("too few rows to train pq".into()));
        }
        let start = (mix(seed) % n as u64) as usize;
        let step = n as f64 / s as f64;
        let picks: Vec<usize> = (0..s)
            .map(|i| (start + (i as f64 * step) as usize) % n)
            .collect();
        let mut centroids = allocation(product(product(m, 16)?, dsub)?, 0f32)?;
        centroids
            .par_chunks_mut(16 * dsub)
            .enumerate()
            .for_each(|(j, cent)| {
                let x: Vec<f32> = picks
                    .iter()
                    .flat_map(|&p| {
                        rows[p * stride + j * dsub..p * stride + (j + 1) * dsub]
                            .iter()
                            .map(|&v| f32::from(v))
                    })
                    .collect();
                for (v, c) in cent.chunks_exact_mut(dsub).enumerate() {
                    c.copy_from_slice(&x[(v * s / 16) * dsub..(v * s / 16 + 1) * dsub]);
                }
                let mut sums = vec![0f32; 16 * dsub];
                let mut counts = [0u32; 16];
                for _ in 0..iters {
                    sums.fill(0.0);
                    counts.fill(0);
                    for p in x.chunks_exact(dsub) {
                        let v = nearest(cent, p, dsub);
                        counts[v] += 1;
                        for (a, &b) in sums[v * dsub..(v + 1) * dsub].iter_mut().zip(p) {
                            *a += b;
                        }
                    }
                    for v in 0..16 {
                        if counts[v] > 0 {
                            for d in 0..dsub {
                                cent[v * dsub + d] = sums[v * dsub + d] / counts[v] as f32;
                            }
                        }
                    }
                }
            });
        let lanes = Self::lanes(&centroids, m, dsub);
        Ok(Self {
            m,
            dsub,
            centroids,
            lanes,
        })
    }

    /// Code of every row: one nibble per byte, `m` per row.
    pub(super) fn encode_all(
        &self,
        rows: &[i8],
        stride: usize,
        n: usize,
    ) -> Result<Vec<u8>, FilterError> {
        let mut out = allocation(product(n, self.m)?, 0u8)?;
        out.par_chunks_mut(self.m)
            .zip(rows.par_chunks(stride))
            .for_each(|(code, row)| {
                let mut x = [0f32; 16];
                for (j, c) in code.iter_mut().enumerate() {
                    let sub = &row[j * self.dsub..(j + 1) * self.dsub];
                    let xs = &mut x[..self.dsub];
                    for (a, &b) in xs.iter_mut().zip(sub) {
                        *a = f32::from(b);
                    }
                    *c = nearest(&self.centroids[j * 16 * self.dsub..], xs, self.dsub) as u8;
                }
            });
        Ok(out)
    }

    fn lanes(centroids: &[f32], m: usize, dsub: usize) -> Vec<f32> {
        let mut lanes = vec![0f32; centroids.len()];
        for j in 0..m {
            for v in 0..16 {
                for d in 0..dsub {
                    lanes[(j * dsub + d) * 16 + v] = centroids[(j * 16 + v) * dsub + d];
                }
            }
        }
        lanes
    }

    /// Squared distance of each query sub-vector to each centroid, `16 * m` entries.
    fn table(&self, q: &[i8], t: &mut [f32]) {
        for (j, out) in t
            .as_chunks_mut::<16>()
            .0
            .iter_mut()
            .enumerate()
            .take(self.m)
        {
            let mut acc = [0f32; 16];
            for d in 0..self.dsub {
                let x = f32::from(q[j * self.dsub + d]);
                let c = &self.lanes[(j * self.dsub + d) * 16..][..16];
                for v in 0..16 {
                    let diff = x - c[v];
                    acc[v] += diff * diff;
                }
            }
            *out = acc;
        }
    }

    pub(super) fn m(&self) -> usize {
        self.m
    }

    pub(super) fn bytes(&self) -> usize {
        self.centroids.len() * 4
    }
}

#[inline]
fn nearest(cent: &[f32], p: &[f32], dsub: usize) -> usize {
    let mut best = (f32::INFINITY, 0usize);
    for v in 0..16 {
        let d: f32 = cent[v * dsub..(v + 1) * dsub]
            .iter()
            .zip(p)
            .map(|(a, b)| (a - b) * (a - b))
            .sum();
        if d < best.0 {
            best = (d, v);
        }
    }
    best.1
}

/// Block-padded ids and codes of one list set: lists start on block boundaries.
pub(super) struct ListCodes {
    pub codes: Vec<u8>,
    pub ids: Vec<u32>,
    /// `lists + 1` first-block indices.
    pub blocks: Vec<u32>,
}

pub(super) fn pack_lists(
    grouped: &[u32],
    offsets: &[u32],
    point_codes: &[u8],
    m: usize,
) -> Result<ListCodes, FilterError> {
    let lists = offsets.len() - 1;
    let mut blocks = allocation(lists + 1, 0u32)?;
    for c in 0..lists {
        let len = (offsets[c + 1] - offsets[c]) as usize;
        blocks[c + 1] = blocks[c] + len.div_ceil(BLOCK) as u32;
    }
    let total = blocks[lists] as usize;
    let mut ids = allocation(product(total, BLOCK)?, u32::MAX)?;
    let mut codes = allocation(product(total, 16 * m)?, 0u8)?;
    let mut nib = vec![0u8; BLOCK * m];
    for c in 0..lists {
        let members = &grouped[offsets[c] as usize..offsets[c + 1] as usize];
        for (b, chunk) in members.chunks(BLOCK).enumerate() {
            let blk = blocks[c] as usize + b;
            nib.fill(0);
            for (i, &id) in chunk.iter().enumerate() {
                ids[blk * BLOCK + i] = id;
                nib[i * m..(i + 1) * m]
                    .copy_from_slice(&point_codes[id as usize * m..(id as usize + 1) * m]);
            }
            pack32(&nib, m, &mut codes[blk * 16 * m..(blk + 1) * 16 * m]);
        }
    }
    Ok(ListCodes { codes, ids, blocks })
}

#[derive(Default)]
pub(super) struct FsScratch {
    table: Vec<f32>,
    zeros: Vec<f32>,
    ent: Vec<f32>,
    mins: Vec<f32>,
    lut: Vec<u8>,
    order: Vec<(i64, u32)>,
    heap: BinaryHeap<(u32, u32)>,
    cursors: Vec<usize>,
    ids: Vec<u32>,
}

impl FilteredIndex {
    /// The codes of the rarest label's nearest lists screened and re-ranked into `top`.
    /// Returns the rows re-ranked, or `None` when the other labels leave so few matches that
    /// the intersection route is cheaper (nothing was written to `top`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search_fs(
        &self,
        ivf: &LabelIvf,
        query: (&[i8], i32),
        others: &[u32],
        params: &FilterSearchParams,
        n_rare: usize,
        top: &mut TopK,
        s: &mut FsScratch,
    ) -> Option<usize> {
        let pq = self.pq.as_ref()?;
        let (q, qsum) = query;
        let m = pq.m;
        let e = 16 * m;
        s.table.resize(e, 0.0);
        s.zeros.resize(e, 0.0);
        s.ent.resize(e, 0.0);
        s.mins.resize(m, 0.0);
        s.lut.resize(e, 0);
        pq.table(q, &mut s.table);
        build_lut(&s.table, &s.zeros, &mut s.ent, &mut s.mins, &mut s.lut);

        self.rank_lists(ivf, q, qsum, &mut s.order);
        let lists = s.order.len();
        let head = lists.min(64);
        if lists > head {
            s.order.select_nth_unstable(head - 1);
        }
        s.order[..head].sort_unstable();
        let mut sorted_to = head;

        let r = params.rerank.max(1);
        let check_deleted = !(self.tuning.skip_tombstones && self.live == self.n);
        s.heap.clear();
        s.cursors.resize(others.len(), 0);
        let (mut ids_seen, mut matches_seen) = (0usize, 0usize);
        let mut need_ids = usize::MAX;
        let mut sums = [0u32; BLOCK];
        let mut cut = u32::MAX;
        let probe = self.probe(others);
        const CALIBRATE: usize = 1;
        let can_fall_back = !others.is_empty() && n_rare <= params.intersect_max;
        for probed in 0..lists {
            if probed == sorted_to {
                s.order[probed..].sort_unstable();
                sorted_to = lists;
            }
            if probed >= params.min_lists.max(1) && ids_seen >= need_ids {
                break;
            }
            if probed == CALIBRATE && !others.is_empty() {
                let p = matches_seen as f64 / ids_seen.max(1) as f64;
                if can_fall_back && p * n_rare as f64 <= params.brute_max as f64 {
                    return None;
                }
                need_ids = if p <= 0.0 {
                    usize::MAX
                } else {
                    (params.target as f64 / p).min(usize::MAX as f64) as usize
                };
            } else if probed == 1 && others.is_empty() {
                need_ids = params.target;
            }
            let c = s.order[probed].1 as usize;
            let calibrating = probed < CALIBRATE && !others.is_empty();
            s.cursors.fill(0);
            let (b0, b1) = (ivf.blocks[c] as usize, ivf.blocks[c + 1] as usize);
            for blk in b0..b1 {
                let ids = &ivf.bids[blk * BLOCK..(blk + 1) * BLOCK];
                let codes = &ivf.codes[blk * e..(blk + 1) * e];
                let valid = if blk + 1 < b1 {
                    u32::MAX
                } else {
                    ids.iter()
                        .enumerate()
                        .fold(0u32, |mk, (i, &id)| mk | (u32::from(id != u32::MAX) << i))
                };
                ids_seen += valid.count_ones() as usize;
                let mut pass = valid;
                if calibrating {
                    pass = self.pass_mask(ids, valid, &probe, check_deleted, &mut s.cursors);
                    matches_seen += pass.count_ones() as usize;
                    if pass == 0 {
                        continue;
                    }
                }
                let mut mask = scan32_lt(codes, &s.lut, m, cut, &mut sums) & pass;
                if mask == 0 {
                    continue;
                }
                if !calibrating {
                    mask = self.pass_mask(ids, mask, &probe, check_deleted, &mut s.cursors);
                    matches_seen += mask.count_ones() as usize;
                }
                while mask != 0 {
                    let i = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let key = (sums[i], ids[i]);
                    if s.heap.len() < r {
                        s.heap.push(key);
                    } else if key < *s.heap.peek().expect("heap is full") {
                        s.heap.pop();
                        s.heap.push(key);
                    }
                    if s.heap.len() == r {
                        cut = s.heap.peek().expect("heap is full").0.saturating_add(1);
                    }
                }
            }
        }
        s.ids.clear();
        s.ids.extend(s.heap.drain().map(|(_, id)| id));
        s.ids.sort_unstable();
        self.score_ids(q, qsum, &s.ids, top);
        Some(s.ids.len())
    }

    /// The bits of `mask` whose ids are live and hold every label of `others`. Ids ascend within
    /// a list, so the posting cursors only move forward.
    #[inline]
    fn pass_mask(
        &self,
        ids: &[u32],
        mut mask: u32,
        probe: &Probe,
        check_deleted: bool,
        cursors: &mut [usize],
    ) -> u32 {
        if mask == 0 {
            return 0;
        }
        let sets = &probe.sets[..probe.nsets];
        for set in sets {
            let mut m = mask;
            while m != 0 {
                let i = m.trailing_zeros() as usize;
                m &= m - 1;
                prefetch_word(&set[(ids[i] >> 6) as usize]);
            }
        }
        let mut pass = 0u32;
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let id = ids[i];
            if check_deleted && self.is_deleted(id) {
                continue;
            }
            if probe.generic {
                if !self.holds(id, probe.others, cursors) {
                    continue;
                }
            } else {
                if !sets.iter().all(|set| bit(set, id)) {
                    continue;
                }
                if !probe.rest[..probe.nrest].iter().all(|&k| {
                    let k = usize::from(k);
                    self.holds(id, &probe.others[k..=k], &mut cursors[k..=k])
                }) {
                    continue;
                }
            }
            pass |= 1 << i;
        }
        pass
    }
}

/// The other labels of a query, split once into bitsets and posting lists.
struct Probe<'a> {
    others: &'a [u32],
    sets: [&'a [u64]; 4],
    nsets: usize,
    rest: [u8; 4],
    nrest: usize,
    generic: bool,
}

impl FilteredIndex {
    fn probe<'a>(&'a self, others: &'a [u32]) -> Probe<'a> {
        let mut p = Probe {
            others,
            sets: [&[]; 4],
            nsets: 0,
            rest: [0; 4],
            nrest: 0,
            generic: others.len() > 4,
        };
        if !p.generic {
            for (j, &l) in others.iter().enumerate() {
                if let Some(set) = self.bitset(l) {
                    p.sets[p.nsets] = set;
                    p.nsets += 1;
                } else {
                    p.rest[p.nrest] = j as u8;
                    p.nrest += 1;
                }
            }
        }
        p
    }
}
