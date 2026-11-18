// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(super) fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Nearest centre index for every row (`rows` row-major, `dim` wide), parallel over blocks.
pub(super) fn nearest_all(rows: &[f32], dim: usize, centres: &[f32], c2: &[f32]) -> Vec<u32> {
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
pub(super) fn kmeans(
    rows: &[f32],
    dim: usize,
    k: usize,
    iters: usize,
    rng: &mut SplitMix,
) -> Vec<f32> {
    kmeans_with(rows, dim, k, iters, false, rng)
}

/// [`kmeans`] with optional rebalancing between iterations (see [`kmeans_from`]).
pub(super) fn kmeans_with(
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
pub(super) fn kmeans_from(
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
pub(super) fn reseed_light(
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
pub(super) fn sample_indices(n: usize, k: usize, rng: &mut SplitMix) -> Vec<usize> {
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

pub(super) struct SplitMix(pub(super) u64);

impl SplitMix {
    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}
