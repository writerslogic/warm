// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Experimental partition tier for streaming L2 search (the L22 design): an IVF of `lists`
//! k-means centres trained once, every row in its nearest list plus one SOAR spill list
//! (Sun et al., 2023), residual PQ codes with 16 centroids per sub-space stored in the
//! 32-code LUT16 blocks of the `fastscan` kernel, an INT8 table per (query, list), and an exact
//! reorder of the best candidates from the resident f32 rows.
//!
//! Before training the tier answers by exact brute force. Training happens at the first search
//! with at least `train_at` live rows; centres are then fixed (no list maintenance).
//!
//! Insert assignment uses one rows x centres product per block: the `spill_k` nearest centres
//! by L2 give the exact nearest list, and the spill list is the SOAR minimiser among the other
//! `spill_k - 1`, computed from `x.c` and the centre Gram matrix (no second product).

use std::collections::HashMap;

use rayon::prelude::*;

use super::exact_l2;
use super::fastscan::{
    build_lut, quad_available, quad_pos, quad_stride, scan32_lt, scan32q_lt, BLOCK,
};
use super::invalid;
use crate::core::error::WarmError;

/// How many lists a query probes, decided at routing time from its sorted centre distances.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ProbeRule {
    /// The `nprobe` nearest lists.
    #[default]
    Fixed,
    /// List `i` while `d_i^2 <= d_1^2 (1 + eps)`.
    Ratio { eps: f32 },
    /// List `i` while `max(0, d_i - scale * r_i) <= (1 + eps) d_1`, with `r_i` the list's member
    /// radius (mean + 2 sd of member-to-centre distance). Not monotone in `i`: a far list with a
    /// large radius is kept past a near list with a small one.
    Radius { eps: f32, scale: f32 },
}

/// Tier parameters. Defaults are the stage-1 operating point.
#[derive(Clone, Debug)]
pub struct PartitionParams {
    pub lists: usize,
    /// Train at the first search with at least this many live rows.
    pub train_at: usize,
    pub train_sample: usize,
    pub kmeans_iters: usize,
    /// PQ sub-spaces (must divide the dimension), 16 centroids each.
    pub pq_m: usize,
    pub pq_sample: usize,
    pub pq_iters: usize,
    /// Candidate centres for the spill choice (nearest `spill_k` by L2).
    pub spill_k: usize,
    pub soar_lambda: f32,
    /// Lists probed by `ProbeRule::Fixed`; the cap for the adaptive rules.
    pub nprobe: usize,
    /// Floor on the lists an adaptive rule probes.
    pub nprobe_min: usize,
    pub probe_rule: ProbeRule,
    pub reorder: usize,
    pub seed: u64,
    /// Ascending live-row counts. After the first training, the first search that finds at
    /// least this many live rows retrains the centres and codebook and re-places every row;
    /// each count fires once. Counts at or below the live size of the previous training are
    /// skipped.
    pub retrain_at: Vec<usize>,
    /// Retrain at a search when the list imbalance (`lists * sum(len^2) / sum(len)^2`) exceeds
    /// this (0 disables) and at least `retrain_min_inserts` rows were inserted since the last
    /// training.
    pub retrain_imb: f32,
    pub retrain_min_inserts: usize,
    /// Lloyd iterations of a retrain warm-started from the current centres on a sample of
    /// `retrain_sample` live rows (0 = a cold k-means as at the first training).
    pub retrain_warm_iters: usize,
    pub retrain_sample: usize,
    /// Coarse routing of row placement: a row is scored against the `route_cands` centres
    /// nearest its nearest of `route_lists` super-centres instead of against all of them
    /// (0 = all, exact). Applies to inserts and to the re-placement in a retrain.
    pub route_cands: usize,
    pub route_lists: usize,
    /// Reseed near-empty centres onto the largest clusters between Lloyd iterations (in the
    /// first training and in retrains), which restores balance after the data has moved.
    pub kmeans_rebalance: bool,
    /// `(live rows, factor)` points, ascending in rows, scaling `nprobe` by the live size at
    /// each search (piecewise linear, flat outside the points). Empty = a constant `nprobe`.
    /// Lists shrink with the live set, so a small set can afford more probes per query.
    pub nprobe_by_live: Vec<(usize, f32)>,
    /// A row also joins its SOAR spill list only when that list's centre is within
    /// `(1 + spill_eps)` times the squared distance to the primary centre; infinity spills
    /// every row.
    pub spill_eps: f32,
}

impl Default for PartitionParams {
    fn default() -> Self {
        Self {
            lists: 5000,
            train_at: 1_000_000,
            train_sample: 262_144,
            kmeans_iters: 10,
            pq_m: 50,
            pq_sample: 32_000,
            pq_iters: 10,
            spill_k: 32,
            soar_lambda: 1.0,
            nprobe: 350,
            nprobe_min: 1,
            probe_rule: ProbeRule::Fixed,
            reorder: 317,
            seed: 7,
            retrain_at: Vec::new(),
            retrain_imb: 0.0,
            retrain_min_inserts: 1_000_000,
            retrain_warm_iters: 0,
            retrain_sample: 262_144,
            route_cands: 0,
            route_lists: 64,
            kmeans_rebalance: false,
            nprobe_by_live: Vec::new(),
            spill_eps: f32::INFINITY,
        }
    }
}

/// Rows per assignment / brute-force block (the block's product is `ROWS x lists` floats).
const ROWS: usize = 1024;
const BF_QUERIES: usize = 256;
const BF_ROWS: usize = 8192;
/// Rows placed per step when a whole tier is (re)placed.
const PLACE_CHUNK: usize = 1 << 18;

#[derive(Default)]
struct List {
    /// `blocks * 16 * m` bytes in the LUT16 block layout.
    codes: Vec<u8>,
    /// Slot of each code, in position order.
    slots: Vec<u32>,
    /// Sum and sum of squares of member-to-centre distance over `slots` (spilled members included).
    dist_sum: f64,
    dist_sq: f64,
}

struct Trained {
    centres: Vec<f32>,
    c2: Vec<f32>,
    /// Centre Gram matrix `C C^T`, row-major `lists x lists`.
    gram: Vec<f32>,
    /// `m x 16 x sub` codebook.
    codebook: Vec<f32>,
    /// Per-list table term `|v|^2 + 2 <c_b, v>`, `lists x 16m`.
    list_tab: Vec<f32>,
    lists: Vec<List>,
    /// Per slot: (list, position) of the primary and spill memberships; `u32::MAX` = none.
    loc: Vec<[(u32, u32); 2]>,
    coarse: Option<Coarse>,
}

/// Super-centres over the list centres and, for each, the nearest list centres.
struct Coarse {
    supers: Vec<f32>,
    s2: Vec<f32>,
    cands: Vec<u32>,
    r: usize,
}

/// Wall seconds of the phases of the last partition search (max over threads for the
/// per-thread phases).
#[derive(Clone, Copy, Debug, Default)]
pub struct SearchProfile {
    pub centres_s: f64,
    pub route_s: f64,
    /// Slowest thread's scan phase (table builds included).
    pub scan_s: f64,
    /// Mean over threads of the scan phase, to expose imbalance against `scan_s`.
    pub scan_mean_s: f64,
    /// Mean over threads of the time spent building per-(query, list) tables.
    pub lut_mean_s: f64,
    /// Lists probed, summed over queries.
    pub probes: u64,
    /// 32-code blocks scanned, summed over threads.
    pub blocks: u64,
    /// Candidates admitted below the running threshold, summed over threads.
    pub admitted: u64,
    pub reorder_s: f64,
    pub total_s: f64,
}

/// One search thread's (query, ids) results with its scan, reorder and
/// table-build seconds and its scanned block count.
type ThreadPart = (Vec<(u32, Vec<u32>)>, f64, f64, f64, u64, u64);

/// Streaming partition tier over external `u32` ids.
pub struct PartitionTier {
    dim: usize,
    params: PartitionParams,
    rows: Vec<f32>,
    slot_id: Vec<u32>,
    id_slot: HashMap<u32, u32, foldhash::fast::RandomState>,
    free: Vec<u32>,
    trained: Option<Trained>,
    shape: Shape,
    profile: std::sync::Mutex<SearchProfile>,
    /// Live rows at the last training, and the next unfired `retrain_at` entry.
    trained_live: usize,
    retrain_next: usize,
    inserts_since_train: usize,
    /// Seconds of the last training: centres, codebook and tables, placement.
    train_phases: [f64; 3],
}

/// Lists to probe for one query under an adaptive rule. `d2` is the query's squared distance to
/// every centre and `radii` the per-list radii (read only by `Radius`). Candidates are the `nmax`
/// nearest lists; the `nmin` nearest are always kept.
fn select_probes(d2: &[f32], radii: &[f32], rule: ProbeRule, nmin: usize, nmax: usize) -> Vec<u32> {
    let nmax = nmax.clamp(1, d2.len());
    let nmin = nmin.clamp(1, nmax);
    let mut idx: Vec<u32> = (0..d2.len() as u32).collect();
    idx.select_nth_unstable_by(nmax - 1, |a, b| d2[*a as usize].total_cmp(&d2[*b as usize]));
    idx.truncate(nmax);
    idx.sort_unstable_by(|a, b| d2[*a as usize].total_cmp(&d2[*b as usize]));
    let d1sq = d2[idx[0] as usize];
    match rule {
        ProbeRule::Fixed => {}
        ProbeRule::Ratio { eps } => {
            let cut = d1sq * (1.0 + eps);
            let keep = idx.partition_point(|&i| d2[i as usize] <= cut);
            idx.truncate(keep.max(nmin));
        }
        ProbeRule::Radius { eps, scale } => {
            let cut = (1.0 + eps) * d1sq.sqrt();
            let mut pos = 0;
            idx.retain(|&i| {
                pos += 1;
                pos <= nmin || (d2[i as usize].sqrt() - scale * radii[i as usize]).max(0.0) <= cut
            });
        }
    }
    idx
}

/// Per-(query, list) table build cost in scanned-code equivalents (measured on wl-03: ~2.7 us
/// per table against ~4.9e8 codes/s per thread).
const TABLE_CODES: u64 = 1300;

/// Assigns queries to `threads` workers by estimated scan work (one table plus the codes of each
/// probed list), longest first onto the least-loaded worker. Returns each query's worker, its
/// index within that worker, and each worker's queries in that index order.
fn balance_queries(
    probe: &[Vec<u32>],
    lists: &[List],
    threads: usize,
) -> (Vec<u32>, Vec<u32>, Vec<Vec<u32>>) {
    let cost = |q: usize| -> u64 {
        probe[q]
            .iter()
            .map(|&li| lists[li as usize].slots.len() as u64 + TABLE_CODES)
            .sum()
    };
    let mut order: Vec<(u64, u32)> = (0..probe.len()).map(|q| (cost(q), q as u32)).collect();
    order.sort_unstable_by(|a, b| b.cmp(a));
    let mut load = vec![0u64; threads];
    let (mut owner, mut local) = (vec![0u32; probe.len()], vec![0u32; probe.len()]);
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); threads];
    for (c, q) in order {
        let th = (0..threads).min_by_key(|&i| (load[i], i)).unwrap_or(0);
        load[th] += c;
        owner[q as usize] = th as u32;
        local[q as usize] = members[th].len() as u32;
        members[th].push(q);
    }
    (owner, local, members)
}

/// The u32 sums a scan admits under the running threshold `thr`: a code with sum `v` is kept when
/// `v < ((thr - base) / scale).max(0)`, which for integer `v` is `v < ceil(..)`.
#[inline]
fn admit_cut(thr: f32, base: f32, scale: f32) -> u32 {
    if thr.is_finite() {
        ((thr - base) / scale).max(0.0).ceil() as u32
    } else {
        u32::MAX
    }
}

/// Scores one quad-layout block against `live` of the [`QB`] tables, table `i` being
/// `luts[sel[i] * lstride..]`; returns the admission masks by `i`.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn scan_group(
    live: usize,
    codes: &[u8],
    luts: &[u8],
    lstride: usize,
    m: usize,
    sel: &[usize; QB],
    cuts: &[u32; QB],
    outs: &mut [[u32; BLOCK]; QB],
) -> [u32; QB] {
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn run<const K: usize>(
        codes: &[u8],
        luts: &[u8],
        lstride: usize,
        m: usize,
        sel: &[usize; QB],
        cuts: &[u32; QB],
        outs: &mut [[u32; BLOCK]; QB],
    ) -> [u32; QB] {
        let mut res = [0u32; QB];
        let masks = scan32q_lt::<K>(
            codes,
            luts,
            lstride,
            m,
            sel[..K].try_into().expect("K <= QB"),
            cuts[..K].try_into().expect("K <= QB"),
            (&mut outs[..K]).try_into().expect("K <= QB"),
        );
        res[..K].copy_from_slice(&masks);
        res
    }
    match live {
        1 => run::<1>(codes, luts, lstride, m, sel, cuts, outs),
        2 => run::<2>(codes, luts, lstride, m, sel, cuts, outs),
        3 => run::<3>(codes, luts, lstride, m, sel, cuts, outs),
        4 => run::<4>(codes, luts, lstride, m, sel, cuts, outs),
        5 => run::<5>(codes, luts, lstride, m, sel, cuts, outs),
        6 => run::<6>(codes, luts, lstride, m, sel, cuts, outs),
        7 => run::<7>(codes, luts, lstride, m, sel, cuts, outs),
        _ => run::<8>(codes, luts, lstride, m, sel, cuts, outs),
    }
}

/// `c[m x n] = a[m x k] . b[n x k]^T`, single-threaded.
fn gemm_nt(a: &[f32], m: usize, b: &[f32], n: usize, k: usize, c: &mut [f32]) {
    assert!(a.len() >= m * k && b.len() >= n * k && c.len() >= m * n);
    if m == 0 || n == 0 {
        return;
    }
    // SAFETY: the asserts bound every access: A is m x k row-major (rsa = k, csa = 1), B^T is
    // read from b as k x n with rsb = 1, csb = k, C is m x n row-major.
    unsafe {
        matrixmultiply::sgemm(
            m,
            k,
            n,
            1.0,
            a.as_ptr(),
            k as isize,
            1,
            b.as_ptr(),
            1,
            k as isize,
            0.0,
            c.as_mut_ptr(),
            n as isize,
            1,
        );
    }
}

/// The `k` smallest `(value, index)` of `vals` (unordered), by a running worst-of-k filter.
fn smallest_k(vals: &[f32], k: usize, out: &mut Vec<(f32, u32)>) {
    out.clear();
    let k = k.min(vals.len());
    if k == 0 {
        return;
    }
    out.extend(vals[..k].iter().enumerate().map(|(i, &v)| (v, i as u32)));
    let worst = |o: &[(f32, u32)]| {
        o.iter()
            .enumerate()
            .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .map(|(i, x)| (i, x.0))
            .unwrap()
    };
    let (mut wi, mut wv) = worst(out);
    for (i, &v) in vals.iter().enumerate().skip(k) {
        if v < wv {
            out[wi] = (v, i as u32);
            (wi, wv) = worst(out);
        }
    }
}

use exact_l2::sq;

/// Membership of slots seen so far in one reorder: open addressing over a power-of-two table
/// sized for at most `cap` members, cleared between queries.
struct SlotSet {
    table: Vec<u32>,
    shift: u32,
}

impl SlotSet {
    const EMPTY: u32 = u32::MAX;

    fn new(cap: usize) -> Self {
        let size = (2 * cap).next_power_of_two().max(16);
        Self {
            table: vec![Self::EMPTY; size],
            shift: 32 - size.trailing_zeros(),
        }
    }

    fn clear(&mut self) {
        self.table.fill(Self::EMPTY);
    }

    /// True when `slot` was not yet a member.
    fn insert(&mut self, slot: u32) -> bool {
        let mask = self.table.len() - 1;
        let mut i = (slot.wrapping_mul(0x9E37_79B1) >> self.shift) as usize;
        loop {
            match self.table[i] {
                Self::EMPTY => {
                    self.table[i] = slot;
                    return true;
                }
                v if v == slot => return false,
                _ => i = (i + 1) & mask,
            }
        }
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Nearest centre index for every row (`rows` row-major, `dim` wide), parallel over blocks.
fn nearest_all(rows: &[f32], dim: usize, centres: &[f32], c2: &[f32]) -> Vec<u32> {
    let l = c2.len();
    let n = rows.len() / dim;
    let mut out = vec![0u32; n];
    out.par_chunks_mut(ROWS)
        .zip(rows.par_chunks(ROWS * dim))
        .for_each(|(o, x)| {
            let b = o.len();
            let mut xc = vec![0f32; b * l];
            gemm_nt(x, b, centres, l, dim, &mut xc);
            for (i, oi) in o.iter_mut().enumerate() {
                let row = &xc[i * l..(i + 1) * l];
                let mut best = (f32::INFINITY, 0u32);
                for (c, (&v, &cc)) in row.iter().zip(c2).enumerate() {
                    let d = cc - 2.0 * v;
                    if d < best.0 {
                        best = (d, c as u32);
                    }
                }
                *oi = best.1;
            }
        });
    out
}

/// Lloyd k-means (`iters` iterations, random-row init, empty clusters reseeded from rows).
fn kmeans(rows: &[f32], dim: usize, k: usize, iters: usize, rng: &mut SplitMix) -> Vec<f32> {
    kmeans_with(rows, dim, k, iters, false, rng)
}

/// [`kmeans`] with optional rebalancing between iterations (see [`kmeans_from`]).
fn kmeans_with(
    rows: &[f32],
    dim: usize,
    k: usize,
    iters: usize,
    rebalance: bool,
    rng: &mut SplitMix,
) -> Vec<f32> {
    let n = rows.len() / dim;
    let mut c: Vec<f32> = Vec::with_capacity(k * dim);
    for &i in &sample_indices(n, k, rng) {
        c.extend_from_slice(&rows[i * dim..(i + 1) * dim]);
    }
    kmeans_from(rows, dim, c, iters, rebalance, rng)
}

/// Lloyd iterations from the given `k` centres.
fn kmeans_from(
    rows: &[f32],
    dim: usize,
    mut c: Vec<f32>,
    iters: usize,
    rebalance: bool,
    rng: &mut SplitMix,
) -> Vec<f32> {
    let n = rows.len() / dim;
    let k = c.len() / dim;
    for it in 0..iters {
        let c2: Vec<f32> = c.chunks(dim).map(|r| dot(r, r)).collect();
        let a = nearest_all(rows, dim, &c, &c2);
        let mut sums = vec![0f64; k * dim];
        let mut counts = vec![0usize; k];
        for (i, &ai) in a.iter().enumerate() {
            counts[ai as usize] += 1;
            for (s, &x) in sums[ai as usize * dim..(ai as usize + 1) * dim]
                .iter_mut()
                .zip(&rows[i * dim..(i + 1) * dim])
            {
                *s += f64::from(x);
            }
        }
        for j in 0..k {
            if counts[j] > 0 {
                for t in 0..dim {
                    c[j * dim + t] = (sums[j * dim + t] / counts[j] as f64) as f32;
                }
            } else {
                let r = (rng.next() % n as u64) as usize;
                c[j * dim..(j + 1) * dim].copy_from_slice(&rows[r * dim..(r + 1) * dim]);
            }
        }
        if rebalance && it + 1 < iters {
            reseed_light(rows, dim, &a, &counts, &mut c, rng);
        }
    }
    c
}

/// Moves centres that own under a quarter of the mean cluster size onto random members of the
/// clusters above twice the mean, so the next Lloyd step splits the heavy clusters. Centres that
/// drifted away from the data (a stale set after the live distribution moved) keep no capacity
/// otherwise: Lloyd steps only move a centre to the mean of what it already owns.
fn reseed_light(
    rows: &[f32],
    dim: usize,
    a: &[u32],
    counts: &[usize],
    c: &mut [f32],
    rng: &mut SplitMix,
) {
    let n = a.len();
    let k = counts.len();
    let mean = n as f64 / k as f64;
    let light: Vec<usize> = (0..k)
        .filter(|&j| (counts[j] as f64) < mean / 4.0)
        .collect();
    let mut heavy: Vec<usize> = (0..k).filter(|&j| counts[j] as f64 > 2.0 * mean).collect();
    if light.is_empty() || heavy.is_empty() {
        return;
    }
    heavy.sort_unstable_by_key(|&j| std::cmp::Reverse(counts[j]));
    let mut slot = vec![u32::MAX; k];
    for (hi, &j) in heavy.iter().enumerate() {
        slot[j] = hi as u32;
    }
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); heavy.len()];
    for (i, &ai) in a.iter().enumerate() {
        let s = slot[ai as usize];
        if s != u32::MAX {
            members[s as usize].push(i as u32);
        }
    }
    // Heavy clusters receive reseeded centres in proportion to their excess over the mean.
    let excess: Vec<f64> = heavy.iter().map(|&j| counts[j] as f64 - mean).collect();
    let total: f64 = excess.iter().sum();
    for (li, &j) in light.iter().enumerate() {
        let mut t = (li as f64 + 0.5) / light.len() as f64 * total;
        let mut hi = 0;
        while hi + 1 < heavy.len() && t > excess[hi] {
            t -= excess[hi];
            hi += 1;
        }
        let m = &members[hi];
        let r = m[(rng.next() % m.len() as u64) as usize] as usize;
        c[j * dim..(j + 1) * dim].copy_from_slice(&rows[r * dim..(r + 1) * dim]);
    }
}

/// `k` distinct indices below `n` (partial Fisher-Yates).
fn sample_indices(n: usize, k: usize, rng: &mut SplitMix) -> Vec<usize> {
    let k = k.min(n);
    let mut idx: Vec<usize> = (0..n).collect();
    for i in 0..k {
        let j = i + (rng.next() % (n - i) as u64) as usize;
        idx.swap(i, j);
    }
    idx.truncate(k);
    idx.sort_unstable();
    idx
}

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Queries scored per pass over a block by the quad kernel.
const QB: usize = 8;

/// How a list stores its codes: `m` nibbles per code in blocks of [`BLOCK`] codes, in either the
/// LUT16 shuffle layout (`16 * m` bytes per block) or, on AVX-512 VBMI/VNNI hardware, the quad
/// layout (see [`quad_pos`]).
#[derive(Clone, Copy)]
struct Shape {
    m: usize,
    quad: bool,
}

impl Shape {
    fn new(m: usize) -> Self {
        Self {
            m,
            quad: quad_available(),
        }
    }

    /// Bytes per block.
    fn stride(self) -> usize {
        if self.quad {
            quad_stride(self.m)
        } else {
            16 * self.m
        }
    }

    fn pos(self, i: usize, j: usize) -> usize {
        if self.quad {
            quad_pos(i, j)
        } else {
            16 * j + (i & 15)
        }
    }

    /// Nibble `j` of code position `i` within `block`.
    #[inline]
    fn get(self, block: &[u8], i: usize, j: usize) -> u8 {
        let byte = block[self.pos(i, j)];
        if i < 16 {
            byte & 15
        } else {
            byte >> 4
        }
    }

    #[inline]
    fn set(self, block: &mut [u8], i: usize, j: usize, v: u8) {
        let byte = &mut block[self.pos(i, j)];
        if i < 16 {
            *byte = (*byte & 0xF0) | v;
        } else {
            *byte = (*byte & 0x0F) | (v << 4);
        }
    }
}

impl List {
    /// Member radius: mean + 2 sd of member-to-centre distance (about its 97.7th percentile).
    fn radius(&self) -> f32 {
        let n = self.slots.len();
        if n == 0 {
            return 0.0;
        }
        let mean = self.dist_sum / n as f64;
        let var = (self.dist_sq / n as f64 - mean * mean).max(0.0);
        (mean + 2.0 * var.sqrt()) as f32
    }

    /// Adds (`sign` = 1) or removes (`sign` = -1) one member's distance to the radius statistics.
    fn track(&mut self, dist: f32, sign: f64) {
        let d = f64::from(dist);
        self.dist_sum += sign * d;
        self.dist_sq += sign * d * d;
    }

    fn append(&mut self, sh: Shape, code: &[u8], slot: u32) -> u32 {
        let (pos, stride) = (self.slots.len(), sh.stride());
        if pos.is_multiple_of(BLOCK) {
            self.codes.resize(self.codes.len() + stride, 0);
        }
        let block = &mut self.codes[(pos / BLOCK) * stride..(pos / BLOCK + 1) * stride];
        for (j, &v) in code.iter().enumerate() {
            sh.set(block, pos % BLOCK, j, v);
        }
        self.slots.push(slot);
        pos as u32
    }

    /// Swap-removes position `pos`; returns the slot moved into `pos`, if any.
    fn remove(&mut self, sh: Shape, pos: usize) -> Option<u32> {
        let (last, stride) = (self.slots.len() - 1, sh.stride());
        let moved = if pos != last {
            let (lb, li) = (last / BLOCK, last % BLOCK);
            let code: Vec<u8> = (0..sh.m)
                .map(|j| sh.get(&self.codes[lb * stride..(lb + 1) * stride], li, j))
                .collect();
            let block = &mut self.codes[(pos / BLOCK) * stride..(pos / BLOCK + 1) * stride];
            for (j, &v) in code.iter().enumerate() {
                sh.set(block, pos % BLOCK, j, v);
            }
            self.slots[pos] = self.slots[last];
            Some(self.slots[pos])
        } else {
            None
        };
        self.slots.pop();
        if self.slots.len().is_multiple_of(BLOCK) {
            self.codes.truncate((self.slots.len() / BLOCK) * stride);
        } else {
            // Clear the vacated code so padding lanes stay zero.
            let (lb, li) = (last / BLOCK, last % BLOCK);
            let block = &mut self.codes[lb * stride..(lb + 1) * stride];
            for j in 0..sh.m {
                sh.set(block, li, j, 0);
            }
        }
        moved
    }
}

impl Trained {
    fn lists_len(&self) -> usize {
        self.c2.len()
    }

    /// Nearest list and SOAR spill list for each row of `x` (`b` rows).
    fn assign(&self, x: &[f32], dim: usize, k: usize, lambda: f32, eps: f32) -> Vec<(u32, u32)> {
        if let Some(c) = &self.coarse {
            return self.assign_coarse(c, x, dim, k, lambda, eps);
        }
        let l = self.lists_len();
        let mut out = vec![(0u32, 0u32); x.len() / dim];
        out.par_chunks_mut(ROWS)
            .zip(x.par_chunks(ROWS * dim))
            .for_each(|(o, xb)| {
                let b = o.len();
                let mut xc = vec![0f32; b * l];
                gemm_nt(xb, b, &self.centres, l, dim, &mut xc);
                let mut d = vec![0f32; l];
                let mut cand = Vec::with_capacity(k);
                for (i, oi) in o.iter_mut().enumerate() {
                    let row = &xc[i * l..(i + 1) * l];
                    for ((dv, &v), &cc) in d.iter_mut().zip(row).zip(&self.c2) {
                        *dv = cc - 2.0 * v;
                    }
                    *oi = self.pick(
                        &xb[i * dim..(i + 1) * dim],
                        row,
                        None,
                        &d,
                        k,
                        lambda,
                        eps,
                        &mut cand,
                    );
                }
            });
        out
    }

    /// `assign` restricted to the candidate centres of each row's nearest super-centre.
    #[allow(clippy::too_many_arguments)]
    fn assign_coarse(
        &self,
        c: &Coarse,
        x: &[f32],
        dim: usize,
        k: usize,
        lambda: f32,
        eps: f32,
    ) -> Vec<(u32, u32)> {
        let (n, ns, r) = (x.len() / dim, c.s2.len(), c.r);
        let mut sup = vec![0u32; n];
        sup.par_chunks_mut(ROWS)
            .zip(x.par_chunks(ROWS * dim))
            .for_each(|(so, xb)| {
                let b = so.len();
                let mut xc = vec![0f32; b * ns];
                gemm_nt(xb, b, &c.supers, ns, dim, &mut xc);
                for (i, o) in so.iter_mut().enumerate() {
                    let best = xc[i * ns..(i + 1) * ns]
                        .iter()
                        .zip(&c.s2)
                        .map(|(&v, &s)| s - 2.0 * v)
                        .enumerate()
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map_or(0, |x| x.0);
                    *o = best as u32;
                }
            });
        // Rows grouped by super-centre (stable), in units of at most ROWS rows.
        let mut start = vec![0usize; ns + 1];
        for &s in &sup {
            start[s as usize + 1] += 1;
        }
        for i in 0..ns {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut perm = vec![0u32; n];
        for (i, &s) in sup.iter().enumerate() {
            perm[fill[s as usize]] = i as u32;
            fill[s as usize] += 1;
        }
        let mut units: Vec<(usize, usize, usize)> = Vec::new();
        for s in 0..ns {
            for a in (start[s]..start[s + 1]).step_by(ROWS) {
                units.push((s, a, (a + ROWS).min(start[s + 1])));
            }
        }
        let parts: Vec<Vec<(u32, u32)>> = units
            .par_iter()
            .map(|&(s, a, e)| {
                let ids = &c.cands[s * r..(s + 1) * r];
                let mut cm = Vec::with_capacity(r * dim);
                for &id in ids {
                    cm.extend_from_slice(&self.centres[id as usize * dim..(id as usize + 1) * dim]);
                }
                let b = e - a;
                let mut xs = Vec::with_capacity(b * dim);
                for &row in &perm[a..e] {
                    xs.extend_from_slice(&x[row as usize * dim..(row as usize + 1) * dim]);
                }
                let mut xc = vec![0f32; b * r];
                gemm_nt(&xs, b, &cm, r, dim, &mut xc);
                let mut d = vec![0f32; r];
                let mut cand = Vec::with_capacity(k);
                (0..b)
                    .map(|i| {
                        let row = &xc[i * r..(i + 1) * r];
                        for ((dv, &v), &id) in d.iter_mut().zip(row).zip(ids) {
                            *dv = self.c2[id as usize] - 2.0 * v;
                        }
                        self.pick(
                            &xs[i * dim..(i + 1) * dim],
                            row,
                            Some(ids),
                            &d,
                            k,
                            lambda,
                            eps,
                            &mut cand,
                        )
                    })
                    .collect()
            })
            .collect();
        let mut out = vec![(0u32, 0u32); n];
        for (&(_, a, _), part) in units.iter().zip(&parts) {
            for (j, &v) in part.iter().enumerate() {
                out[perm[a + j] as usize] = v;
            }
        }
        out
    }

    /// Primary list and SOAR spill of one row. `row[j]` is `<x, c_j>` and `d[j]` the ranking
    /// distance for candidate `j`, which is centre `ids[j]` (`j` itself when `ids` is None).
    #[allow(clippy::too_many_arguments)]
    fn pick(
        &self,
        xi: &[f32],
        row: &[f32],
        ids: Option<&[u32]>,
        d: &[f32],
        k: usize,
        lambda: f32,
        eps: f32,
        cand: &mut Vec<(f32, u32)>,
    ) -> (u32, u32) {
        let l = self.lists_len();
        let id = |j: u32| ids.map_or(j as usize, |s| s[j as usize] as usize);
        smallest_k(d, k, cand);
        let p1 = cand
            .iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|x| x.1)
            .unwrap();
        let c1 = id(p1);
        let ci = &self.centres[c1 * xi.len()..(c1 + 1) * xi.len()];
        let rr: f64 = xi
            .iter()
            .zip(ci)
            .map(|(a, c)| f64::from(a - c).powi(2))
            .sum::<f64>()
            .max(1e-20);
        let xr: f64 = xi
            .iter()
            .zip(ci)
            .map(|(a, c)| f64::from(*a) * f64::from(a - c))
            .sum();
        let x2: f64 = xi.iter().map(|a| f64::from(*a).powi(2)).sum();
        let g = &self.gram[c1 * l..(c1 + 1) * l];
        let mut best = (f64::INFINITY, c1 as u32);
        let mut best_dist = 0f64;
        for &(_, j) in cand.iter() {
            if j == p1 {
                continue;
            }
            let c = id(j);
            let xcc = f64::from(row[j as usize]);
            let dist = x2 - 2.0 * xcc + f64::from(self.c2[c]);
            // <x - c, r> with r = x - c1: <x, r> - (<c, x> - <c, c1>).
            let proj = xr - (xcc - f64::from(g[c]));
            let loss = dist + f64::from(lambda) * proj * proj / rr;
            if loss < best.0 {
                best = (loss, c as u32);
                best_dist = dist;
            }
        }
        let spill = best.1 != c1 as u32 && best_dist <= (1.0 + f64::from(eps)) * rr;
        (c1 as u32, if spill { best.1 } else { u32::MAX })
    }

    /// PQ nibbles of the residual `x - c_list`.
    fn encode(&self, x: &[f32], list: usize, dim: usize, m: usize, code: &mut [u8]) {
        let sub = dim / m;
        let c = &self.centres[list * dim..(list + 1) * dim];
        for b in 0..m {
            let mut best = (f32::INFINITY, 0u8);
            for v in 0..16 {
                let cb = &self.codebook[(b * 16 + v) * sub..(b * 16 + v + 1) * sub];
                let mut s = 0f32;
                for t in 0..sub {
                    let r = x[b * sub + t] - c[b * sub + t] - cb[t];
                    s += r * r;
                }
                if s < best.0 {
                    best = (s, v as u8);
                }
            }
            code[b] = best.1;
        }
    }
}

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

    fn row(&self, slot: u32) -> &[f32] {
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
    fn place(&mut self, slots: &[u32], vectors: &[f32]) {
        let dim = self.dim;
        let t = self.trained.as_mut().expect("trained");
        let asg = t.assign(
            vectors,
            dim,
            self.params.spill_k,
            self.params.soar_lambda,
            self.params.spill_eps,
        );
        let asg = asg;
        self.place_asg(slots, vectors, &asg);
    }

    /// Encodes and appends the given slots to the lists named by `asg`.
    fn place_asg(&mut self, slots: &[u32], vectors: &[f32], asg: &[(u32, u32)]) {
        let (dim, m, sh) = (self.dim, self.params.pq_m, self.shape);
        let t = self.trained.as_mut().expect("trained");
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
        if t.loc.len() < self.slot_id.len() {
            t.loc.resize(self.slot_id.len(), [(u32::MAX, u32::MAX); 2]);
        }
        for (i, (&slot, &(a1, a2))) in slots.iter().zip(asg).enumerate() {
            let x = &vectors[i * dim..(i + 1) * dim];
            let p1 = t.lists[a1 as usize].append(sh, &codes[2 * i * m..(2 * i + 1) * m], slot);
            let mut p2 = u32::MAX;
            if a2 != u32::MAX {
                p2 =
                    t.lists[a2 as usize].append(sh, &codes[(2 * i + 1) * m..(2 * i + 2) * m], slot);
            }
            for a in [a1, a2].into_iter().filter(|&a| a != u32::MAX) {
                let a = a as usize;
                let d = sq(x, &t.centres[a * dim..(a + 1) * dim]).sqrt();
                t.lists[a].track(d, 1.0);
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
        let coarse = (p.route_cands > 0).then(|| {
            let (ns, r) = (p.route_lists.clamp(1, l), p.route_cands.min(l));
            let supers = kmeans(&centres, dim, ns, 5, &mut SplitMix(p.seed ^ 0xC0A2_5E));
            let s2: Vec<f32> = supers.chunks(dim).map(|v| dot(v, v)).collect();
            let mut cands = Vec::with_capacity(ns * r);
            let (mut d, mut near) = (vec![0f32; l], Vec::with_capacity(r));
            for sv in supers.chunks(dim) {
                for ((dv, cv), &cc) in d.iter_mut().zip(centres.chunks(dim)).zip(&c2) {
                    *dv = cc - 2.0 * dot(sv, cv);
                }
                smallest_k(&d, r, &mut near);
                cands.extend(near.iter().map(|x| x.1));
            }
            Coarse {
                supers,
                s2,
                cands,
                r,
            }
        });
        self.trained = Some(Trained {
            centres,
            c2,
            gram,
            codebook,
            list_tab,
            lists: (0..l).map(|_| List::default()).collect(),
            loc: vec![[(u32::MAX, u32::MAX); 2]; self.slot_id.len()],
            coarse,
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
    fn retrain(&mut self, why: &str) -> Result<(), WarmError> {
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
    fn effective_nprobe(&self) -> usize {
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
    fn list_imbalance(&self) -> f64 {
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

    fn search_partition(&self, queries: &[f32], k: usize) -> Vec<Vec<u32>> {
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
        // Centre distances |q - c|^2 and the probed lists per query.
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
                let mut idx: Vec<u32> = (0..l as u32).collect();
                idx.select_nth_unstable_by(nprobe - 1, |a, b| {
                    row[*a as usize].total_cmp(&row[*b as usize])
                });
                idx.truncate(nprobe);
                idx
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
        // Per-query table term -2 <q_b, v>.
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
                let mut bufs: Vec<Vec<(f32, u32)>> =
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
                                        buf.push((
                                            base + outs[i][c] as f32 * scale,
                                            list.slots[blk * BLOCK + c],
                                        ));
                                    }
                                    if buf.len() >= 2 * reorder {
                                        buf.select_nth_unstable_by(reorder - 1, |a, b| {
                                            a.0.total_cmp(&b.0)
                                        });
                                        buf.truncate(reorder);
                                        // The selected element is the largest kept score.
                                        thr[j] = buf[reorder - 1].0;
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
                                buf.push((
                                    base + out[c] as f32 * scale,
                                    list.slots[blk * BLOCK + c],
                                ));
                            }
                            if buf.len() >= 2 * reorder {
                                buf.select_nth_unstable_by(reorder - 1, |a, b| a.0.total_cmp(&b.0));
                                buf.truncate(reorder);
                                thr[j] = buf.iter().map(|x| x.0).fold(f32::NEG_INFINITY, f32::max);
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
                        buf.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                        let qv = &queries[q as usize * dim..(q as usize + 1) * dim];
                        // The best `reorder` distinct slots in score order.
                        seen.clear();
                        uniq.clear();
                        for (_, s) in buf {
                            if uniq.len() == reorder {
                                break;
                            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn data(n: usize, dim: usize, clusters: usize, seed: u64) -> Vec<f32> {
        let mut r = SplitMix(seed);
        let mut u = move || (r.next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
        let centres: Vec<f32> = (0..clusters * dim).map(|_| 8.0 * u()).collect();
        let mut out = Vec::with_capacity(n * dim);
        for i in 0..n {
            let c = i % clusters;
            for t in 0..dim {
                out.push(centres[c * dim + t] + u());
            }
        }
        out
    }

    fn naive(rows: &[(u32, &[f32])], q: &[f32], k: usize) -> Vec<u32> {
        let mut d: Vec<(f32, u32)> = rows.iter().map(|(id, r)| (sq(q, r), *id)).collect();
        d.sort_by(|a, b| a.0.total_cmp(&b.0));
        d.into_iter().take(k).map(|x| x.1).collect()
    }

    fn recall(a: &[Vec<u32>], b: &[Vec<u32>]) -> f64 {
        let hit: usize = a
            .iter()
            .zip(b)
            .map(|(x, y)| x.iter().filter(|i| y.contains(i)).count())
            .sum();
        hit as f64 / b.iter().map(Vec::len).sum::<usize>() as f64
    }

    /// Every live slot's two memberships point at list positions holding that slot, and the
    /// lists hold exactly two memberships per live row.
    fn check_lists(t: &PartitionTier) {
        let tr = t.trained.as_ref().unwrap();
        let total: usize = tr.lists.iter().map(|l| l.slots.len()).sum();
        let mut memberships = 0;
        for &slot in t.id_slot.values() {
            let e = tr.loc[slot as usize];
            assert_ne!(e[0].0, u32::MAX, "no primary list");
            assert_ne!(e[0].0, e[1].0, "spill list equals primary");
            for (list, pos) in e.into_iter().filter(|x| x.0 != u32::MAX) {
                memberships += 1;
                assert_eq!(tr.lists[list as usize].slots[pos as usize], slot);
            }
        }
        assert_eq!(total, memberships);
        if t.params.spill_eps.is_infinite() {
            assert_eq!(total, 2 * t.len());
        }
        for (li, l) in tr.lists.iter().enumerate() {
            assert_eq!(
                l.codes.len(),
                l.slots.len().div_ceil(BLOCK) * t.shape.stride()
            );
            let c = &tr.centres[li * t.dim..(li + 1) * t.dim];
            let (mut s1, mut s2) = (0f64, 0f64);
            for &slot in &l.slots {
                let d = f64::from(sq(t.row(slot), c).sqrt());
                (s1, s2) = (s1 + d, s2 + d * d);
            }
            assert!(
                (l.dist_sum - s1).abs() <= 1e-6 * (1.0 + s1),
                "list {li} dist_sum"
            );
            assert!(
                (l.dist_sq - s2).abs() <= 1e-6 * (1.0 + s2),
                "list {li} dist_sq"
            );
        }
    }

    /// Distances 1.0, 1.2, 1.5, 2.0 (squared 1, 1.44, 2.25, 4), lists in that order.
    fn ladder() -> Vec<f32> {
        vec![4.0, 1.0, 2.25, 1.44]
    }

    #[test]
    fn select_probes_rules_respect_bounds_and_cutoffs() {
        let d2 = ladder();
        let sel = |rule, nmin, nmax| select_probes(&d2, &[0.0; 4], rule, nmin, nmax);
        // Ratio: d_i^2 <= d_1^2 (1+eps) keeps 1 and 1.44 at eps 0.5.
        assert_eq!(sel(ProbeRule::Ratio { eps: 0.5 }, 1, 4), [1, 3]);
        assert_eq!(sel(ProbeRule::Ratio { eps: 0.0 }, 1, 4), [1]);
        // Floor and cap.
        assert_eq!(sel(ProbeRule::Ratio { eps: 0.0 }, 3, 4), [1, 3, 2]);
        assert_eq!(sel(ProbeRule::Ratio { eps: 9.0 }, 1, 2), [1, 3]);
        // Monotone in eps.
        let mut prev = 0;
        for e in 0..40 {
            let n = sel(
                ProbeRule::Ratio {
                    eps: e as f32 * 0.1,
                },
                1,
                4,
            )
            .len();
            assert!(n >= prev);
            prev = n;
        }
        // Radius: with zero radii the cut is d_i <= (1+eps) d_1 = 1.3, keeping 1.0 and 1.2.
        assert_eq!(
            sel(
                ProbeRule::Radius {
                    eps: 0.3,
                    scale: 1.0
                },
                1,
                4
            ),
            [1, 3]
        );
        // A large radius on the farthest list (d = 2) keeps it past the nearer, small-radius list
        // with d = 1.5; scale 0 ignores radii.
        let radii = [1.0, 0.0, 0.0, 0.0];
        let r = |scale| select_probes(&d2, &radii, ProbeRule::Radius { eps: 0.3, scale }, 1, 4);
        assert_eq!(r(1.0), [1, 3, 0]);
        assert_eq!(r(0.0), [1, 3]);
    }

    /// With a cutoff that never bites, an adaptive rule probes the same lists as `Fixed` and so
    /// returns identical results; `Fixed` ignores the rule fields.
    #[test]
    fn adaptive_rules_without_a_cut_match_fixed() {
        let (n, dim, k) = (3000, 16, 10);
        let x = data(n, dim, 25, 1);
        let q = data(40, dim, 25, 2);
        let ids: Vec<u32> = (0..n as u32).collect();
        let mut t = PartitionTier::new(
            dim,
            PartitionParams {
                nprobe: 7,
                reorder: 40,
                ..params(20)
            },
        )
        .unwrap();
        t.insert(&ids, &x).unwrap();
        let fixed = t.search(&q, k).unwrap();
        let pf = t.last_search_profile();
        assert_eq!(pf.probes, 40 * 7);
        t.set_probe_rule(ProbeRule::Fixed, 5);
        assert_eq!(t.search(&q, k).unwrap(), fixed);
        for rule in [
            ProbeRule::Ratio { eps: 1e30 },
            ProbeRule::Radius {
                eps: 1e30,
                scale: 1.0,
            },
        ] {
            t.set_probe_rule(rule, 1);
            assert_eq!(t.search(&q, k).unwrap(), fixed, "{rule:?}");
            assert_eq!(t.last_search_profile().probes, pf.probes);
        }
        // A tight cut probes fewer lists, never fewer than the floor.
        t.set_probe_rule(ProbeRule::Ratio { eps: 0.0 }, 2);
        t.search(&q, k).unwrap();
        let tight = t.last_search_profile().probes;
        assert!((40 * 2..40 * 7).contains(&(tight as usize)), "{tight}");
    }

    fn params(lists: usize) -> PartitionParams {
        PartitionParams {
            lists,
            train_at: 1000,
            train_sample: 2000,
            pq_m: 8,
            pq_sample: 2000,
            nprobe: lists,
            reorder: 4000,
            ..PartitionParams::default()
        }
    }

    #[test]
    fn full_probe_matches_exact_through_deletes_and_reinserts() {
        let (n, dim, k) = (3000, 16, 10);
        let x = data(n, dim, 25, 1);
        let q = data(40, dim, 25, 2);
        let mut t = PartitionTier::new(dim, params(20)).unwrap();
        let ids: Vec<u32> = (0..n as u32).map(|i| i * 7 + 3).collect();
        t.insert(&ids[..500], &x[..500 * dim]).unwrap();
        // Below train_at: exact brute force.
        let got = t.search(&q, k).unwrap();
        assert!(!t.is_trained());
        let rows: Vec<(u32, &[f32])> = (0..500)
            .map(|i| (ids[i], &x[i * dim..(i + 1) * dim]))
            .collect();
        let want: Vec<Vec<u32>> = q.chunks(dim).map(|qq| naive(&rows, qq, k)).collect();
        assert_eq!(recall(&got, &want), 1.0);
        t.insert(&ids[500..], &x[500 * dim..]).unwrap();
        let got = t.search(&q, k).unwrap();
        assert!(t.is_trained());
        check_lists(&t);
        let rows: Vec<(u32, &[f32])> = (0..n)
            .map(|i| (ids[i], &x[i * dim..(i + 1) * dim]))
            .collect();
        let want: Vec<Vec<u32>> = q.chunks(dim).map(|qq| naive(&rows, qq, k)).collect();
        assert_eq!(recall(&got, &want), 1.0);
        // Delete every third row, reinsert a third of those, check invariants and exactness.
        let del: Vec<u32> = ids.iter().copied().step_by(3).collect();
        assert_eq!(t.delete(&del), del.len());
        assert_eq!(t.delete(&del[..5]), 0);
        check_lists(&t);
        let back: Vec<usize> = (0..n).step_by(9).collect();
        let bid: Vec<u32> = back.iter().map(|&i| ids[i]).collect();
        let bx: Vec<f32> = back
            .iter()
            .flat_map(|&i| x[i * dim..(i + 1) * dim].to_vec())
            .collect();
        t.insert(&bid, &bx).unwrap();
        assert!(t.insert(&bid[..1], &bx[..dim]).is_err());
        check_lists(&t);
        let live: Vec<(u32, &[f32])> = (0..n)
            .filter(|i| i % 3 != 0 || i % 9 == 0)
            .map(|i| (ids[i], &x[i * dim..(i + 1) * dim]))
            .collect();
        assert_eq!(live.len(), t.len());
        let want: Vec<Vec<u32>> = q.chunks(dim).map(|qq| naive(&live, qq, k)).collect();
        assert_eq!(recall(&t.search(&q, k).unwrap(), &want), 1.0);
        assert_eq!(recall(&t.search_exact(&q, k), &want), 1.0);
    }

    /// Starting from centres that sit on a small part of the data (a stale set), a few Lloyd
    /// iterations with rebalancing give clusters that are more even than without it.
    #[test]
    fn rebalancing_kmeans_evens_out_stale_centres() {
        let (n, dim, k) = (6000, 8, 40);
        let x = data(n, dim, 8, 5);
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&i, &j| x[i * dim].total_cmp(&x[j * dim]));
        let stale: Vec<f32> = order[..k]
            .iter()
            .flat_map(|&i| x[i * dim..(i + 1) * dim].to_vec())
            .collect();
        let spread = |c: &[f32]| -> f64 {
            let c2: Vec<f32> = c.chunks(dim).map(|r| dot(r, r)).collect();
            let a = nearest_all(&x, dim, c, &c2);
            let mut counts = vec![0f64; k];
            for &ai in &a {
                counts[ai as usize] += 1.0;
            }
            counts.iter().map(|v| v * v).sum::<f64>() * k as f64 / (n as f64 * n as f64)
        };
        let plain = kmeans_from(&x, dim, stale.clone(), 3, false, &mut SplitMix(1));
        let even = kmeans_from(&x, dim, stale, 3, true, &mut SplitMix(1));
        assert!(
            spread(&even) < spread(&plain),
            "{} vs {}",
            spread(&even),
            spread(&plain)
        );
    }

    #[test]
    fn nprobe_scales_piecewise_linearly_with_live_size() {
        let dim = 4;
        let mut t = PartitionTier::new(
            dim,
            PartitionParams {
                lists: 100,
                nprobe: 40,
                pq_m: 2,
                nprobe_by_live: vec![(100, 2.0), (300, 1.0), (500, 0.5)],
                ..PartitionParams::default()
            },
        )
        .unwrap();
        let x = vec![0.5f32; 600 * dim];
        let ids: Vec<u32> = (0..600).collect();
        let mut seen = Vec::new();
        for upto in [50usize, 100, 200, 300, 400, 500, 600] {
            let have = t.len();
            t.insert(&ids[have..upto], &x[have * dim..upto * dim])
                .unwrap();
            seen.push(t.effective_nprobe());
        }
        // 40 * (2.0, 2.0, 1.5, 1.0, 0.75, 0.5, 0.5), capped by the list count.
        assert_eq!(seen, vec![80, 80, 60, 40, 30, 20, 20]);
    }

    /// Retraining (scheduled by live size, by imbalance, cold or warm) re-places every live row:
    /// the list invariants hold and a full probe still equals exact search after deletes.
    #[test]
    fn retrain_keeps_every_live_row_searchable() {
        let (n, dim, k) = (4000, 16, 10);
        let x = data(n, dim, 25, 3);
        let q = data(30, dim, 25, 4);
        let ids: Vec<u32> = (0..n as u32).collect();
        let want_for = |live: &[usize]| -> Vec<Vec<u32>> {
            let rows: Vec<(u32, &[f32])> = live
                .iter()
                .map(|&i| (ids[i], &x[i * dim..(i + 1) * dim]))
                .collect();
            q.chunks(dim).map(|qq| naive(&rows, qq, k)).collect()
        };
        for (warm, imb, route) in [
            (0, 0.0f32, 0),
            (2, 0.0, 0),
            (2, 1.0001, 0),
            (2, 0.0, 6),
            (2, 1.0001, 4),
        ] {
            let mut t = PartitionTier::new(
                dim,
                PartitionParams {
                    retrain_at: vec![2000, 3000],
                    retrain_imb: imb,
                    retrain_min_inserts: 500,
                    retrain_warm_iters: warm,
                    retrain_sample: 1500,
                    route_cands: route,
                    spill_eps: if imb > 0.0 { 0.25 } else { f32::INFINITY },
                    ..params(20)
                },
            )
            .unwrap();
            t.insert(&ids[..1200], &x[..1200 * dim]).unwrap();
            t.search(&q, k).unwrap();
            assert_eq!((t.trained_live, t.retrain_next), (1200, 0));
            t.insert(&ids[1200..2500], &x[1200 * dim..2500 * dim])
                .unwrap();
            let got = t.search(&q, k).unwrap();
            // The 2000 threshold fired once; the imbalance rule may add retrains.
            assert!(
                t.trained_live >= 2500 && t.retrain_next >= 1,
                "warm {warm} imb {imb} route {route}"
            );
            check_lists(&t);
            assert_eq!(recall(&got, &want_for(&(0..2500).collect::<Vec<_>>())), 1.0);
            let del: Vec<u32> = ids[..2500].iter().copied().step_by(4).collect();
            t.delete(&del);
            t.insert(&ids[2500..], &x[2500 * dim..]).unwrap();
            let got = t.search(&q, k).unwrap();
            check_lists(&t);
            let live: Vec<usize> = (0..n).filter(|&i| i >= 2500 || i % 4 != 0).collect();
            assert_eq!(live.len(), t.len());
            assert_eq!(recall(&got, &want_for(&live)), 1.0);
        }
    }

    /// The quad layout (AVX-512 VBMI/VNNI) and the LUT16 layout give the same results, blocks
    /// and admissions through deletes and reinserts, with a reorder small enough that the
    /// running threshold prunes and an `m` that pads the last quad.
    #[test]
    fn slot_set_agrees_with_a_hash_set() {
        let mut rng = SplitMix(9);
        for cap in [1usize, 7, 1000] {
            let mut set = SlotSet::new(cap);
            let mut want = std::collections::HashSet::new();
            for round in 0..3 {
                set.clear();
                want.clear();
                // `cap` distinct members at most, with repeats and clustered values.
                for _ in 0..4 * cap {
                    let s = (rng.next() % (cap as u64).max(2)) as u32 * 4096 + round;
                    if want.len() < cap || want.contains(&s) {
                        assert_eq!(set.insert(s), want.insert(s), "cap {cap}");
                    }
                }
            }
        }
    }

    #[test]
    fn quad_layout_equals_lut16_layout() {
        if !quad_available() {
            eprintln!("skipped: no AVX-512 VBMI/VNNI");
            return;
        }
        let (n, dim, k) = (3000, 40, 10);
        let x = data(n, dim, 25, 1);
        // Enough queries that each thread meets full chunks of `QB` and partial ones.
        let q = data(400, dim, 25, 2);
        let ids: Vec<u32> = (0..n as u32).map(|i| i * 5 + 1).collect();
        for m in [8usize, 10] {
            let mk = |quad: bool| {
                let mut t = PartitionTier::new(
                    dim,
                    PartitionParams {
                        pq_m: m,
                        nprobe: 8,
                        reorder: 40,
                        ..params(20)
                    },
                )
                .unwrap();
                t.shape.quad = quad;
                t
            };
            let (mut a, mut b) = (mk(true), mk(false));
            for t in [&mut a, &mut b] {
                t.insert(&ids, &x).unwrap();
            }
            let same = |a: &mut PartitionTier, b: &mut PartitionTier| {
                let (ra, rb) = (a.search(&q, k).unwrap(), b.search(&q, k).unwrap());
                assert_eq!(ra, rb, "m {m}");
                let (pa, pb) = (a.last_search_profile(), b.last_search_profile());
                assert_eq!((pa.blocks, pa.admitted), (pb.blocks, pb.admitted), "m {m}");
            };
            // The first search trains both tiers.
            same(&mut a, &mut b);
            check_lists(&a);
            check_lists(&b);
            let del: Vec<u32> = ids.iter().copied().step_by(3).collect();
            for t in [&mut a, &mut b] {
                assert_eq!(t.delete(&del), del.len());
                check_lists(t);
                t.insert(&del[..200], &x[..200 * dim]).unwrap();
                check_lists(t);
            }
            same(&mut a, &mut b);
        }
    }

    #[test]
    fn restricted_spill_equals_full_soar_when_all_centres_are_candidates() {
        let (n, dim, lists) = (2000, 16, 12);
        let x = data(n, dim, 30, 3);
        let mut p = params(lists);
        p.spill_k = lists;
        let mut t = PartitionTier::new(dim, p).unwrap();
        t.insert(&(0..n as u32).collect::<Vec<_>>(), &x).unwrap();
        t.train(None).unwrap();
        let tr = t.trained.as_ref().unwrap();
        let asg = tr.assign(&x, dim, lists, 1.0, f32::INFINITY);
        for (i, &(a1, a2)) in asg.iter().enumerate() {
            let xi: Vec<f64> = x[i * dim..(i + 1) * dim]
                .iter()
                .map(|&v| f64::from(v))
                .collect();
            let c = |j: usize| -> Vec<f64> {
                tr.centres[j * dim..(j + 1) * dim]
                    .iter()
                    .map(|&v| f64::from(v))
                    .collect()
            };
            let d = |j: usize| -> f64 { xi.iter().zip(c(j)).map(|(a, b)| (a - b).powi(2)).sum() };
            let near = (0..lists).min_by(|&a, &b| d(a).total_cmp(&d(b))).unwrap();
            assert!(d(a1 as usize) <= d(near) * (1.0 + 1e-5) + 1e-6);
            let r: Vec<f64> = xi.iter().zip(c(a1 as usize)).map(|(a, b)| a - b).collect();
            let rr: f64 = r.iter().map(|v| v * v).sum::<f64>().max(1e-20);
            let loss = |j: usize| -> f64 {
                let xc: Vec<f64> = xi.iter().zip(c(j)).map(|(a, b)| a - b).collect();
                let pr: f64 = xc.iter().zip(&r).map(|(a, b)| a * b).sum();
                xc.iter().map(|v| v * v).sum::<f64>() + pr * pr / rr
            };
            let best = (0..lists)
                .filter(|&j| j != a1 as usize)
                .map(loss)
                .fold(f64::INFINITY, f64::min);
            assert_ne!(a2, a1);
            assert!(loss(a2 as usize) <= best * (1.0 + 1e-4) + 1e-6, "row {i}");
        }
    }

    #[test]
    fn coarse_assignment_equals_full_when_all_centres_are_candidates() {
        let (n, dim, lists) = (1500, 16, 12);
        let x = data(n, dim, 20, 5);
        let mut t = PartitionTier::new(
            dim,
            PartitionParams {
                route_cands: lists,
                route_lists: 3,
                ..params(lists)
            },
        )
        .unwrap();
        t.insert(&(0..n as u32).collect::<Vec<_>>(), &x).unwrap();
        t.train(None).unwrap();
        let tr = t.trained.as_mut().unwrap();
        assert!(tr.coarse.is_some());
        let routed = tr.assign(&x, dim, 8, 1.0, f32::INFINITY);
        let coarse = tr.coarse.take();
        let full = tr.assign(&x, dim, 8, 1.0, f32::INFINITY);
        tr.coarse = coarse;
        assert_eq!(routed, full);
    }

    /// The `i`-th token of a `;`-separated rule list as a JSON-key-safe label.
    fn tokens(rules: &str, i: usize) -> String {
        rules.split(';').nth(i).unwrap().replace(':', "_")
    }

    /// A2 prefix acceptance (STATE.md 02:15Z): WARM_L22_ACC_{BASE,N,QUERIES,GT,CENTRES,OUT}.
    /// Base is the msturing fbin (first N rows), queries `nq x 100` f32, GT `nq x 10` i32,
    /// centres `5000 x 100` f32 (A2's, reproduced by seed). Prints recall at nprobe 350 and 700.
    #[test]
    #[ignore]
    fn partition_prefix_acceptance() {
        use std::time::Instant;
        let var = |k: &str| crate::compatibility::var(&format!("WARM_L22_ACC_{k}")).unwrap();
        let f32s = |b: Vec<u8>| -> Vec<f32> {
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect()
        };
        let dim = 100;
        let n: usize = var("N").parse().unwrap();
        let base = {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(var("BASE")).unwrap();
            f.seek(SeekFrom::Start(8)).unwrap();
            let mut b = vec![0u8; n * dim * 4];
            f.read_exact(&mut b).unwrap();
            f32s(b)
        };
        let q = f32s(std::fs::read(var("QUERIES")).unwrap());
        let gt: Vec<Vec<u32>> = std::fs::read(var("GT"))
            .unwrap()
            .as_chunks::<40>()
            .0
            .iter()
            .map(|r| {
                r.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| i32::from_le_bytes(*c) as u32)
                    .collect()
            })
            .collect();
        let envn = |k: &str| -> Option<usize> {
            crate::compatibility::var(&format!("WARM_L22_ACC_{k}"))
                .ok()
                .map(|v| v.trim().parse().unwrap())
        };
        let defaults = PartitionParams::default();
        let lists = envn("LISTS").unwrap_or(defaults.lists);
        // Centres: a file (control), or k-means on the first TRAIN_N rows of TRAIN_BASE
        // (replay of a retrain at that state), or None = train at the evaluated state.
        let t0 = Instant::now();
        let centres: Option<Vec<f32>> =
            if let Ok(p) = crate::compatibility::var("WARM_L22_ACC_CENTRES") {
                Some(f32s(std::fs::read(p).unwrap()))
            } else if let Ok(tb) = crate::compatibility::var("WARM_L22_ACC_TRAIN_BASE") {
                use std::io::{Read, Seek, SeekFrom};
                let tn = envn("TRAIN_N").unwrap();
                let mut f = std::fs::File::open(tb).unwrap();
                f.seek(SeekFrom::Start(8)).unwrap();
                let mut b = vec![0u8; tn * dim * 4];
                f.read_exact(&mut b).unwrap();
                let rows = f32s(b);
                let mut rng = SplitMix(envn("SEED").map_or(defaults.seed, |v| v as u64));
                let idx = sample_indices(
                    tn,
                    envn("TRAIN_SAMPLE").unwrap_or(defaults.train_sample),
                    &mut rng,
                );
                let mut sample = Vec::with_capacity(idx.len() * dim);
                for &i in &idx {
                    sample.extend_from_slice(&rows[i * dim..(i + 1) * dim]);
                }
                let rebal = envn("REBAL") == Some(1);
                let stale = kmeans_with(
                    &sample,
                    dim,
                    lists,
                    envn("KM_ITERS").unwrap_or(defaults.kmeans_iters),
                    rebal,
                    &mut rng,
                );
                if let Some(wi) = envn("WARM_ITERS") {
                    // Retrain warm-started from the stale centres on a sample of the evaluated state.
                    let idx = sample_indices(
                        n,
                        envn("WARM_SAMPLE").unwrap_or(defaults.train_sample),
                        &mut rng,
                    );
                    let mut ws = Vec::with_capacity(idx.len() * dim);
                    for &i in &idx {
                        ws.extend_from_slice(&base[i * dim..(i + 1) * dim]);
                    }
                    Some(kmeans_from(&ws, dim, stale, wi, rebal, &mut rng))
                } else {
                    Some(stale)
                }
            } else {
                None
            };
        let kmeans_s = t0.elapsed().as_secs_f64();
        let mut t = PartitionTier::new(
            dim,
            PartitionParams {
                train_at: usize::MAX,
                lists,
                train_sample: envn("TRAIN_SAMPLE").unwrap_or(defaults.train_sample),
                kmeans_iters: envn("KM_ITERS").unwrap_or(defaults.kmeans_iters),
                spill_k: envn("SPILLK").unwrap_or(defaults.spill_k),
                seed: envn("SEED").map_or(defaults.seed, |v| v as u64),
                kmeans_rebalance: envn("REBAL") == Some(1),
                spill_eps: crate::compatibility::var("WARM_L22_ACC_SPILLEPS")
                    .map_or(f32::INFINITY, |v| v.trim().parse().unwrap()),
                soar_lambda: crate::compatibility::var("WARM_L22_ACC_LAMBDA")
                    .map_or(defaults.soar_lambda, |v| v.trim().parse().unwrap()),
                reorder: crate::compatibility::var("WARM_L22_ACC_REORDER")
                    .map(|v| v.trim().parse().unwrap())
                    .unwrap_or(PartitionParams::default().reorder),
                pq_m: crate::compatibility::var("WARM_L22_ACC_PQM")
                    .map(|v| v.trim().parse().unwrap())
                    .unwrap_or(PartitionParams::default().pq_m),
                ..PartitionParams::default()
            },
        )
        .unwrap();
        let t0 = Instant::now();
        t.insert(&(0..n as u32).collect::<Vec<_>>(), &base).unwrap();
        let insert_s = t0.elapsed().as_secs_f64();
        let t0 = Instant::now();
        t.train(centres).unwrap();
        let train_s = t0.elapsed().as_secs_f64();
        // RETRAIN=1: replay a warm retrain of the placed tier (stale centres above) with the
        // RT_ITERS / RT_SAMPLE / REBAL / ROUTE knobs and report its phases.
        let mut retrain_rep = String::new();
        if envn("RETRAIN") == Some(1) {
            t.params.retrain_warm_iters = envn("RT_ITERS").unwrap_or(3);
            t.params.retrain_sample = envn("RT_SAMPLE").unwrap_or(1_000_000);
            t.params.route_cands = envn("ROUTE").unwrap_or(0);
            t.params.route_lists = envn("ROUTES").unwrap_or(64);
            let t0 = Instant::now();
            t.train(None).unwrap();
            let ph = t.train_phases;
            retrain_rep = format!(
                ", \"retrain_s\": {:.3}, \"retrain_centres_s\": {:.3}, \"retrain_tables_s\": {:.3}, \"retrain_place_s\": {:.3}",
                t0.elapsed().as_secs_f64(),
                ph[0],
                ph[1],
                ph[2]
            );
        }
        let ls = {
            let mut v: Vec<usize> = t
                .trained
                .as_ref()
                .unwrap()
                .lists
                .iter()
                .map(|l| l.slots.len())
                .collect();
            v.sort_unstable();
            let tot: usize = v.iter().sum();
            let sq: f64 = v.iter().map(|&x| (x as f64).powi(2)).sum();
            format!(
                "{{\"entries\": {tot}, \"max\": {}, \"p99\": {}, \"p50\": {}, \"empty\": {}, \"imb\": {:.3}}}",
                v[v.len() - 1],
                v[v.len() * 99 / 100],
                v[v.len() / 2],
                v.iter().filter(|&&x| x == 0).count(),
                sq * v.len() as f64 / (tot as f64).powi(2)
            )
        };
        let mut rep = format!("{{\"n\": {n}, \"nq\": {}, \"insert_rows_s\": {insert_s:.3}, \"kmeans_s\": {kmeans_s:.3}, \"train_place_s\": {train_s:.3}, \"lists\": {ls}{retrain_rep}", gt.len());
        // WARM_L22_ACC_RULES: `;`-separated `fixed:NP`, `ratio:EPS:MIN:MAX`, `radius:EPS:SCALE:MIN:MAX`.
        // Each is searched in order after one discarded warm-up search; keys are `<idx>_<rule>`.
        let rules: Vec<(String, ProbeRule, usize, usize)> =
            crate::compatibility::var("WARM_L22_ACC_RULES")
                .map(|v| {
                    v.split(';')
                        .map(|tok| {
                            let f: Vec<&str> = tok.split(':').collect();
                            let num = |i: usize| f[i].trim().parse::<f32>().unwrap();
                            match f[0] {
                                "fixed" => (ProbeRule::Fixed, 1, num(1) as usize),
                                "ratio" => (
                                    ProbeRule::Ratio { eps: num(1) },
                                    num(2) as usize,
                                    num(3) as usize,
                                ),
                                "radius" => (
                                    ProbeRule::Radius {
                                        eps: num(1),
                                        scale: num(2),
                                    },
                                    num(3) as usize,
                                    num(4) as usize,
                                ),
                                other => panic!("rule {other}"),
                            }
                        })
                        .enumerate()
                        .map(|(i, (r, lo, hi))| (format!("{i}_{}", tokens(&v, i)), r, lo, hi))
                        .collect()
                })
                .unwrap_or_else(|_| {
                    crate::compatibility::var("WARM_L22_ACC_NPROBES")
                        .map(|v| v.split(',').map(|x| x.trim().parse().unwrap()).collect())
                        .unwrap_or_else(|_| vec![350usize, 700])
                        .into_iter()
                        .map(|np| (format!("np{np}"), ProbeRule::Fixed, 1, np))
                        .collect()
                });
        if crate::compatibility::var("WARM_L22_ACC_RULES").is_ok() {
            t.set_nprobe(rules[0].3);
            let _ = t.search(&q, 10).unwrap();
        }
        let nsearch = (q.len() / dim) as f64;
        for (label, rule, lo, hi) in rules {
            t.set_nprobe(hi);
            t.set_probe_rule(rule, lo);
            let t0 = Instant::now();
            let got = t.search(&q, 10).unwrap();
            let s = t0.elapsed().as_secs_f64();
            let pf = t.last_search_profile();
            let digest = got
                .iter()
                .flatten()
                .fold(0xcbf2_9ce4_8422_2325u64, |h, &id| {
                    (h ^ u64::from(id)).wrapping_mul(0x0000_0100_0000_01b3)
                });
            let miss = {
                let tr = t.trained.as_ref().unwrap();
                let l = tr.lists_len();
                let mut per_q: Vec<usize> = Vec::new();
                let (mut ranks, mut sizes) = (Vec::new(), Vec::new());
                for (qi, (g, want)) in got.iter().zip(&gt).enumerate() {
                    let miss: Vec<u32> = want.iter().copied().filter(|w| !g.contains(w)).collect();
                    per_q.push(miss.len());
                    if miss.is_empty() {
                        continue;
                    }
                    let qv = &q[qi * dim..(qi + 1) * dim];
                    let d: Vec<f32> = (0..l)
                        .map(|c| sq(qv, &tr.centres[c * dim..(c + 1) * dim]))
                        .collect();
                    for w in miss {
                        let e = tr.loc[w as usize];
                        let best = e
                            .iter()
                            .filter(|x| x.0 != u32::MAX)
                            .map(|x| x.0 as usize)
                            .min_by(|&a, &b| d[a].total_cmp(&d[b]))
                            .unwrap();
                        ranks.push(d.iter().filter(|&&x| x < d[best]).count());
                        sizes.push(tr.lists[best].slots.len());
                    }
                }
                let total: usize = per_q.iter().sum();
                let mut sorted = per_q.clone();
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                let top10: usize = sorted.iter().take(10).sum();
                ranks.sort_unstable();
                sizes.sort_unstable();
                let pick = |v: &Vec<usize>, f: usize| {
                    v.get((v.len().saturating_sub(1)) * f / 100)
                        .copied()
                        .unwrap_or(0)
                };
                format!(
                    "{{\"missed\": {total}, \"queries_missing\": {}, \"top10_queries\": {top10}, \"worst_q\": {}, \"rank_p50\": {}, \"rank_p90\": {}, \"rank_max\": {}, \"size_p50\": {}, \"size_p90\": {}}}",
                    per_q.iter().filter(|&&x| x > 0).count(),
                    sorted[0],
                    pick(&ranks, 50), pick(&ranks, 90), pick(&ranks, 100),
                    pick(&sizes, 50), pick(&sizes, 90)
                )
            };
            rep += &format!(", \"miss_{label}\": {miss}");
            rep += &format!(
                ", \"digest_{label}\": \"{digest:016x}\", \"recall_{label}\": {:.4}, \"search_{label}_s\": {s:.3}, \"profile_{label}\": \
                 {{\"centres\": {:.3}, \"route\": {:.3}, \"scan\": {:.3}, \"scan_mean\": {:.3}, \"lut_mean\": {:.3}, \"probes_mean\": {:.1}, \"blocks\": {}, \"admitted\": {}, \
                 \"reorder\": {:.3}, \"total\": {:.3}}}",
                recall(&got, &gt),
                pf.centres_s,
                pf.route_s,
                pf.scan_s,
                pf.scan_mean_s,
                pf.lut_mean_s,
                pf.probes as f64 / nsearch,
                pf.blocks,
                pf.admitted,
                pf.reorder_s,
                pf.total_s
            );
        }
        rep += "}";
        println!("{rep}");
        if let Ok(p) = crate::compatibility::var("WARM_L22_ACC_OUT") {
            std::fs::write(p, rep + "\n").unwrap();
        }
    }
}
