// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Batched 4-bit lookup-table scan over a block of 32 codes (the FastScan form of André,
//! Kermarrec and Le Scouarnec, 2015, as used by FAISS, RaBitQ and SymphonyQG).
//!
//! A block holds `m` groups of 16 bytes. Byte `i` of group `j` carries nibble `j` of code
//! `i` in its low half and of code `i + 16` in its high half, so one 16-byte load and two
//! table lookups score nibble `j` of all 32 codes. A query is a table of `m * 16` bytes: entry
//! `16 * j + v` is the contribution of nibble value `v` at position `j`. Each code's score is
//! the sum of its `m` entries.

/// Codes per block.
pub(crate) const BLOCK: usize = 32;

/// Writes `out[c] = sum_j lut[16 * j + nibble_j(c)]` for the 32 codes of `codes`
/// (`16 * m` bytes) and the table `lut` (`16 * m` bytes). Any `m` is allowed; sums are exact
/// in `u32`.
#[inline]
pub(crate) fn scan32(codes: &[u8], lut: &[u8], m: usize, out: &mut [u32; BLOCK]) {
    assert!(codes.len() >= 16 * m && lut.len() >= 16 * m);
    // At most 256 groups of at most 255 fit a u16 lane without overflow: one pass, widened
    // once (every index's case), or 256-group passes summed in u32.
    if m <= 256 {
        let mut part = [0u16; BLOCK];
        scan32_u16(&codes[..16 * m], &lut[..16 * m], m, &mut part);
        for (o, p) in out.iter_mut().zip(part) {
            *o = u32::from(p);
        }
        return;
    }
    out.fill(0);
    for (c, l) in codes[..16 * m]
        .chunks(16 * 256)
        .zip(lut[..16 * m].chunks(16 * 256))
    {
        let mut part = [0u16; BLOCK];
        scan32_u16(c, l, c.len() / 16, &mut part);
        for (o, p) in out.iter_mut().zip(part) {
            *o += u32::from(p);
        }
    }
}

/// [`scan32`] plus the admission test: returns a bit mask with bit `c` set when code `c`'s sum is
/// below `cut`. `out` holds the sums when the mask is non-zero and is unspecified otherwise, so
/// a caller that admits almost nothing never widens or stores them.
#[inline]
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
pub(crate) fn scan32_lt(
    codes: &[u8],
    lut: &[u8],
    m: usize,
    cut: u32,
    out: &mut [u32; BLOCK],
) -> u32 {
    assert!(codes.len() >= 16 * m && lut.len() >= 16 * m);
    // The largest u16 sum is 255 * 256 = 65280, so a cut of 65535 admits every code.
    #[cfg(target_arch = "x86_64")]
    if m <= 256 {
        let cut16 = cut.min(65535) as u16;
        if use_avx512() {
            // SAFETY: AVX-512F/BW/VL and AVX2 detected; lengths as above.
            return unsafe { avx512::scan32_lt(&codes[..16 * m], &lut[..16 * m], m, cut16, out) };
        }
        if is_x86_feature_detected!("avx2") {
            // SAFETY: AVX2 detected; lengths as above.
            return unsafe { avx2::scan32_lt(&codes[..16 * m], &lut[..16 * m], m, cut16, out) };
        }
    }
    scan32(codes, lut, m, out);
    out.iter()
        .enumerate()
        .fold(0u32, |mask, (c, &v)| mask | (u32::from(v < cut) << c))
}

/// [`scan32`] for `m <= 256` groups, accumulating in `u16`.
#[inline]
fn scan32_u16(codes: &[u8], lut: &[u8], m: usize, out: &mut [u16; BLOCK]) {
    debug_assert!(m <= 256);
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is part of the aarch64 baseline; `codes` and `lut` hold at least
        // `16 * m` bytes (checked by the public caller).
        unsafe { neon::scan32(codes, lut, m, out) }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if use_avx512() {
            // SAFETY: AVX-512F/BW and AVX2 detected; lengths as above.
            unsafe { avx512::scan32(codes, lut, m, out) };
            return;
        }
        if is_x86_feature_detected!("avx2") {
            // SAFETY: AVX2 detected; lengths as above.
            unsafe { avx2::scan32(codes, lut, m, out) };
            return;
        }
        scan32_scalar(codes, lut, m, out);
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    scan32_scalar(codes, lut, m, out);
}

/// True when the AVX-512BW scan runs. `WARM_FASTSCAN_NO_AVX512` (any value) forces the AVX2
/// path, for A/B timing; it is read once.
#[cfg(target_arch = "x86_64")]
fn use_avx512() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        avx512_available() && crate::compatibility::var_os("WARM_FASTSCAN_NO_AVX512").is_none()
    })
}

/// True when `WARM_FASTSCAN_SCALAR_LUT` (any value) forces the scalar table build, for A/B
/// timing; it is read once.
#[cfg(target_arch = "x86_64")]
fn scalar_lut() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| crate::compatibility::var_os("WARM_FASTSCAN_SCALAR_LUT").is_some())
}

#[cfg(target_arch = "x86_64")]
fn avx512_available() -> bool {
    is_x86_feature_detected!("avx512f")
        && is_x86_feature_detected!("avx512bw")
        && is_x86_feature_detected!("avx512vl")
        && is_x86_feature_detected!("avx2")
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn scan32_scalar(codes: &[u8], lut: &[u8], m: usize, out: &mut [u16; BLOCK]) {
    out.fill(0);
    for j in 0..m {
        add_group(&codes[16 * j..16 * j + 16], &lut[16 * j..16 * j + 16], out);
    }
}

/// Adds the entries of one group (16 code bytes, 16 table bytes) to the 32 sums.
#[inline]
fn add_group(codes: &[u8], lut: &[u8], out: &mut [u16; BLOCK]) {
    for i in 0..16 {
        out[i] += u16::from(lut[(codes[i] & 15) as usize]);
        out[i + 16] += u16::from(lut[(codes[i] >> 4) as usize]);
    }
}

/// Bytes per block of 32 codes in the quad layout used by [`scan32q_lt`].
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
pub(crate) fn quad_stride(m: usize) -> usize {
    64 * m.div_ceil(4)
}

/// Byte index of code position `i`'s nibble `j` within a quad-layout block; the nibble sits in
/// the low half for `i < 16` and in the high half otherwise. Four groups share 64 bytes, and
/// byte `4 * (i % 16) + g` of a quad holds group `g` of codes `i % 16` and `i % 16 + 16`, so a
/// dword gathers one code's four lookups.
#[inline]
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
pub(crate) fn quad_pos(i: usize, j: usize) -> usize {
    64 * (j / 4) + 4 * (i & 15) + (j & 3)
}

/// True when the quad kernel runs: AVX-512 VBMI and VNNI on top of the BW path, and
/// `WARM_FASTSCAN_NO_QUAD` unset (read once).
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
pub(crate) fn quad_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            avx512_available()
                && is_x86_feature_detected!("avx512vbmi")
                && is_x86_feature_detected!("avx512vnni")
                && crate::compatibility::var_os("WARM_FASTSCAN_NO_QUAD").is_none()
        })
    }
    #[cfg(not(target_arch = "x86_64"))]
    false
}

/// Scores one quad-layout block (`quad_stride(m)` bytes) against `Q` tables at once, loading and
/// unpacking the codes a single time. Table `k` starts at `luts[sel[k] * lstride]` and holds
/// `quad_stride(m)` bytes (groups beyond `m` zero). Returns per table the mask of codes whose
/// sum is below `cuts[k]`; `out[k]` holds that table's sums only when its mask is non-zero.
/// Sums are exact in `u32` for any `m`. Requires [`quad_available`].
#[inline]
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
pub(crate) fn scan32q_lt<const Q: usize>(
    codes: &[u8],
    luts: &[u8],
    lstride: usize,
    m: usize,
    sel: [usize; Q],
    cuts: [u32; Q],
    out: &mut [[u32; BLOCK]; Q],
) -> [u32; Q] {
    let nq = m.div_ceil(4);
    assert!(codes.len() >= 64 * nq && lstride >= 64 * nq);
    for &t in &sel {
        assert!(t * lstride + 64 * nq <= luts.len());
    }
    quad_dispatch::<Q>(codes, luts, lstride, sel, nq, cuts, out)
}

#[cfg(target_arch = "x86_64")]
#[inline]
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
fn quad_dispatch<const Q: usize>(
    codes: &[u8],
    luts: &[u8],
    lstride: usize,
    sel: [usize; Q],
    nq: usize,
    cuts: [u32; Q],
    out: &mut [[u32; BLOCK]; Q],
) -> [u32; Q] {
    assert!(quad_available());
    // SAFETY: AVX-512 F/BW/VL/VBMI/VNNI detected; the slices hold the bytes read (asserted by
    // the caller).
    unsafe { quad::scan::<Q>(codes.as_ptr(), luts.as_ptr(), lstride, sel, nq, cuts, out) }
}

#[cfg(not(target_arch = "x86_64"))]
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
fn quad_dispatch<const Q: usize>(
    _codes: &[u8],
    _luts: &[u8],
    _lstride: usize,
    _sel: [usize; Q],
    _nq: usize,
    _cuts: [u32; Q],
    _out: &mut [[u32; BLOCK]; Q],
) -> [u32; Q] {
    unreachable!("the quad layout needs x86-64 AVX-512")
}

#[cfg(target_arch = "x86_64")]
mod quad {
    use std::arch::x86_64::*;

    use super::BLOCK;

    /// `vpermb` looks each code's four nibbles up in the 64-byte table of a group quad (index
    /// `16 * g + nibble`), and `vpdpbusd` adds the four bytes of every dword into that code's
    /// running sum: two ALU uops per 16 codes per quad, with the codes unpacked once for all `Q`
    /// tables.
    ///
    /// # Safety
    /// The CPU supports AVX-512 F/BW/VL/VBMI/VNNI; `codes` holds `64 * nq` bytes and table `k`
    /// holds `64 * nq` bytes at `luts + sel[k] * lstride`.
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,avx512vbmi,avx512vnni")]
    #[cfg_attr(not(feature = "experimental"), allow(dead_code))]
    pub(super) unsafe fn scan<const Q: usize>(
        codes: *const u8,
        luts: *const u8,
        lstride: usize,
        sel: [usize; Q],
        nq: usize,
        cuts: [u32; Q],
        out: &mut [[u32; BLOCK]; Q],
    ) -> [u32; Q] {
        let low = _mm512_set1_epi8(0x0f);
        // Byte p of an index vector gets the group offset 16 * (p % 4).
        let off = _mm512_set1_epi32(0x3020_1000);
        let ones = _mm512_set1_epi8(1);
        let zero = _mm512_setzero_si512();
        let (mut lo, mut hi) = ([zero; Q], [zero; Q]);
        for q in 0..nq {
            let c = _mm512_loadu_si512(codes.add(64 * q).cast());
            // (c & low) | off.
            let a = _mm512_ternarylogic_epi32::<0xEA>(c, low, off);
            let b = _mm512_ternarylogic_epi32::<0xEA>(_mm512_srli_epi16::<4>(c), low, off);
            for k in 0..Q {
                let t = _mm512_loadu_si512(luts.add(sel[k] * lstride + 64 * q).cast());
                lo[k] = _mm512_dpbusd_epi32(lo[k], _mm512_permutexvar_epi8(a, t), ones);
                hi[k] = _mm512_dpbusd_epi32(hi[k], _mm512_permutexvar_epi8(b, t), ones);
            }
        }
        let mut mask = [0u32; Q];
        for k in 0..Q {
            let cut = _mm512_set1_epi32(cuts[k] as i32);
            let (ml, mh) = (
                _mm512_cmplt_epu32_mask(lo[k], cut),
                _mm512_cmplt_epu32_mask(hi[k], cut),
            );
            if ml | mh != 0 {
                _mm512_storeu_si512(out[k].as_mut_ptr().cast(), lo[k]);
                _mm512_storeu_si512(out[k].as_mut_ptr().add(16).cast(), hi[k]);
                mask[k] = u32::from(ml) | (u32::from(mh) << 16);
            }
        }
        mask
    }
}

/// Builds the u8 table of one (query, list) pair from a shared per-query table and a per-list
/// table (`16 * m` entries each, summed): each group is shifted by its minimum and all groups
/// share one scale so the largest group span maps to 255. Returns `(base, scale)`: a code's
/// float score is `base + scale * sum`, where `base` is the sum of the group minimums.
// The only library caller is the experimental partition tier.
#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
pub(crate) fn build_lut(
    qt: &[f32],
    lt: &[f32],
    ent: &mut [f32],
    mins: &mut [f32],
    lut: &mut [u8],
) -> (f32, f32) {
    #[cfg(target_arch = "x86_64")]
    if use_avx512() && !scalar_lut() {
        let e = qt.len();
        assert!(e.is_multiple_of(16) && lt.len() == e && mins.len() >= e / 16 && lut.len() >= e);
        // SAFETY: AVX-512F/BW/VL and AVX2 detected; every slice holds `e` (`e / 16` for `mins`)
        // elements, checked above.
        return unsafe { avx512::build_lut(qt, lt, &mut mins[..e / 16], &mut lut[..e]) };
    }
    build_lut_scalar(qt, lt, ent, mins, lut)
}

#[cfg_attr(not(feature = "experimental"), allow(dead_code))]
fn build_lut_scalar(
    qt: &[f32],
    lt: &[f32],
    ent: &mut [f32],
    mins: &mut [f32],
    lut: &mut [u8],
) -> (f32, f32) {
    let e = qt.len();
    let m = e / 16;
    assert!(
        e.is_multiple_of(16)
            && lt.len() == e
            && ent.len() >= e
            && mins.len() >= m
            && lut.len() >= e
    );
    let (mut base, mut span) = (0f32, 0f32);
    for (b, mn) in mins[..m].iter_mut().enumerate() {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for v in 16 * b..16 * b + 16 {
            let x = qt[v] + lt[v];
            ent[v] = x;
            lo = lo.min(x);
            hi = hi.max(x);
        }
        *mn = lo;
        span = span.max(hi - lo);
        base += lo;
    }
    let scale = span.max(1e-12) / 255.0;
    let inv = 1.0 / scale;
    for (b, &mn) in mins[..m].iter().enumerate() {
        for v in 16 * b..16 * b + 16 {
            lut[v] = ((ent[v] - mn) * inv + 0.5) as u8;
        }
    }
    (base, scale)
}

/// Packs 32 codes of `m` nibbles each (`nibbles[c * m + j]`, values below 16) into the block
/// layout `scan32` reads; `block` holds `16 * m` bytes.
pub(crate) fn pack32(nibbles: &[u8], m: usize, block: &mut [u8]) {
    assert!(nibbles.len() == BLOCK * m && block.len() == 16 * m);
    for j in 0..m {
        for i in 0..16 {
            let lo = nibbles[i * m + j];
            let hi = nibbles[(i + 16) * m + j];
            debug_assert!(lo < 16 && hi < 16);
            block[16 * j + i] = lo | (hi << 4);
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use std::arch::aarch64::*;

    use super::BLOCK;

    /// # Safety
    /// `codes` and `lut` hold at least `16 * m` bytes.
    #[inline]
    pub(super) unsafe fn scan32(codes: &[u8], lut: &[u8], m: usize, out: &mut [u16; BLOCK]) {
        let mask = vdupq_n_u8(0x0f);
        let (mut a0, mut a1, mut a2, mut a3) = (
            vdupq_n_u16(0),
            vdupq_n_u16(0),
            vdupq_n_u16(0),
            vdupq_n_u16(0),
        );
        let (mut b0, mut b1, mut b2, mut b3) = (
            vdupq_n_u16(0),
            vdupq_n_u16(0),
            vdupq_n_u16(0),
            vdupq_n_u16(0),
        );
        let (cp, lp) = (codes.as_ptr(), lut.as_ptr());
        let mut j = 0;
        // Two independent accumulator sets break the add latency chain.
        while j + 2 <= m {
            let c = vld1q_u8(cp.add(16 * j));
            let t = vld1q_u8(lp.add(16 * j));
            let rlo = vqtbl1q_u8(t, vandq_u8(c, mask));
            let rhi = vqtbl1q_u8(t, vshrq_n_u8::<4>(c));
            a0 = vaddw_u8(a0, vget_low_u8(rlo));
            a1 = vaddw_high_u8(a1, rlo);
            a2 = vaddw_u8(a2, vget_low_u8(rhi));
            a3 = vaddw_high_u8(a3, rhi);
            let c = vld1q_u8(cp.add(16 * j + 16));
            let t = vld1q_u8(lp.add(16 * j + 16));
            let rlo = vqtbl1q_u8(t, vandq_u8(c, mask));
            let rhi = vqtbl1q_u8(t, vshrq_n_u8::<4>(c));
            b0 = vaddw_u8(b0, vget_low_u8(rlo));
            b1 = vaddw_high_u8(b1, rlo);
            b2 = vaddw_u8(b2, vget_low_u8(rhi));
            b3 = vaddw_high_u8(b3, rhi);
            j += 2;
        }
        if j < m {
            let c = vld1q_u8(cp.add(16 * j));
            let t = vld1q_u8(lp.add(16 * j));
            let rlo = vqtbl1q_u8(t, vandq_u8(c, mask));
            let rhi = vqtbl1q_u8(t, vshrq_n_u8::<4>(c));
            a0 = vaddw_u8(a0, vget_low_u8(rlo));
            a1 = vaddw_high_u8(a1, rlo);
            a2 = vaddw_u8(a2, vget_low_u8(rhi));
            a3 = vaddw_high_u8(a3, rhi);
        }
        let o = out.as_mut_ptr();
        vst1q_u16(o, vaddq_u16(a0, b0));
        vst1q_u16(o.add(8), vaddq_u16(a1, b1));
        vst1q_u16(o.add(16), vaddq_u16(a2, b2));
        vst1q_u16(o.add(24), vaddq_u16(a3, b3));
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    use super::{add_group, BLOCK};

    /// Two 16-byte groups per 256-bit register: lanes 0..16 score group `j`, lanes 16..32
    /// group `j + 1`, both with their own table half. The shuffled bytes are added as 16-bit
    /// words: word i holds byte 2i plus 256 times byte 2i + 1. A second accumulator sums the
    /// odd bytes alone (word >> 8), so the even bytes are `all - (odd << 8)` at the end. Two
    /// adds and a shift per shuffle replace four widening conversions; sums stay below 2^16
    /// for m <= 256 (each lane adds at most m / 2 groups of 255). Returns the accumulators
    /// (low-nibble all, low odd, high all, high odd) and the number of groups consumed.
    ///
    /// # Safety
    /// The CPU supports AVX2; `codes` and `lut` hold at least `16 * m` bytes.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn accumulate(
        codes: &[u8],
        lut: &[u8],
        m: usize,
    ) -> (__m256i, __m256i, __m256i, __m256i, usize) {
        let mask = _mm256_set1_epi8(0x0f);
        let zero = _mm256_setzero_si256();
        let (mut lo_all, mut lo_odd, mut hi_all, mut hi_odd) = (zero, zero, zero, zero);
        let (cp, lp) = (codes.as_ptr(), lut.as_ptr());
        let mut j = 0;
        while j + 2 <= m {
            let c = _mm256_loadu_si256(cp.add(16 * j).cast());
            let t = _mm256_loadu_si256(lp.add(16 * j).cast());
            let rlo = _mm256_shuffle_epi8(t, _mm256_and_si256(c, mask));
            let rhi = _mm256_shuffle_epi8(t, _mm256_and_si256(_mm256_srli_epi16::<4>(c), mask));
            lo_all = _mm256_add_epi16(lo_all, rlo);
            lo_odd = _mm256_add_epi16(lo_odd, _mm256_srli_epi16::<8>(rlo));
            hi_all = _mm256_add_epi16(hi_all, rhi);
            hi_odd = _mm256_add_epi16(hi_odd, _mm256_srli_epi16::<8>(rhi));
            j += 2;
        }
        (lo_all, lo_odd, hi_all, hi_odd, j)
    }

    /// Adds the lane halves (even and odd groups) of one accumulator pair and returns the
    /// sums of the even-numbered and odd-numbered codes (8 u16 each).
    ///
    /// # Safety
    /// The CPU supports AVX2.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn fold(all: __m256i, odd: __m256i) -> (__m128i, __m128i) {
        let even = _mm256_sub_epi16(all, _mm256_slli_epi16::<8>(odd));
        let e = _mm_add_epi16(
            _mm256_castsi256_si128(even),
            _mm256_extracti128_si256::<1>(even),
        );
        let o = _mm_add_epi16(
            _mm256_castsi256_si128(odd),
            _mm256_extracti128_si256::<1>(odd),
        );
        (e, o)
    }

    /// Interleaves even and odd codes back into code order.
    ///
    /// # Safety
    /// The CPU supports AVX2; `dst` has room for 16 u16.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn store(e: __m128i, o: __m128i, dst: *mut u16) {
        _mm_storeu_si128(dst.cast(), _mm_unpacklo_epi16(e, o));
        _mm_storeu_si128(dst.add(8).cast(), _mm_unpackhi_epi16(e, o));
    }

    /// # Safety
    /// The CPU supports AVX2; `codes` and `lut` hold at least `16 * m` bytes.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn scan32(codes: &[u8], lut: &[u8], m: usize, out: &mut [u16; BLOCK]) {
        let (lo_all, lo_odd, hi_all, hi_odd, j) = accumulate(codes, lut, m);
        let o = out.as_mut_ptr();
        let (e, od) = fold(lo_all, lo_odd);
        store(e, od, o);
        let (e, od) = fold(hi_all, hi_odd);
        store(e, od, o.add(16));
        if j < m {
            add_group(&codes[16 * j..16 * j + 16], &lut[16 * j..16 * j + 16], out);
        }
    }

    /// [`scan32`] plus the mask of codes whose sum is below `cut`; `out` is written only when
    /// the mask is non-zero.
    ///
    /// # Safety
    /// As [`scan32`].
    #[target_feature(enable = "avx2")]
    #[cfg_attr(not(feature = "experimental"), allow(dead_code))]
    pub(super) unsafe fn scan32_lt(
        codes: &[u8],
        lut: &[u8],
        m: usize,
        cut: u16,
        out: &mut [u32; BLOCK],
    ) -> u32 {
        let (lo_all, lo_odd, hi_all, hi_odd, j) = accumulate(codes, lut, m);
        let (el, ol) = fold(lo_all, lo_odd);
        let (eh, oh) = fold(hi_all, hi_odd);
        if j == m {
            // The smallest sum is below `cut` iff `max(min, cut) != min`.
            let mn = _mm_min_epu16(_mm_min_epu16(el, ol), _mm_min_epu16(eh, oh));
            let same = _mm_cmpeq_epi16(_mm_max_epu16(mn, _mm_set1_epi16(cut as i16)), mn);
            if _mm_movemask_epi8(same) == 0xFFFF {
                return 0;
            }
        }
        let mut sums = [0u16; BLOCK];
        store(el, ol, sums.as_mut_ptr());
        store(eh, oh, sums.as_mut_ptr().add(16));
        if j < m {
            add_group(
                &codes[16 * j..16 * j + 16],
                &lut[16 * j..16 * j + 16],
                &mut sums,
            );
        }
        let mut mask = 0u32;
        for (c, (o, &s)) in out.iter_mut().zip(&sums).enumerate() {
            *o = u32::from(s);
            mask |= u32::from(s < cut) << c;
        }
        mask
    }
}

#[cfg(target_arch = "x86_64")]
mod avx512 {
    use std::arch::x86_64::*;

    use super::BLOCK;

    /// Sums the four 128-bit lanes of `v` (u16 words).
    ///
    /// # Safety
    /// The CPU supports AVX-512F and AVX2.
    #[inline]
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2")]
    unsafe fn fold_lanes(v: __m512i) -> __m128i {
        let h = _mm256_add_epi16(_mm512_castsi512_si256(v), _mm512_extracti64x4_epi64::<1>(v));
        _mm_add_epi16(_mm256_castsi256_si128(h), _mm256_extracti128_si256::<1>(h))
    }

    /// The AVX2 scheme at 512 bits: four 16-byte groups per register, each 128-bit lane scored
    /// against its own table lane (`vpshufb` never crosses lanes). A tail of one to three
    /// groups uses zero-masked loads, so the missing lanes look up a zero table. Same even/odd
    /// u16 word scheme as the AVX2 path: `*_all` adds whole words (even byte plus 256 times odd
    /// byte), `*_odd` adds the odd bytes alone. A word sums at most ceil(m / 4) <= 64 groups of
    /// 255 per lane before the lanes fold, and the folded sums are below 2^16 for m <= 256.
    /// Returns (low-nibble all, low odd, high all, high odd).
    ///
    /// # Safety
    /// The CPU supports AVX-512F, AVX-512BW and AVX2; `codes` and `lut` hold at least
    /// `16 * m` bytes.
    #[inline]
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2")]
    unsafe fn accumulate(
        codes: &[u8],
        lut: &[u8],
        m: usize,
    ) -> (__m512i, __m512i, __m512i, __m512i) {
        let mask = _mm512_set1_epi8(0x0f);
        let zero = _mm512_setzero_si512();
        let (mut lo_all, mut lo_odd, mut hi_all, mut hi_odd) = (zero, zero, zero, zero);
        let (cp, lp) = (codes.as_ptr(), lut.as_ptr());
        let mut j = 0;
        while j + 4 <= m {
            let c = _mm512_loadu_si512(cp.add(16 * j).cast());
            let t = _mm512_loadu_si512(lp.add(16 * j).cast());
            let rlo = _mm512_shuffle_epi8(t, _mm512_and_si512(c, mask));
            let rhi = _mm512_shuffle_epi8(t, _mm512_and_si512(_mm512_srli_epi16::<4>(c), mask));
            lo_all = _mm512_add_epi16(lo_all, rlo);
            lo_odd = _mm512_add_epi16(lo_odd, _mm512_srli_epi16::<8>(rlo));
            hi_all = _mm512_add_epi16(hi_all, rhi);
            hi_odd = _mm512_add_epi16(hi_odd, _mm512_srli_epi16::<8>(rhi));
            j += 4;
        }
        if j < m {
            let k: __mmask64 = (1u64 << (16 * (m - j))) - 1;
            let c = _mm512_maskz_loadu_epi8(k, cp.add(16 * j).cast());
            let t = _mm512_maskz_loadu_epi8(k, lp.add(16 * j).cast());
            let rlo = _mm512_shuffle_epi8(t, _mm512_and_si512(c, mask));
            let rhi = _mm512_shuffle_epi8(t, _mm512_and_si512(_mm512_srli_epi16::<4>(c), mask));
            lo_all = _mm512_add_epi16(lo_all, rlo);
            lo_odd = _mm512_add_epi16(lo_odd, _mm512_srli_epi16::<8>(rlo));
            hi_all = _mm512_add_epi16(hi_all, rhi);
            hi_odd = _mm512_add_epi16(hi_odd, _mm512_srli_epi16::<8>(rhi));
        }
        (lo_all, lo_odd, hi_all, hi_odd)
    }

    /// [`super::build_lut_scalar`] with the same arithmetic order: the sums are recomputed in
    /// the second pass instead of stored, and min, max and the u8 conversion are lane-wise
    /// (min and max are exact; the scale step is a multiply then an add, never fused).
    ///
    /// # Safety
    /// The CPU supports AVX-512F/BW/VL and AVX2; `qt` and `lt` hold `16 * mins.len()` floats and
    /// `lut` holds as many bytes.
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2")]
    pub(super) unsafe fn build_lut(
        qt: &[f32],
        lt: &[f32],
        mins: &mut [f32],
        lut: &mut [u8],
    ) -> (f32, f32) {
        let (mut base, mut span) = (0f32, 0f32);
        for (b, mn) in mins.iter_mut().enumerate() {
            let x = _mm512_add_ps(
                _mm512_loadu_ps(qt.as_ptr().add(16 * b)),
                _mm512_loadu_ps(lt.as_ptr().add(16 * b)),
            );
            let lo = _mm512_reduce_min_ps(x);
            let hi = _mm512_reduce_max_ps(x);
            *mn = lo;
            span = span.max(hi - lo);
            base += lo;
        }
        let scale = span.max(1e-12) / 255.0;
        let (inv, half) = (_mm512_set1_ps(1.0 / scale), _mm512_set1_ps(0.5));
        let top = _mm512_set1_epi32(255);
        for (b, &mn) in mins.iter().enumerate() {
            let x = _mm512_add_ps(
                _mm512_loadu_ps(qt.as_ptr().add(16 * b)),
                _mm512_loadu_ps(lt.as_ptr().add(16 * b)),
            );
            let y = _mm512_add_ps(
                _mm512_mul_ps(_mm512_sub_ps(x, _mm512_set1_ps(mn)), inv),
                half,
            );
            // y is in [0.5, 255.5 + rounding]: truncation equals the scalar `as u8` cast.
            let q = _mm512_min_epi32(_mm512_cvttps_epi32(y), top);
            _mm_storeu_si128(lut.as_mut_ptr().add(16 * b).cast(), _mm512_cvtepi32_epi8(q));
        }
        (base, scale)
    }

    /// Folds one accumulator pair into the sums of the even-numbered and odd-numbered codes.
    ///
    /// # Safety
    /// The CPU supports AVX-512F/BW and AVX2.
    #[inline]
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2")]
    unsafe fn fold(all: __m512i, odd: __m512i) -> (__m128i, __m128i) {
        let even = _mm512_sub_epi16(all, _mm512_slli_epi16::<8>(odd));
        (fold_lanes(even), fold_lanes(odd))
    }

    /// # Safety
    /// The CPU supports AVX2; `dst` has room for 16 u16.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn store(e: __m128i, o: __m128i, dst: *mut u16) {
        _mm_storeu_si128(dst.cast(), _mm_unpacklo_epi16(e, o));
        _mm_storeu_si128(dst.add(8).cast(), _mm_unpackhi_epi16(e, o));
    }

    /// # Safety
    /// The CPU supports AVX-512F, AVX-512BW, AVX-512VL and AVX2; `codes` and `lut` hold at
    /// least `16 * m` bytes.
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2")]
    pub(super) unsafe fn scan32(codes: &[u8], lut: &[u8], m: usize, out: &mut [u16; BLOCK]) {
        let (lo_all, lo_odd, hi_all, hi_odd) = accumulate(codes, lut, m);
        let o = out.as_mut_ptr();
        let (e, od) = fold(lo_all, lo_odd);
        store(e, od, o);
        let (e, od) = fold(hi_all, hi_odd);
        store(e, od, o.add(16));
    }

    /// [`scan32`] plus the mask of codes whose sum is below `cut`; `out` is written only when
    /// the mask is non-zero. The test runs on the folded halves before they are interleaved:
    /// the smallest sum is below `cut` iff any code is.
    ///
    /// # Safety
    /// As [`scan32`].
    #[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2")]
    #[cfg_attr(not(feature = "experimental"), allow(dead_code))]
    pub(super) unsafe fn scan32_lt(
        codes: &[u8],
        lut: &[u8],
        m: usize,
        cut: u16,
        out: &mut [u32; BLOCK],
    ) -> u32 {
        let (lo_all, lo_odd, hi_all, hi_odd) = accumulate(codes, lut, m);
        let (el, ol) = fold(lo_all, lo_odd);
        let (eh, oh) = fold(hi_all, hi_odd);
        let mn = _mm_min_epu16(_mm_min_epu16(el, ol), _mm_min_epu16(eh, oh));
        if _mm_cmplt_epu16_mask(mn, _mm_set1_epi16(cut as i16)) == 0 {
            return 0;
        }
        let mut sums = [0u16; BLOCK];
        store(el, ol, sums.as_mut_ptr());
        store(eh, oh, sums.as_mut_ptr().add(16));
        let v = _mm512_loadu_si512(sums.as_ptr().cast());
        let mask = _mm512_cmplt_epu16_mask(v, _mm512_set1_epi16(cut as i16));
        let o = out.as_mut_ptr();
        _mm512_storeu_si512(o.cast(), _mm512_cvtepu16_epi32(_mm512_castsi512_si256(v)));
        _mm512_storeu_si512(
            o.add(16).cast(),
            _mm512_cvtepu16_epi32(_mm512_extracti64x4_epi64::<1>(v)),
        );
        mask
    }
}

#[cfg(test)]
mod tests {
    use super::super::kernels::SplitMix;
    use super::*;

    fn random_block(m: usize, seed: u64) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut rng = SplitMix(seed);
        let nibbles: Vec<u8> = (0..BLOCK * m)
            .map(|_| (rng.next_u64() & 15) as u8)
            .collect();
        let lut: Vec<u8> = (0..16 * m).map(|_| (rng.next_u64() & 255) as u8).collect();
        let mut block = vec![0u8; 16 * m];
        pack32(&nibbles, m, &mut block);
        (nibbles, lut, block)
    }

    #[test]
    fn lut_reconstructs_scores_within_half_a_step() {
        let mut rng = SplitMix(3);
        for &m in &[1usize, 2, 25, 50, 100] {
            let e = 16 * m;
            let unit = |r: &mut SplitMix| (r.next_u64() >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
            let qt: Vec<f32> = (0..e).map(|_| unit(&mut rng) * 4.0).collect();
            let lt: Vec<f32> = (0..e).map(|_| unit(&mut rng)).collect();
            let (mut ent, mut mins, mut lut) = (vec![0f32; e], vec![0f32; m], vec![0u8; e]);
            let (base, scale) = build_lut(&qt, &lt, &mut ent, &mut mins, &mut lut);
            let (nibbles, _, block) = random_block(m, m as u64 + 9);
            let mut out = [0u32; BLOCK];
            scan32(&block, &lut, m, &mut out);
            for c in 0..BLOCK {
                let exact: f32 = (0..m)
                    .map(|j| {
                        qt[16 * j + nibbles[c * m + j] as usize]
                            + lt[16 * j + nibbles[c * m + j] as usize]
                    })
                    .sum();
                let approx = base + scale * out[c] as f32;
                assert!(
                    (approx - exact).abs() <= 0.5 * scale * m as f32 + 1e-4,
                    "m {m} code {c}: {approx} vs {exact}"
                );
            }
        }
    }

    #[test]
    fn scan_matches_direct_sum_at_every_width() {
        for &m in &[1usize, 2, 3, 16, 25, 32, 64, 100, 255, 256, 257, 300, 520] {
            let (nibbles, lut, block) = random_block(m, m as u64);
            let mut out = [0u32; BLOCK];
            scan32(&block, &lut, m, &mut out);
            for c in 0..BLOCK {
                let want: u32 = (0..m)
                    .map(|j| u32::from(lut[16 * j + nibbles[c * m + j] as usize]))
                    .sum();
                assert_eq!(out[c], want, "code {c} m {m}");
            }
            let mut scalar = [0u16; BLOCK];
            if m <= 256 {
                scan32_scalar(&block, &lut, m, &mut scalar);
                for c in 0..BLOCK {
                    assert_eq!(u32::from(scalar[c]), out[c]);
                }
            }
        }
    }

    /// Every x86 kernel the CPU supports must equal the scalar sum bit for bit, at widths
    /// that exercise the 2-group (AVX2) and 4-group (AVX-512) tails.
    /// The admission mask equals a compare of the exact sums, at the cut values around the
    /// minimum, a middle value, the maximum and beyond the u16 range, for odd and even `m`
    /// and `m` above 256 (the generic path), on every kernel the CPU runs.
    #[test]
    fn scan32_lt_matches_the_sums() {
        for m in (1usize..=9).chain([25, 50, 51, 100, 255, 256, 257, 300]) {
            for seed in 0..3u64 {
                let (_, lut, block) = random_block(m, 7000 * m as u64 + seed);
                let lut = if seed == 2 { vec![255u8; 16 * m] } else { lut };
                let mut sums = [0u32; BLOCK];
                scan32(&block, &lut, m, &mut sums);
                let (lo, hi) = (*sums.iter().min().unwrap(), *sums.iter().max().unwrap());
                let mid = sums[7];
                for cut in [
                    0,
                    1,
                    lo,
                    lo + 1,
                    mid,
                    mid + 1,
                    hi,
                    hi + 1,
                    65535,
                    65536,
                    u32::MAX,
                ] {
                    let want = sums
                        .iter()
                        .enumerate()
                        .fold(0u32, |k, (c, &v)| k | (u32::from(v < cut) << c));
                    let mut got = [0u32; BLOCK];
                    let mask = scan32_lt(&block, &lut, m, cut, &mut got);
                    assert_eq!(mask, want, "m {m} cut {cut}");
                    if mask != 0 {
                        assert_eq!(got, sums, "m {m} cut {cut}");
                    }
                }
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn simd_lut_build_is_bit_identical_to_scalar() {
        if !avx512_available() {
            eprintln!("skipped: no AVX-512");
            return;
        }
        let mut rng = SplitMix(5);
        for &m in &[1usize, 3, 25, 50, 100] {
            for case in 0..40 {
                let e = 16 * m;
                let unit = |r: &mut SplitMix| (r.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
                // Wide, narrow and constant-group spreads exercise the scale and the rounding.
                let spread = [8.0f32, 1.0, 1e-3, 0.0][case % 4];
                let qt: Vec<f32> = (0..e).map(|_| (unit(&mut rng) - 0.5) * spread).collect();
                let lt: Vec<f32> = (0..e).map(|_| (unit(&mut rng) - 0.5) * spread).collect();
                let (mut ent, mut mins, mut lut) = (vec![0f32; e], vec![0f32; m], vec![0u8; e]);
                let want = build_lut_scalar(&qt, &lt, &mut ent, &mut mins, &mut lut);
                let (mut ent2, mut mins2, mut lut2) = (vec![0f32; e], vec![0f32; m], vec![0u8; e]);
                let got = build_lut(&qt, &lt, &mut ent2, &mut mins2, &mut lut2);
                assert_eq!(got.0.to_bits(), want.0.to_bits(), "base m {m} case {case}");
                assert_eq!(got.1.to_bits(), want.1.to_bits(), "scale m {m} case {case}");
                assert_eq!(lut2, lut, "m {m} case {case}");
            }
        }
    }

    /// The quad kernel sums, per table and block, equal the direct sums at every width, with
    /// one to four tables per call and cuts around the extremes.
    #[test]
    fn quad_scan_matches_the_sums() {
        if !quad_available() {
            eprintln!("skipped: no AVX-512 VBMI/VNNI");
            return;
        }
        let mut rng = SplitMix(21);
        for m in (1usize..=9).chain([25, 50, 51, 100, 255, 256, 257, 300]) {
            let stride = quad_stride(m);
            let nibbles: Vec<u8> = (0..BLOCK * m)
                .map(|_| (rng.next_u64() & 15) as u8)
                .collect();
            let mut block = vec![0u8; stride];
            for i in 0..BLOCK {
                for j in 0..m {
                    let v = nibbles[i * m + j];
                    block[quad_pos(i, j)] |= if i < 16 { v } else { v << 4 };
                }
            }
            let mut luts = vec![0u8; 4 * stride];
            for k in 0..4 {
                for b in luts[k * stride..k * stride + 16 * m].iter_mut() {
                    *b = if k == 3 {
                        255
                    } else {
                        (rng.next_u64() & 255) as u8
                    };
                }
            }
            let sums = |k: usize| -> [u32; BLOCK] {
                let mut s = [0u32; BLOCK];
                for (c, sc) in s.iter_mut().enumerate() {
                    *sc = (0..m)
                        .map(|j| u32::from(luts[k * stride + 16 * j + nibbles[c * m + j] as usize]))
                        .sum();
                }
                s
            };
            let want: Vec<[u32; BLOCK]> = (0..4).map(sums).collect();
            let mid = want[0][5];
            let hi = *want[0].iter().max().unwrap();
            for cut in [0, 1, mid, mid + 1, hi, hi + 1, u32::MAX] {
                let mut out = [[0u32; BLOCK]; 4];
                let cuts = [cut, 0, cut, u32::MAX];
                let masks = scan32q_lt::<4>(&block, &luts, stride, m, [0, 1, 2, 3], cuts, &mut out);
                for k in 0..4 {
                    let exp = want[k]
                        .iter()
                        .enumerate()
                        .fold(0u32, |a, (c, &v)| a | (u32::from(v < cuts[k]) << c));
                    assert_eq!(masks[k], exp, "m {m} table {k} cut {cut}");
                    if masks[k] != 0 {
                        assert_eq!(out[k], want[k], "m {m} table {k}");
                    }
                }
                let mut one = [[0u32; BLOCK]; 1];
                let m1 = scan32q_lt::<1>(&block, &luts, stride, m, [0], [cut], &mut one);
                // A permuted subset of tables gives the same masks.
                let mut two = [[0u32; BLOCK]; 2];
                let m2 =
                    scan32q_lt::<2>(&block, &luts, stride, m, [3, 0], [u32::MAX, cut], &mut two);
                assert_eq!(m2[1], masks[0], "m {m} cut {cut} subset");
                assert_eq!(m2[0], u32::MAX, "m {m} subset all-pass");
                assert_eq!(m1[0], masks[0], "m {m} cut {cut} single");
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn x86_kernels_match_scalar() {
        let (has2, has512) = (is_x86_feature_detected!("avx2"), avx512_available());
        if !has2 && !has512 {
            eprintln!("skipped: no AVX2/AVX-512");
            return;
        }
        let widths = (1usize..=70).chain([100, 127, 128, 129, 200, 254, 255, 256]);
        for m in widths {
            for seed in 0..4u64 {
                let (_, lut, block) = random_block(m, 1000 * m as u64 + seed);
                // All-255 table: the largest possible sums at the limit width.
                let full = vec![255u8; 16 * m];
                for table in [&lut, &full] {
                    let mut want = [0u16; BLOCK];
                    scan32_scalar(&block, table, m, &mut want);
                    if has2 {
                        let mut got = [0u16; BLOCK];
                        // SAFETY: AVX2 detected; `block` and `table` hold `16 * m` bytes.
                        unsafe { avx2::scan32(&block, table, m, &mut got) };
                        assert_eq!(got, want, "avx2 m {m}");
                    }
                    if has512 {
                        let mut got = [0u16; BLOCK];
                        // SAFETY: AVX-512F/BW and AVX2 detected; lengths as above.
                        unsafe { avx512::scan32(&block, table, m, &mut got) };
                        assert_eq!(got, want, "avx512 m {m}");
                    }
                }
            }
        }
    }

    /// Kill test of the neighbour-block estimator (lever 1A.1): ns per neighbour of the
    /// block scan at the nibble counts of 1-, 2- and 4-bit codes against the 8-bit SDOT
    /// gather estimate, same method as `raw_screen_microbenchmark` (300k rows, dim 256,
    /// 20k expansions of 32 neighbours, best of 5). Kill rule: block scan < 0.25 of est.
    /// `cargo test --release block_scan_microbenchmark -- --ignored --nocapture`; writes
    /// JSON to `$WARM_BLOCK_BENCH_OUT` when set.
    #[test]
    #[ignore = "microbenchmark: run in release with --ignored --nocapture"]
    fn block_scan_microbenchmark() {
        use super::super::kernels::{dot_i8_inline, prefetch_bytes};
        use std::time::Instant;
        const N: usize = 300_000;
        const DIM: usize = 256;
        const DEG: usize = 32;
        const EXPANSIONS: usize = 20_000;
        let mut rng = SplitMix(7);
        let codes: Vec<i8> = (0..N * DIM)
            .map(|_| (rng.next_u64() & 0xff) as i8)
            .collect();
        let q: Vec<i8> = (0..DIM).map(|_| (rng.next_u64() & 0xff) as i8).collect();
        let ids: Vec<u32> = (0..EXPANSIONS * DEG)
            .map(|_| (rng.next_u64() % N as u64) as u32)
            .collect();
        let code = |v: u32| &codes[v as usize * DIM..(v as usize + 1) * DIM];
        let est = |batch: &[u32]| -> i64 {
            batch.iter().for_each(|&v| prefetch_bytes(code(v)));
            batch
                .iter()
                .map(|&v| i64::from(dot_i8_inline(&q, code(v))))
                .sum()
        };
        let mut best = f64::INFINITY;
        let mut acc = 0i64;
        for _ in 0..5 {
            let t = Instant::now();
            for batch in ids.as_chunks::<DEG>().0 {
                acc = acc.wrapping_add(est(batch));
            }
            best = best.min(t.elapsed().as_secs_f64() * 1e9 / ids.len() as f64);
        }
        let t_est = best;
        let mut report = format!(
            "{{\"n_rows\": {N}, \"dim\": {DIM}, \"degree\": {DEG}, \"expansions\": {EXPANSIONS}, \
             \"est_8bit_sdot_ns_per_neighbour\": {t_est:.2}, \"kill_rule\": \"block scan < 0.25 of est\", \
             \"est_checksum\": {acc}, \"block_scan\": ["
        );
        let mut first = true;
        for (bits, m) in [(1usize, DIM / 4), (2, DIM / 2), (4, DIM)] {
            // One block of 32 neighbour codes per vertex, read at a random vertex per
            // expansion; the lookup table is per query.
            let blocks: Vec<u8> = (0..N * 16 * m)
                .map(|_| (rng.next_u64() & 255) as u8)
                .collect();
            let lut: Vec<u8> = (0..16 * m).map(|_| (rng.next_u64() & 255) as u8).collect();
            let block = |v: u32| &blocks[v as usize * 16 * m..(v as usize + 1) * 16 * m];
            let mut out = [0u32; BLOCK];
            let mut best = f64::INFINITY;
            let mut sum = 0u64;
            for _ in 0..5 {
                let t = Instant::now();
                let mut s = 0u64;
                for (e, batch) in ids.as_chunks::<DEG>().0.iter().enumerate() {
                    let v = batch[0];
                    if let Some(next) = ids.as_chunks::<DEG>().0.get(e + 1) {
                        for line in block(next[0]).chunks(128) {
                            // SAFETY: u8 and i8 have the same size and alignment, and every
                            // byte is a valid i8; the view borrows `line`.
                            prefetch_bytes(unsafe {
                                std::slice::from_raw_parts(line.as_ptr().cast::<i8>(), line.len())
                            });
                        }
                    }
                    scan32(block(v), &lut, m, &mut out);
                    s += out.iter().map(|&x| u64::from(x)).sum::<u64>();
                }
                sum = s;
                best = best.min(t.elapsed().as_secs_f64() * 1e9 / ids.len() as f64);
            }
            report.push_str(&format!(
                "{}{{\"bits\": {bits}, \"m\": {m}, \"bytes_per_block\": {}, \"ns_per_neighbour\": {best:.2}, \
                 \"ratio_to_est\": {:.3}, \"pass\": {}, \"checksum\": {sum}}}",
                if first { "" } else { ", " },
                16 * m,
                best / t_est,
                best < 0.25 * t_est
            ));
            first = false;
        }
        report.push_str("]}");
        println!("{report}");
        if let Ok(path) = crate::compatibility::var("WARM_BLOCK_BENCH_OUT") {
            std::fs::write(path, format!("{report}\n")).unwrap();
        }
    }

    /// L22 kill test (research/STREAMING-L22-DESIGN.md section 6, STATE.md 21:15Z): one search
    /// step of the partition tier, list-major, on 8 threads. Per (query, list): the LUT is the
    /// sum of a shared per-query table and a per-list table (the IVFPQ precomputed split),
    /// quantized to u8 per block with one scale; the scan admits a code only when its u16 sum
    /// beats the query's running reorder threshold converted into that list's units; admitted
    /// codes go to a per-thread bounded heap per query, merged at the end. Random codes and
    /// tables (the scan cost does not depend on the values). Kill rule: codes/s including the
    /// LUT build and admission >= 6e9 on 8 threads.
    /// `cargo test --release partition_step_microbenchmark -- --ignored --nocapture`; knobs
    /// `WARM_L22_{LISTS,LIST_LEN,QUERIES,NPROBE,M,REORDER,THREADS,REPS}`, JSON to `WARM_L22_BENCH_OUT`.
    #[test]
    #[ignore = "microbenchmark: run in release with --ignored --nocapture"]
    fn partition_step_microbenchmark() {
        use std::time::Instant;
        let knob = |name: &str, default: usize| -> usize {
            crate::compatibility::var(&format!("WARM_L22_{name}"))
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let lists = knob("LISTS", 5000);
        let list_len = knob("LIST_LEN", 2000);
        let nq = knob("QUERIES", 10_000);
        let nprobe = knob("NPROBE", 700);
        let m = knob("M", 50);
        let reorder = knob("REORDER", 317);
        let threads = knob("THREADS", 8);
        let e = 16 * m;
        let blocks = list_len.div_ceil(BLOCK);
        let mut rng = SplitMix(11);
        let codes: Vec<u8> = (0..lists * blocks * e)
            .map(|_| (rng.next_u64() & 255) as u8)
            .collect();
        let unit = |r: &mut SplitMix| (r.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        let qtab: Vec<f32> = (0..nq * e).map(|_| unit(&mut rng)).collect();
        let ltab: Vec<f32> = (0..lists * e).map(|_| unit(&mut rng)).collect();
        // Each list is probed by each query with probability nprobe / lists.
        let probes: Vec<Vec<u32>> = (0..lists)
            .map(|l| {
                let mut r = SplitMix(1000 + l as u64);
                (0..nq as u32)
                    .filter(|_| (r.next_u64() % lists as u64) < (nprobe as u64))
                    .collect()
            })
            .collect();
        let pairs: usize = probes.iter().map(Vec::len).sum();
        let scanned = pairs as f64 * list_len as f64;

        // mode 0: full step; mode 1: scan + admission with the LUT build skipped (one table
        // per list reused); mode 2: LUT build only.
        let run = |mode: u8| -> (f64, u64) {
            // Threads own disjoint query sets (q % threads) and walk every list in order, so
            // each query has one candidate buffer and no merge; the list's codes are shared
            // through the cache. Candidates below the running threshold go to a buffer of
            // 2 * reorder; a full buffer is cut to the best `reorder` and the threshold
            // tightens to the worst kept (selection instead of a heap).
            let t = Instant::now();
            let states: Vec<u64> = std::thread::scope(|s| {
                let workers: Vec<_> = (0..threads)
                    .map(|w| {
                        let (codes, qtab, ltab, probes) = (&codes, &qtab, &ltab, &probes);
                        s.spawn(move || {
                            let mine: Vec<usize> = (w..nq).step_by(threads).collect();
                            let mut bufs: Vec<Vec<(u32, u32)>> = mine
                                .iter()
                                .map(|_| Vec::with_capacity(2 * reorder))
                                .collect();
                            let mut thr = vec![f32::INFINITY; mine.len()];
                            let mut ent = vec![0f32; e];
                            let mut lut = vec![0u8; e];
                            let mut mins = vec![0f32; m];
                            let mut out = [0u32; BLOCK];
                            let mut check = 0u64;
                            for l in 0..lists {
                                let lc = &codes[l * blocks * e..(l + 1) * blocks * e];
                                let lt = &ltab[l * e..(l + 1) * e];
                                let mut first = true;
                                // Mode 1 reuses the list's first table, so its base and scale
                                // carry over to the later queries of the list.
                                let (mut base, mut scale) = (0f32, 1f32);
                                for &q in &probes[l] {
                                    let q = q as usize;
                                    if q % threads != w {
                                        continue;
                                    }
                                    let k = q / threads;
                                    if mode != 1 || first {
                                        first = false;
                                        let qt = &qtab[q * e..(q + 1) * e];
                                        (base, scale) =
                                            build_lut(qt, lt, &mut ent, &mut mins, &mut lut);
                                    }
                                    if mode == 2 {
                                        check = check.wrapping_add(u64::from(lut[q % e]));
                                        continue;
                                    }
                                    let t_q = thr[k];
                                    if t_q <= base {
                                        continue;
                                    }
                                    let lim = ((t_q - base) / scale).min(65535.0) as u32;
                                    let buf = &mut bufs[k];
                                    for blk in 0..blocks {
                                        scan32(&lc[blk * e..(blk + 1) * e], &lut, m, &mut out);
                                        for (c, &v) in out.iter().enumerate() {
                                            if v < lim {
                                                let d = base + v as f32 * scale;
                                                let id = (l * list_len + blk * BLOCK + c) as u32;
                                                buf.push((d.to_bits(), id));
                                            }
                                        }
                                        if buf.len() >= 2 * reorder {
                                            buf.select_nth_unstable(reorder - 1);
                                            buf.truncate(reorder);
                                            let worst = buf.iter().map(|x| x.0).max().unwrap();
                                            thr[k] = f32::from_bits(worst);
                                            if thr[k] <= base {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            for buf in &mut bufs {
                                if buf.len() > reorder {
                                    buf.select_nth_unstable(reorder - 1);
                                    buf.truncate(reorder);
                                }
                                check = check
                                    .wrapping_add(buf.iter().map(|x| u64::from(x.1)).sum::<u64>());
                            }
                            check
                        })
                    })
                    .collect();
                workers.into_iter().map(|w| w.join().unwrap()).collect()
            });
            let check: u64 = states.iter().sum();
            (t.elapsed().as_secs_f64(), check)
        };
        // Modes interleave per repetition so a sustained run (WARM_L22_REPS) shows each mode's
        // trajectory over the whole run; the min is reported as before, plus mean/median/p90.
        let reps = knob("REPS", 3).max(1);
        let mut times: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut checks = [0u64; 3];
        for _ in 0..reps {
            for mode in 0..3u8 {
                let (secs, c) = run(mode);
                times[mode as usize].push(secs);
                checks[mode as usize] = c;
            }
        }
        let stats = |v: &[f64]| {
            let mut s = v.to_vec();
            s.sort_by(f64::total_cmp);
            let mean = s.iter().sum::<f64>() / s.len() as f64;
            let p90 = s[((s.len() as f64 * 0.9).ceil() as usize).clamp(1, s.len()) - 1];
            let all: Vec<String> = v.iter().map(|x| format!("{x:.4}")).collect();
            (
                s[0],
                format!(
                    "{{\"mean\": {mean:.4}, \"median\": {:.4}, \"p90\": {p90:.4}, \"all\": [{}]}}",
                    s[s.len() / 2],
                    all.join(", ")
                ),
            )
        };
        let ((full, full_stats), (scan_only, scan_stats), (lut_only, lut_stats)) =
            (stats(&times[0]), stats(&times[1]), stats(&times[2]));
        let [c0, c1, c2] = checks;
        let rate = scanned / full;
        let report = format!(
            "{{\"lists\": {lists}, \"list_len\": {list_len}, \"queries\": {nq}, \"nprobe\": {nprobe}, \
             \"m\": {m}, \"reorder\": {reorder}, \"threads\": {threads}, \"pairs\": {pairs}, \
             \"codes_scanned\": {scanned:.0}, \"step_s\": {full:.4}, \"scan_admit_s\": {scan_only:.4}, \
             \"lut_build_s\": {lut_only:.4}, \"codes_per_s\": {rate:.4e}, \
             \"ns_per_lut\": {:.1}, \"kill_rule\": \"codes_per_s >= 6e9 on 8 threads\", \
             \"pass\": {}, \"checksums\": [{c0}, {c1}, {c2}], \"reps\": {reps}, \
             \"step_stats\": {full_stats}, \"scan_admit_stats\": {scan_stats}, \"lut_build_stats\": {lut_stats}}}",
            lut_only * 1e9 * threads as f64 / pairs as f64,
            rate >= 6e9
        );
        println!("{report}");
        if let Ok(path) = crate::compatibility::var("WARM_L22_BENCH_OUT") {
            std::fs::write(path, format!("{report}\n")).unwrap();
        }
    }
}
