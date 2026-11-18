// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use std::arch::aarch64::*;

use super::{scalar_word, QUERY_BITS};

/// [`sdot`] behind a length check, for the kernel tests (after detecting `dotprod`).
#[cfg(test)]
pub(super) fn dot_i8_sdot(a: &[i8], b: &[i8]) -> i32 {
    assert!(a.len() == b.len() && a.len().is_multiple_of(16));
    // SAFETY: the only caller detected `dotprod` at runtime; lengths checked above.
    unsafe { sdot(a, b) }
}

/// # Safety
/// The CPU supports `dotprod`; `a` and `b` have the same length, a multiple of 16.
#[target_feature(enable = "dotprod")]
#[inline]
pub(super) unsafe fn sdot(a: &[i8], b: &[i8]) -> i32 {
    let (mut s0, mut s1) = (vdupq_n_s32(0), vdupq_n_s32(0));
    let mut i = 0;
    // SAFETY (all loads): `i + 16 <= len` (or `i + 32 <= len`) for both slices, whose
    // lengths are equal and a multiple of 16 (asserted by the caller).
    while i + 32 <= a.len() {
        let (x0, y0) = (vld1q_s8(a.as_ptr().add(i)), vld1q_s8(b.as_ptr().add(i)));
        let (x1, y1) = (
            vld1q_s8(a.as_ptr().add(i + 16)),
            vld1q_s8(b.as_ptr().add(i + 16)),
        );
        std::arch::asm!(
            "sdot {s0:v}.4s, {x0:v}.16b, {y0:v}.16b",
            "sdot {s1:v}.4s, {x1:v}.16b, {y1:v}.16b",
            s0 = inout(vreg) s0, s1 = inout(vreg) s1,
            x0 = in(vreg) x0, y0 = in(vreg) y0, x1 = in(vreg) x1, y1 = in(vreg) y1,
            options(pure, nomem, nostack, preserves_flags)
        );
        i += 32;
    }
    if i < a.len() {
        let (x0, y0) = (vld1q_s8(a.as_ptr().add(i)), vld1q_s8(b.as_ptr().add(i)));
        std::arch::asm!(
            "sdot {s0:v}.4s, {x0:v}.16b, {y0:v}.16b",
            s0 = inout(vreg) s0, x0 = in(vreg) x0, y0 = in(vreg) y0,
            options(pure, nomem, nostack, preserves_flags)
        );
    }
    vaddvq_s32(vaddq_s32(s0, s1))
}

/// [`sdot`] of one row `a` against four rows, `a` loaded once per 16 bytes and one
/// accumulator per row. The same integer sums as four single calls.
///
/// # Safety
/// The CPU supports `dotprod`; `a` and every row have the same length, a multiple of 16.
#[target_feature(enable = "dotprod")]
#[inline]
pub(super) unsafe fn sdot_x4(a: &[i8], rows: [&[i8]; 4]) -> [i32; 4] {
    let mut s = [vdupq_n_s32(0); 4];
    let mut i = 0;
    // SAFETY (all loads): `i + 16 <= len` for `a` and every row, whose lengths are equal and
    // a multiple of 16 (asserted by the caller).
    while i + 16 <= a.len() {
        let x = vld1q_s8(a.as_ptr().add(i));
        let y = rows.map(|r| vld1q_s8(r.as_ptr().add(i)));
        let [mut s0, mut s1, mut s2, mut s3] = s;
        std::arch::asm!(
            "sdot {s0:v}.4s, {x:v}.16b, {y0:v}.16b",
            "sdot {s1:v}.4s, {x:v}.16b, {y1:v}.16b",
            "sdot {s2:v}.4s, {x:v}.16b, {y2:v}.16b",
            "sdot {s3:v}.4s, {x:v}.16b, {y3:v}.16b",
            s0 = inout(vreg) s0, s1 = inout(vreg) s1, s2 = inout(vreg) s2, s3 = inout(vreg) s3,
            x = in(vreg) x, y0 = in(vreg) y[0], y1 = in(vreg) y[1], y2 = in(vreg) y[2],
            y3 = in(vreg) y[3],
            options(pure, nomem, nostack, preserves_flags)
        );
        s = [s0, s1, s2, s3];
        i += 16;
    }
    s.map(|v| vaddvq_s32(v))
}

/// [`super::dot_i4_inline`] with SDOT: each 16 packed bytes become the 16 even and the 16 odd
/// coordinates of a 32-wide block, matched with the query laid out by
/// [`super::interleave_for_i4`].
///
/// # Safety
/// The CPU supports `dotprod`; `q.len() == 2 * p.len()` and `p.len()` is a multiple of 16.
#[target_feature(enable = "dotprod")]
#[inline]
pub(super) unsafe fn sdot4(q: &[i8], p: &[i8]) -> i32 {
    let (mut s0, mut s1) = (vdupq_n_s32(0), vdupq_n_s32(0));
    let mut i = 0;
    // SAFETY (all loads): the caller checked `q.len() == 2 * p.len()` and `p.len() % 16 == 0`,
    // so `i + 16 <= p.len()` and `2 * i + 32 <= q.len()`.
    while i + 16 <= p.len() {
        let x = vld1q_s8(p.as_ptr().add(i));
        let lo = vshrq_n_s8::<4>(vshlq_n_s8::<4>(x));
        let hi = vshrq_n_s8::<4>(x);
        let (qe, qo) = (
            vld1q_s8(q.as_ptr().add(2 * i)),
            vld1q_s8(q.as_ptr().add(2 * i + 16)),
        );
        std::arch::asm!(
            "sdot {s0:v}.4s, {lo:v}.16b, {qe:v}.16b",
            "sdot {s1:v}.4s, {hi:v}.16b, {qo:v}.16b",
            s0 = inout(vreg) s0, s1 = inout(vreg) s1,
            lo = in(vreg) lo, qe = in(vreg) qe, hi = in(vreg) hi, qo = in(vreg) qo,
            options(pure, nomem, nostack, preserves_flags)
        );
        i += 16;
    }
    vaddvq_s32(vaddq_s32(s0, s1))
}

/// [`sdot4`] with widening multiplies; for cores without `dotprod`. Each i16 lane holds one
/// product (|c * q| <= 8 * 128) and the pairwise adds go to i32.
#[cfg_attr(target_feature = "dotprod", allow(dead_code))]
pub(super) fn dot_i4(q: &[i8], p: &[i8]) -> i32 {
    assert!(q.len() == 2 * p.len() && p.len().is_multiple_of(16));
    let mut i = 0;
    // SAFETY: NEON is part of the aarch64 baseline; `i + 16 <= p.len()` and
    // `2 * i + 32 <= q.len()` by the assert above.
    unsafe {
        let mut s = vdupq_n_s32(0);
        while i < p.len() {
            let x = vld1q_s8(p.as_ptr().add(i));
            let lo = vshrq_n_s8::<4>(vshlq_n_s8::<4>(x));
            let hi = vshrq_n_s8::<4>(x);
            let (qe, qo) = (
                vld1q_s8(q.as_ptr().add(2 * i)),
                vld1q_s8(q.as_ptr().add(2 * i + 16)),
            );
            s = vpadalq_s16(s, vmull_s8(vget_low_s8(lo), vget_low_s8(qe)));
            s = vpadalq_s16(s, vmull_high_s8(lo, qe));
            s = vpadalq_s16(s, vmull_s8(vget_low_s8(hi), vget_low_s8(qo)));
            s = vpadalq_s16(s, vmull_high_s8(hi, qo));
            i += 16;
        }
        vaddvq_s32(s)
    }
}

/// [`super::dot_i8_inline`] with widening multiplies; for cores without `dotprod`.
#[cfg_attr(target_feature = "dotprod", allow(dead_code))]
pub(super) fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    assert!(a.len() == b.len() && a.len().is_multiple_of(16));
    let mut i = 0;
    // SAFETY: NEON is part of the aarch64 baseline; `i + 16 <= len` for both slices. Each
    // i16 lane holds one product (|x| <= 128 * 128), and the pairwise adds go to i32.
    unsafe {
        let mut s = vdupq_n_s32(0);
        while i < a.len() {
            let (x, y) = (vld1q_s8(a.as_ptr().add(i)), vld1q_s8(b.as_ptr().add(i)));
            s = vpadalq_s16(s, vmull_s8(vget_low_s8(x), vget_low_s8(y)));
            s = vpadalq_s16(s, vmull_high_s8(x, y));
            i += 16;
        }
        vaddvq_s32(s)
    }
}

pub(super) fn raw<const W: usize>(codes: &[u64], planes: &[u64], out: &mut [u32]) {
    assert_eq!(QUERY_BITS, 4);
    assert_eq!(planes.len(), 4 * W);
    assert_eq!(codes.len(), out.len() * W);
    for (code, o) in codes.as_chunks::<W>().0.iter().zip(out.iter_mut()) {
        let mut total = 0u32;
        let mut k = 0;
        while k + 2 <= W {
            // SAFETY: NEON is part of the aarch64 baseline. `k + 2 <= W` keeps each 16-byte
            // load inside `code` (length W) and inside plane `p` (planes[p*W..p*W+W], length
            // checked by the assert above). Per byte the weighted sum is at most
            // 8 * (1 + 2 + 4 + 8) = 120, so the u8 additions cannot overflow.
            total += unsafe {
                let x = vreinterpretq_u8_u64(vld1q_u64(code.as_ptr().add(k)));
                let p0 = vreinterpretq_u8_u64(vld1q_u64(planes.as_ptr().add(k)));
                let p1 = vreinterpretq_u8_u64(vld1q_u64(planes.as_ptr().add(W + k)));
                let p2 = vreinterpretq_u8_u64(vld1q_u64(planes.as_ptr().add(2 * W + k)));
                let p3 = vreinterpretq_u8_u64(vld1q_u64(planes.as_ptr().add(3 * W + k)));
                let c0 = vcntq_u8(vandq_u8(x, p0));
                let c1 = vcntq_u8(vandq_u8(x, p1));
                let c2 = vcntq_u8(vandq_u8(x, p2));
                let c3 = vcntq_u8(vandq_u8(x, p3));
                let s = vaddq_u8(
                    vaddq_u8(c0, vshlq_n_u8::<1>(c1)),
                    vaddq_u8(vshlq_n_u8::<2>(c2), vshlq_n_u8::<3>(c3)),
                );
                u32::from(vaddlvq_u8(s))
            };
            k += 2;
        }
        if W % 2 == 1 {
            total += scalar_word(code[W - 1], planes, W, W - 1);
        }
        *o = total;
    }
}
