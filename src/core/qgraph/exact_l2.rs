// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Exact squared-L2 distances for the partition tier's reorder.
//!
//! [`sq_batch`] returns, for every candidate, the same `f32` as the sequential sum
//! `sum((q[d] - r[d]) * (q[d] - r[d]))` over `d` in order (a subtract, a multiply and an add per
//! element, never fused), so the reorder keeps its bit-exact results. The AVX path keeps eight
//! candidates in the lanes of one register, which turns the latency-bound chain of adds of a single
//! candidate into eight independent chains.

/// Candidates scored together by [`sq_batch`].
pub(super) const LANES: usize = 8;

/// Writes `out[i] = sq(q, rows[slots[i] * dim..][..dim])` for each slot. `rows` is the flat
/// row-major table, `q.len() == dim`, and every slot indexes a whole row.
pub(super) fn sq_batch(q: &[f32], rows: &[f32], dim: usize, slots: &[u32], out: &mut [f32]) {
    assert!(q.len() == dim && out.len() >= slots.len());
    for &s in slots {
        assert!((s as usize + 1) * dim <= rows.len());
    }
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_mut))]
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx") {
        while i + LANES <= slots.len() {
            let mut s = [0u32; LANES];
            s.copy_from_slice(&slots[i..i + LANES]);
            // SAFETY: AVX detected; the loop above checked that every slot's row lies inside
            // `rows` and `q` holds `dim` floats.
            unsafe { avx::sq8(q.as_ptr(), rows.as_ptr(), dim, s, out[i..].as_mut_ptr()) };
            i += LANES;
        }
    }
    for (o, &s) in out[i..slots.len()].iter_mut().zip(&slots[i..]) {
        *o = sq(q, &rows[s as usize * dim..(s as usize + 1) * dim]);
    }
}

/// Requests the cache lines of one row ahead of its use.
#[inline]
pub(super) fn prefetch_row(rows: &[f32], dim: usize, slot: u32) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
        let row = &rows[slot as usize * dim..(slot as usize + 1) * dim];
        for line in row.chunks(16) {
            // SAFETY: SSE is baseline on x86-64 and a prefetch never faults; the pointer is
            // in-bounds anyway.
            unsafe { _mm_prefetch::<_MM_HINT_T0>(line.as_ptr().cast()) };
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = (rows, dim, slot);
}

/// Requests a slice's cache lines into L2.
#[inline]
pub(super) fn prefetch_l2(data: &[f32]) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T1};
        for line in data.chunks(16) {
            // SAFETY: SSE is baseline on x86-64 and a prefetch never faults.
            unsafe { _mm_prefetch::<_MM_HINT_T1>(line.as_ptr().cast()) };
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = data;
}

pub(super) fn sq(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

#[cfg(target_arch = "x86_64")]
mod avx {
    use std::arch::x86_64::*;

    use super::LANES;

    /// Eight rows against `q`; lane `c` of the result is candidate `c`'s distance.
    ///
    /// # Safety
    /// The CPU supports AVX; `q` holds `dim` floats, `out` has room for eight, and each of the
    /// slots in `s` addresses `dim` floats at `rows + s * dim`.
    #[target_feature(enable = "avx")]
    pub(super) unsafe fn sq8(
        q: *const f32,
        rows: *const f32,
        dim: usize,
        s: [u32; LANES],
        out: *mut f32,
    ) {
        let r: [*const f32; LANES] = s.map(|s| rows.add(s as usize * dim));
        let mut acc = _mm256_set1_ps(-0.0);
        let mut d = 0;
        while d + LANES <= dim {
            let mut t = [_mm256_setzero_ps(); LANES];
            for c in 0..LANES {
                t[c] = _mm256_loadu_ps(r[c].add(d));
            }
            let t = transpose(t);
            for (k, v) in t.into_iter().enumerate() {
                acc = step(acc, *q.add(d + k), v);
            }
            d += LANES;
        }
        while d < dim {
            let v = _mm256_set_ps(
                *r[7].add(d),
                *r[6].add(d),
                *r[5].add(d),
                *r[4].add(d),
                *r[3].add(d),
                *r[2].add(d),
                *r[1].add(d),
                *r[0].add(d),
            );
            acc = step(acc, *q.add(d), v);
            d += 1;
        }
        _mm256_storeu_ps(out, acc);
    }

    /// `acc + (q - v) * (q - v)` with two roundings, as the scalar `sq`.
    ///
    /// # Safety
    /// The CPU supports AVX.
    #[inline]
    #[target_feature(enable = "avx")]
    unsafe fn step(acc: __m256, q: f32, v: __m256) -> __m256 {
        let t = _mm256_sub_ps(_mm256_set1_ps(q), v);
        _mm256_add_ps(acc, _mm256_mul_ps(t, t))
    }

    /// 8x8 transpose of f32 lanes.
    ///
    /// # Safety
    /// The CPU supports AVX.
    #[inline]
    #[target_feature(enable = "avx")]
    unsafe fn transpose(r: [__m256; LANES]) -> [__m256; LANES] {
        let t0 = _mm256_unpacklo_ps(r[0], r[1]);
        let t1 = _mm256_unpackhi_ps(r[0], r[1]);
        let t2 = _mm256_unpacklo_ps(r[2], r[3]);
        let t3 = _mm256_unpackhi_ps(r[2], r[3]);
        let t4 = _mm256_unpacklo_ps(r[4], r[5]);
        let t5 = _mm256_unpackhi_ps(r[4], r[5]);
        let t6 = _mm256_unpacklo_ps(r[6], r[7]);
        let t7 = _mm256_unpackhi_ps(r[6], r[7]);
        let s0 = _mm256_shuffle_ps::<0x44>(t0, t2);
        let s1 = _mm256_shuffle_ps::<0xEE>(t0, t2);
        let s2 = _mm256_shuffle_ps::<0x44>(t1, t3);
        let s3 = _mm256_shuffle_ps::<0xEE>(t1, t3);
        let s4 = _mm256_shuffle_ps::<0x44>(t4, t6);
        let s5 = _mm256_shuffle_ps::<0xEE>(t4, t6);
        let s6 = _mm256_shuffle_ps::<0x44>(t5, t7);
        let s7 = _mm256_shuffle_ps::<0xEE>(t5, t7);
        [
            _mm256_permute2f128_ps::<0x20>(s0, s4),
            _mm256_permute2f128_ps::<0x20>(s1, s5),
            _mm256_permute2f128_ps::<0x20>(s2, s6),
            _mm256_permute2f128_ps::<0x20>(s3, s7),
            _mm256_permute2f128_ps::<0x31>(s0, s4),
            _mm256_permute2f128_ps::<0x31>(s1, s5),
            _mm256_permute2f128_ps::<0x31>(s2, s6),
            _mm256_permute2f128_ps::<0x31>(s3, s7),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*state >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 8.0
    }

    #[test]
    fn batch_distances_are_bit_identical_to_the_sequential_sum() {
        let mut st = 7u64;
        // Widths around the 8-lane step, candidate counts around the batch size.
        for dim in [1usize, 7, 8, 9, 15, 16, 100, 103] {
            let n = 40;
            let rows: Vec<f32> = (0..n * dim).map(|_| lcg(&mut st)).collect();
            let q: Vec<f32> = (0..dim).map(|_| lcg(&mut st)).collect();
            for count in [0usize, 1, 7, 8, 9, 17, 40] {
                let slots: Vec<u32> = (0..count as u32).map(|i| (i * 7 + 3) % n as u32).collect();
                let mut out = vec![0f32; count];
                sq_batch(&q, &rows, dim, &slots, &mut out);
                for (o, &s) in out.iter().zip(&slots) {
                    let want = sq(&q, &rows[s as usize * dim..(s as usize + 1) * dim]);
                    assert_eq!(o.to_bits(), want.to_bits(), "dim {dim} count {count}");
                }
            }
        }
    }
}
