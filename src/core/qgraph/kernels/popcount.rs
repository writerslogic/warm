// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// A query coordinate vector quantized to [`QUERY_BITS`] unsigned bits per coordinate and
/// stored as bit planes: `planes[p * words + w]` holds bit `p` of coordinates `64w..64w+63`.
/// `sum_i b_i x_i ~= lo * popcount(b) + delta * raw(b)` for any bit vector `b`.
pub(crate) struct QueryCode {
    pub(crate) planes: Vec<u64>,
    pub(crate) lo: f32,
    pub(crate) delta: f32,
    /// Sum of the dequantized coordinates.
    pub(crate) sum: f32,
}

impl QueryCode {
    pub(crate) fn new(words: usize) -> Self {
        Self {
            planes: vec![0; QUERY_BITS * words],
            lo: 0.0,
            delta: 1.0,
            sum: 0.0,
        }
    }

    pub(crate) fn encode(&mut self, x: &[f32]) {
        let words = self.planes.len() / QUERY_BITS;
        debug_assert_eq!(x.len(), 64 * words);
        let (lo, hi) = x
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &v| {
                (l.min(v), h.max(v))
            });
        let levels = ((1 << QUERY_BITS) - 1) as f32;
        let delta = if hi > lo { (hi - lo) / levels } else { 1.0 };
        self.planes.fill(0);
        let mut total = 0u32;
        // One word of every plane is built in registers per 64 coordinates, then stored once.
        for (w, chunk) in x.chunks(64).enumerate() {
            let mut bits = [0u64; QUERY_BITS];
            for (j, &v) in chunk.iter().enumerate() {
                let q = ((v - lo) / delta).round().clamp(0.0, levels) as u32;
                total += q;
                for (p, b) in bits.iter_mut().enumerate() {
                    *b |= u64::from((q >> p) & 1) << j;
                }
            }
            for (p, b) in bits.into_iter().enumerate() {
                self.planes[p * words + w] = b;
            }
        }
        self.lo = lo;
        self.delta = delta;
        self.sum = lo * x.len() as f32 + delta * total as f32;
    }
}

/// Narrowest row (in u64 words) the AVX2 screen kernel takes on x86. At 2 words it lost to
/// POPCNT in every shape of `screen_popcount_microbench` (paired median 1.06 to 1.67); from 4
/// words it won in every shape (`benchmarks/results/platform_x86_screen_popcount_v1.json`).
#[cfg(target_arch = "x86_64")]
pub(super) const AVX2_SCREEN_MIN_WORDS: usize = 4;

/// For each code row (`words` u64 each) computes `sum_p 2^p * popcount(code & plane_p)`.
/// x86: AVX2 from [`AVX2_SCREEN_MIN_WORDS`], else POPCNT when detected, else scalar.
pub(crate) fn raw_estimates(codes: &[u64], planes: &[u64], words: usize, out: &mut [u32]) {
    #[cfg(target_arch = "aarch64")]
    match words {
        1 => return neon::raw::<1>(codes, planes, out),
        2 => return neon::raw::<2>(codes, planes, out),
        4 => return neon::raw::<4>(codes, planes, out),
        8 => return neon::raw::<8>(codes, planes, out),
        16 => return neon::raw::<16>(codes, planes, out),
        _ => {}
    }
    #[cfg(target_arch = "x86_64")]
    if words >= AVX2_SCREEN_MIN_WORDS && tier::screen_avx2() {
        // SAFETY: `screen_avx2` is true only after detecting `avx2` and `popcnt`.
        return unsafe { x86::raw_avx2(codes, planes, words, out) };
    }
    #[cfg(target_arch = "x86_64")]
    if tier::popcnt() {
        // SAFETY: `popcnt` is true only after detecting the `popcnt` feature.
        return unsafe { x86::raw_popcnt(codes, planes, words, out) };
    }
    raw_scalar(codes, planes, words, out)
}

pub(super) fn raw_scalar(codes: &[u64], planes: &[u64], words: usize, out: &mut [u32]) {
    assert_eq!(planes.len(), QUERY_BITS * words);
    assert_eq!(codes.len(), out.len() * words);
    for (code, o) in codes.chunks_exact(words).zip(out.iter_mut()) {
        *o = raw_one_scalar(code, planes, words);
    }
}

/// [`raw_estimates`] for one code row: NEON AND + CNT over the bit planes for the row widths
/// the index produces (16 planes x 64 words at most); on x86 the same choice as
/// [`raw_estimates`]; scalar otherwise.
#[must_use]
#[inline]
pub(crate) fn raw_one(code: &[u64], planes: &[u64], words: usize) -> u32 {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = [0u32; 1];
        match words {
            1 => {
                neon::raw::<1>(code, planes, &mut out);
                return out[0];
            }
            2 => {
                neon::raw::<2>(code, planes, &mut out);
                return out[0];
            }
            4 => {
                neon::raw::<4>(code, planes, &mut out);
                return out[0];
            }
            8 => {
                neon::raw::<8>(code, planes, &mut out);
                return out[0];
            }
            16 => {
                neon::raw::<16>(code, planes, &mut out);
                return out[0];
            }
            _ => {}
        }
    }
    #[cfg(target_arch = "x86_64")]
    if words >= AVX2_SCREEN_MIN_WORDS && tier::screen_avx2() {
        // SAFETY: `screen_avx2` is true only after detecting `avx2` and `popcnt`.
        return unsafe { x86::raw_one_avx2(code, planes, words) };
    }
    #[cfg(target_arch = "x86_64")]
    if tier::popcnt() {
        // SAFETY: `popcnt` is true only after detecting the `popcnt` feature.
        return unsafe { x86::raw_one_popcnt(code, planes, words) };
    }
    raw_one_scalar(code, planes, words)
}

/// Scalar form of [`raw_one`] (the reference, and the microbenchmark's baseline).
#[inline]
pub(super) fn raw_one_scalar(code: &[u64], planes: &[u64], words: usize) -> u32 {
    code.iter()
        .enumerate()
        .map(|(w, &c)| scalar_word(c, planes, words, w))
        .sum()
}

pub(super) fn scalar_word(c: u64, planes: &[u64], words: usize, w: usize) -> u32 {
    (0..QUERY_BITS)
        .map(|p| (c & planes[p * words + w]).count_ones() << p)
        .sum()
}
