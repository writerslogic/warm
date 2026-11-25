// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Branch-free argmin of `c2[c] - 2 * dot[c]` over a tile of centres, lowest id on ties.
//!
//! The vector kernels keep 16 per-lane running minima (lane `j` sees centres `j, j + 16, ..`,
//! updated on strict `<`), then reduce the lanes by (value, id). That equals the scalar
//! first-minimum scan: the value is `c2 - 2 * v` in one rounding either way (`2 * v` is exact).
//! Dispatch is by runtime CPU detection; the scalar loop is the fallback and the reference.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Argmin {
    Scalar,
    #[cfg(target_arch = "x86_64")]
    Avx2,
    #[cfg(target_arch = "x86_64")]
    Avx512,
}

impl Argmin {
    /// Widest kernel this CPU runs.
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx512f") {
                return Self::Avx512;
            }
            if is_x86_feature_detected!("avx2") {
                return Self::Avx2;
            }
        }
        Self::Scalar
    }
}

/// Folds each of the `b` rows of `dots` (`b x w`, row-major) over the centre tile starting at
/// global id `base` into the running `(best_d, best_i)`; ids below `base` already won ties.
pub(super) fn scan_tile(
    kernel: Argmin,
    dots: &[f32],
    w: usize,
    c2: &[f32],
    base: u32,
    best_d: &mut [f32],
    best_i: &mut [u32],
) {
    debug_assert!(c2.len() == w && dots.len() >= best_d.len() * w);
    match kernel {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: a kernel is only requested after `detect` (or the caller's own check) saw it.
        Argmin::Avx512 => unsafe { x86::tile_avx512(dots, w, c2, base, best_d, best_i) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: as above.
        Argmin::Avx2 => unsafe { x86::tile_avx2(dots, w, c2, base, best_d, best_i) },
        _ => {
            for (i, (bd, bi)) in best_d.iter_mut().zip(best_i.iter_mut()).enumerate() {
                scalar_row(&dots[i * w..(i + 1) * w], c2, base, bd, bi);
            }
        }
    }
}

#[inline(always)]
fn scalar_row(row: &[f32], c2: &[f32], base: u32, bd: &mut f32, bi: &mut u32) {
    for (c, (&v, &cc)) in row.iter().zip(c2).enumerate() {
        let d = cc - 2.0 * v;
        if d < *bd {
            *bd = d;
            *bi = base + c as u32;
        }
    }
}

/// Lowest-id lane minimum, ties by id; `None` when no lane is below `bd`.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn reduce_lanes(d: &[f32; 16], id: &[u32; 16], bd: &mut f32, bi: &mut u32) {
    let (mut m, mut mi) = (f32::INFINITY, u32::MAX);
    for l in 0..16 {
        if d[l] < m || (d[l] == m && id[l] < mi) {
            m = d[l];
            mi = id[l];
        }
    }
    if m < *bd {
        *bd = m;
        *bi = mi;
    }
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::{reduce_lanes, scalar_row};
    use std::arch::x86_64::*;

    #[target_feature(enable = "avx512f")]
    pub(super) unsafe fn tile_avx512(
        dots: &[f32],
        w: usize,
        c2: &[f32],
        base: u32,
        best_d: &mut [f32],
        best_i: &mut [u32],
    ) {
        let full = w / 16 * 16;
        let two = _mm512_set1_ps(2.0);
        let lane0 = _mm512_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
        for (i, (bd, bi)) in best_d.iter_mut().zip(best_i.iter_mut()).enumerate() {
            let row = &dots[i * w..(i + 1) * w];
            let mut vd = _mm512_set1_ps(f32::INFINITY);
            let mut vi = _mm512_setzero_si512();
            let mut ids = _mm512_add_epi32(lane0, _mm512_set1_epi32(base as i32));
            let step = _mm512_set1_epi32(16);
            let mut c = 0;
            while c < full {
                let v = _mm512_loadu_ps(row.as_ptr().add(c));
                let cc = _mm512_loadu_ps(c2.as_ptr().add(c));
                let d = _mm512_sub_ps(cc, _mm512_mul_ps(two, v));
                let m = _mm512_cmp_ps_mask::<_CMP_LT_OQ>(d, vd);
                vd = _mm512_mask_mov_ps(vd, m, d);
                vi = _mm512_mask_mov_epi32(vi, m, ids);
                ids = _mm512_add_epi32(ids, step);
                c += 16;
            }
            let mut d = [0f32; 16];
            let mut id = [0u32; 16];
            _mm512_storeu_ps(d.as_mut_ptr(), vd);
            _mm512_storeu_si512(id.as_mut_ptr().cast(), vi);
            reduce_lanes(&d, &id, bd, bi);
            scalar_row(&row[full..], &c2[full..], base + full as u32, bd, bi);
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn tile_avx2(
        dots: &[f32],
        w: usize,
        c2: &[f32],
        base: u32,
        best_d: &mut [f32],
        best_i: &mut [u32],
    ) {
        let full = w / 16 * 16;
        let two = _mm256_set1_ps(2.0);
        let lane0 = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
        for (i, (bd, bi)) in best_d.iter_mut().zip(best_i.iter_mut()).enumerate() {
            let row = &dots[i * w..(i + 1) * w];
            let inf = _mm256_set1_ps(f32::INFINITY);
            let (mut d0, mut d1) = (inf, inf);
            let (mut i0, mut i1) = (_mm256_setzero_si256(), _mm256_setzero_si256());
            let mut id0 = _mm256_add_epi32(lane0, _mm256_set1_epi32(base as i32));
            let mut id1 = _mm256_add_epi32(id0, _mm256_set1_epi32(8));
            let step = _mm256_set1_epi32(16);
            let mut c = 0;
            while c < full {
                let p = row.as_ptr().add(c);
                let q = c2.as_ptr().add(c);
                let x0 = _mm256_sub_ps(_mm256_loadu_ps(q), _mm256_mul_ps(two, _mm256_loadu_ps(p)));
                let x1 = _mm256_sub_ps(
                    _mm256_loadu_ps(q.add(8)),
                    _mm256_mul_ps(two, _mm256_loadu_ps(p.add(8))),
                );
                let m0 = _mm256_cmp_ps::<_CMP_LT_OQ>(x0, d0);
                let m1 = _mm256_cmp_ps::<_CMP_LT_OQ>(x1, d1);
                d0 = _mm256_blendv_ps(d0, x0, m0);
                d1 = _mm256_blendv_ps(d1, x1, m1);
                i0 = _mm256_blendv_epi8(i0, id0, _mm256_castps_si256(m0));
                i1 = _mm256_blendv_epi8(i1, id1, _mm256_castps_si256(m1));
                id0 = _mm256_add_epi32(id0, step);
                id1 = _mm256_add_epi32(id1, step);
                c += 16;
            }
            let mut d = [0f32; 16];
            let mut id = [0u32; 16];
            _mm256_storeu_ps(d.as_mut_ptr(), d0);
            _mm256_storeu_ps(d.as_mut_ptr().add(8), d1);
            _mm256_storeu_si256(id.as_mut_ptr().cast(), i0);
            _mm256_storeu_si256(id.as_mut_ptr().add(8).cast(), i1);
            reduce_lanes(&d, &id, bd, bi);
            scalar_row(&row[full..], &c2[full..], base + full as u32, bd, bi);
        }
    }
}
