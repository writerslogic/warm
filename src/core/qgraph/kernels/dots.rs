// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Integer dot product of two i8 rows whose length is a multiple of 16, dispatched inline so
/// the search loop makes no indirect call: SDOT when the build target enables `dotprod`
/// (every Apple core), else the kernel of the detected tier (one relaxed load, a plain move
/// on x86).
#[must_use]
#[inline(always)]
pub(crate) fn dot_i8_inline(a: &[i8], b: &[i8]) -> i32 {
    assert!(a.len() == b.len() && a.len().is_multiple_of(16));
    #[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
    {
        // SAFETY: `dotprod` is enabled for the whole build; the lengths are checked above.
        unsafe { neon::sdot(a, b) }
    }
    #[cfg(not(all(target_arch = "aarch64", target_feature = "dotprod")))]
    {
        #[cfg(target_arch = "aarch64")]
        {
            if std::arch::is_aarch64_feature_detected!("dotprod") {
                // SAFETY: `dotprod` was detected at runtime; the lengths are checked above.
                return unsafe { neon::sdot(a, b) };
            }
            neon::dot_i8(a, b)
        }
        #[cfg(target_arch = "x86_64")]
        match tier::get() {
            // SAFETY: the tier is VNNI only after detecting `avx512vnni`, `avx512vl` and `avx2`;
            // the lengths are checked above.
            tier::VNNI => unsafe { x86::dot_i8_vnni(a, b) },
            // SAFETY: the tier is AVX2 or AVXVNNI only after detecting `avx2`; the lengths are
            // checked above.
            tier::AVX2 | tier::AVXVNNI => unsafe { x86::dot_i8_avx2(a, b) },
            _ => dot_i8_scalar(a, b),
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        dot_i8_scalar(a, b)
    }
}

/// Lays out an i8 query code for [`dot_i4_inline`]: per 32-wide block, its 16 even then its
/// 16 odd coordinates. `q.len()` is a multiple of 32.
pub(crate) fn interleave_for_i4(q: &[i8], out: &mut [i8]) {
    assert!(q.len() == out.len() && q.len().is_multiple_of(32));
    for (src, dst) in q
        .as_chunks::<32>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<32>().0.iter_mut())
    {
        for j in 0..16 {
            dst[j] = src[2 * j];
            dst[16 + j] = src[2 * j + 1];
        }
    }
}

/// `sum_i q_i * c_i` for a query code laid out by [`interleave_for_i4`] and a row packed as
/// [`dot_f32_i4`] reads it; `q.len() == 2 * packed.len()`, a multiple of 32.
#[must_use]
#[inline(always)]
pub(crate) fn dot_i4_inline(q: &[i8], packed: &[i8]) -> i32 {
    assert!(q.len() == 2 * packed.len() && packed.len().is_multiple_of(16));
    #[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
    {
        // SAFETY: `dotprod` is enabled for the whole build; the lengths are checked above.
        unsafe { neon::sdot4(q, packed) }
    }
    #[cfg(not(all(target_arch = "aarch64", target_feature = "dotprod")))]
    {
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            // SAFETY: `dotprod` was detected at runtime; the slice lengths were checked above.
            return unsafe { neon::sdot4(q, packed) };
        }
        #[cfg(target_arch = "aarch64")]
        {
            neon::dot_i4(q, packed)
        }
        #[cfg(target_arch = "x86_64")]
        match tier::get() {
            // SAFETY: the tier is VNNI only after detecting `avx512vnni`, `avx512vl` and `avx2`;
            // the slice lengths were checked above.
            tier::VNNI => return unsafe { x86::dot_i4_vnni(q, packed) },
            // SAFETY: the tier is AVX2 or AVXVNNI only after detecting `avx2`.
            tier::AVX2 | tier::AVXVNNI => return unsafe { x86::dot_i4_avx2(q, packed) },
            _ => {}
        }
        #[cfg(not(target_arch = "aarch64"))]
        dot_i4_scalar(q, packed)
    }
}

/// [`dot_i4_inline`] of one query against four rows of equal length. On AVX2 the query's
/// widened halves are loaded once per 16 packed bytes and the four row loads are in flight
/// together; the sums are the same integers, so every result equals the single call's.
#[must_use]
#[inline(always)]
pub(crate) fn dot_i4_inline_x4(q: &[i8], rows: [&[i8]; 4]) -> [i32; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        let len = rows[0].len();
        if rows.iter().all(|r| r.len() == len)
            && q.len() == 2 * len
            && len.is_multiple_of(16)
            && tier::get() >= tier::AVX2
        {
            // SAFETY: the tier is AVX2 or VNNI only after detecting `avx2`; the lengths were
            // checked above.
            return unsafe { x86::dot_i4_avx2_x4(q, rows) };
        }
    }
    [
        dot_i4_inline(q, rows[0]),
        dot_i4_inline(q, rows[1]),
        dot_i4_inline(q, rows[2]),
        dot_i4_inline(q, rows[3]),
    ]
}

/// [`dot_i8_inline`] for a traversal query `a` with `qsum == super::super::vnni::query_sum(a)`,
/// computed once per query: the VNNI tiers then spend one `vpdpbusd` per 32 coordinates.
/// The same integer on every tier, widening partial sums before a long row can overflow i32.
#[must_use]
#[inline(always)]
pub(crate) fn dot_i8_query(a: &[i8], b: &[i8], qsum: i32) -> i64 {
    if a.len() > i32::MAX as usize / (128 * 128) {
        return dot_i8_long(a, b);
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert!(a.len() == b.len() && a.len().is_multiple_of(16));
        match tier::get() {
            // SAFETY: the tier is AVXVNNI only after detecting `avx2` and `avxvnni`, VNNI only
            // after `avx2`, `avx512f`, `avx512vl` and `avx512vnni`; the lengths are checked.
            tier::AVXVNNI => {
                return i64::from(unsafe { super::super::vnni::avx::dot_i8(a, b, qsum) })
            }
            // SAFETY: as above.
            tier::VNNI => {
                return i64::from(unsafe { super::super::vnni::evex::dot_i8(a, b, qsum) })
            }
            _ => {}
        }
    }
    let _ = qsum;
    i64::from(dot_i8_inline(a, b))
}

#[cold]
pub(super) fn dot_i8_long(a: &[i8], b: &[i8]) -> i64 {
    assert!(a.len() == b.len() && a.len().is_multiple_of(16));
    // Even (-128)*(-128) stays in i32 within a chunk; bias correction may wrap,
    // but the signed chunk result cannot. Widen before adding chunks together.
    a.chunks(1 << 16)
        .zip(b.chunks(1 << 16))
        .map(|(q, row)| dot_i8_query(q, row, super::super::vnni::query_sum(q)))
        .sum()
}

/// [`dot_i8_query`] of one row `a` against four rows of its length: `a` is loaded once per
/// chunk for all four, and the four row loads are in flight together. The same integers as
/// four single calls, on every tier.
#[must_use]
#[inline(always)]
pub(crate) fn dot_i8_query_x4(a: &[i8], rows: [&[i8]; 4], qsum: i32) -> [i64; 4] {
    assert!(a.len().is_multiple_of(16) && rows.iter().all(|r| r.len() == a.len()));
    if a.len() > i32::MAX as usize / (128 * 128) {
        return rows.map(|row| dot_i8_long(a, row));
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
    {
        let _ = qsum;
        // SAFETY: `dotprod` is enabled for the whole build; the lengths are checked above.
        unsafe { neon::sdot_x4(a, rows) }.map(i64::from)
    }
    #[cfg(not(all(target_arch = "aarch64", target_feature = "dotprod")))]
    {
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            // SAFETY: `dotprod` was detected at runtime; the lengths are checked above.
            return unsafe { neon::sdot_x4(a, rows) }.map(i64::from);
        }
        #[cfg(target_arch = "x86_64")]
        match tier::get() {
            // SAFETY: the tier is AVXVNNI only after detecting `avx2` and `avxvnni`; the
            // lengths are checked above.
            tier::AVXVNNI => {
                return unsafe { super::super::vnni::avx::dot_i8_x4(a, rows, qsum) }.map(i64::from)
            }
            // SAFETY: the tier is VNNI only after detecting `avx2`, `avx512f`, `avx512vl` and
            // `avx512vnni`; the lengths are checked above.
            tier::VNNI => {
                return unsafe { super::super::vnni::evex::dot_i8_x4(a, rows, qsum) }.map(i64::from)
            }
            // SAFETY: the tier is AVX2 only after detecting `avx2`; the lengths are checked.
            tier::AVX2 => return unsafe { x86::dot_i8_avx2_x4(a, rows) }.map(i64::from),
            _ => {}
        }
        let _ = qsum;
        rows.map(|r| i64::from(dot_i8_inline(a, r)))
    }
}

/// [`dot_i8_query_x4`] with 512-bit `vpdpbusd` on the VNNI tier; the same integers on every
/// tier (other tiers run [`dot_i8_query_x4`]).
#[must_use]
#[inline(always)]
pub(crate) fn dot_i8_query_x4_wide(a: &[i8], rows: [&[i8]; 4], qsum: i32) -> [i64; 4] {
    if a.len() > i32::MAX as usize / (128 * 128) {
        return dot_i8_query_x4(a, rows, qsum);
    }
    #[cfg(target_arch = "x86_64")]
    if tier::get() == tier::VNNI {
        assert!(a.len().is_multiple_of(16) && rows.iter().all(|r| r.len() == a.len()));
        // SAFETY: the tier is VNNI only after detecting `avx2`, `avx512f`, `avx512vl` and
        // `avx512vnni`; the lengths are checked above.
        return unsafe { super::super::vnni::zmm::dot_i8_x4(a, rows, qsum) }.map(i64::from);
    }
    dot_i8_query_x4(a, rows, qsum)
}

/// [`dot_i4_inline`] with the query sum precomputed, as [`dot_i8_query`].
#[must_use]
#[inline(always)]
pub(crate) fn dot_i4_query(q: &[i8], packed: &[i8], qsum: i32) -> i32 {
    #[cfg(target_arch = "x86_64")]
    {
        assert!(q.len() == 2 * packed.len() && packed.len().is_multiple_of(16));
        match tier::get() {
            // SAFETY: as in `dot_i8_query`; the lengths are checked above.
            tier::AVXVNNI => return unsafe { super::super::vnni::avx::dot_i4(q, packed, qsum) },
            // SAFETY: as above.
            tier::VNNI => return unsafe { super::super::vnni::evex::dot_i4(q, packed, qsum) },
            _ => {}
        }
    }
    let _ = qsum;
    dot_i4_inline(q, packed)
}

/// [`dot_i4_inline_x4`] with the query sum precomputed, as [`dot_i8_query`].
#[must_use]
#[inline(always)]
pub(crate) fn dot_i4_query_x4(q: &[i8], rows: [&[i8]; 4], qsum: i32) -> [i32; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        let len = rows[0].len();
        if rows.iter().all(|r| r.len() == len) && q.len() == 2 * len && len.is_multiple_of(16) {
            match tier::get() {
                // SAFETY: as in `dot_i8_query`; the lengths were checked above.
                tier::AVXVNNI => {
                    return unsafe { super::super::vnni::avx::dot_i4_x4(q, rows, qsum) }
                }
                // SAFETY: as above.
                tier::VNNI => return unsafe { super::super::vnni::evex::dot_i4_x4(q, rows, qsum) },
                _ => {}
            }
        }
    }
    let _ = qsum;
    dot_i4_inline_x4(q, rows)
}

#[cfg(any(test, not(all(target_arch = "aarch64", target_feature = "dotprod"))))]
pub(super) fn dot_i4_scalar(q: &[i8], packed: &[i8]) -> i32 {
    let mut s = 0i32;
    for (qb, pb) in q
        .as_chunks::<32>()
        .0
        .iter()
        .zip(packed.as_chunks::<16>().0.iter())
    {
        for (j, &x) in pb.iter().enumerate() {
            let lo = (((x as u8) << 4) as i8) >> 4;
            s += i32::from(qb[j]) * i32::from(lo) + i32::from(qb[16 + j]) * i32::from(x >> 4);
        }
    }
    s
}

/// `sum_i a_i * c_i` where `c` is packed two signed 4-bit values per byte, element `2j` in the
/// low nibble of byte `j` and `2j + 1` in the high nibble; `a.len() == 2 * packed.len()`.
#[must_use]
pub(crate) fn dot_f32_i4(a: &[f32], packed: &[i8]) -> f32 {
    debug_assert_eq!(a.len(), 2 * packed.len());
    let mut acc = [0f32; 16];
    let ((ca, ta), (cb, tb)) = (a.as_chunks::<16>(), packed.as_chunks::<8>());
    debug_assert!(
        ta.is_empty() && tb.is_empty(),
        "length not a multiple of 16"
    );
    for (x, y) in ca.iter().zip(cb) {
        for (j, &b) in y.iter().enumerate() {
            let lo = f32::from((((b as u8) << 4) as i8) >> 4);
            let hi = f32::from(b >> 4);
            acc[2 * j] += x[2 * j] * lo;
            acc[2 * j + 1] += x[2 * j + 1] * hi;
        }
    }
    acc.iter().sum::<f32>()
}

#[cfg(any(test, not(target_arch = "aarch64")))]
pub(super) fn dot_i8_scalar(a: &[i8], b: &[i8]) -> i32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(&x, &y)| i32::from(x) * i32::from(y))
        .sum()
}

/// `sum_i a_i * b_i` for a float row and an i8 row of the same length.
#[must_use]
pub(crate) fn dot_f32_i8(a: &[f32], b: &[i8]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = [0f32; 16];
    let ((ca, ta), (cb, tb)) = (a.as_chunks::<16>(), b.as_chunks::<16>());
    let tail: f32 = ta.iter().zip(tb).map(|(x, &y)| x * f32::from(y)).sum();
    for (x, y) in ca.iter().zip(cb) {
        for ((s, p), &q) in acc.iter_mut().zip(x).zip(y) {
            *s += p * f32::from(q);
        }
    }
    acc.iter().sum::<f32>() + tail
}
