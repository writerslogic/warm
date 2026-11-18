// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use std::arch::x86_64::*;

use super::QUERY_BITS;

/// [`super::raw_estimates`] with the POPCNT instruction (baseline x86_64 lowers
/// `count_ones` to a bit-twiddling sequence). The same integer sums as the scalar form.
///
/// # Safety
/// The CPU supports `popcnt`.
#[target_feature(enable = "popcnt")]
pub(super) unsafe fn raw_popcnt(codes: &[u64], planes: &[u64], words: usize, out: &mut [u32]) {
    assert_eq!(planes.len(), QUERY_BITS * words);
    assert_eq!(codes.len(), out.len() * words);
    for (code, o) in codes.chunks_exact(words).zip(out.iter_mut()) {
        *o = raw_one_popcnt(code, planes, words);
    }
}

/// [`super::raw_one`] with the POPCNT instruction; `code.len() == words`.
///
/// # Safety
/// The CPU supports `popcnt`.
#[target_feature(enable = "popcnt")]
#[inline]
pub(super) unsafe fn raw_one_popcnt(code: &[u64], planes: &[u64], words: usize) -> u32 {
    debug_assert_eq!(code.len(), words);
    let (p0, rest) = planes.split_at(words);
    let (p1, rest) = rest.split_at(words);
    let (p2, p3) = rest.split_at(words);
    let mut total = 0u32;
    for ((((&c, &a0), &a1), &a2), &a3) in code.iter().zip(p0).zip(p1).zip(p2).zip(&p3[..words]) {
        total += (c & a0).count_ones()
            + ((c & a1).count_ones() << 1)
            + ((c & a2).count_ones() << 2)
            + ((c & a3).count_ones() << 3);
    }
    total
}

/// Popcount of each nibble value, for `vpshufb`.
const NIBBLE_POPCOUNT: [i8; 16] = [0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4];

/// [`super::raw_one`] with AVX2: per 32 bytes (then 16) of code, AND with each bit plane,
/// popcount every byte with the nibble table (`vpshufb`), weight the planes in u8 by
/// Horner doubling (plane 3 first), and sum the bytes with `vpsadbw`. An odd last word
/// takes POPCNT. The same integer sums as the scalar form.
///
/// # Safety
/// The CPU supports `avx2` and `popcnt`.
#[target_feature(enable = "avx2,popcnt")]
#[inline]
pub(super) unsafe fn raw_one_avx2(code: &[u64], planes: &[u64], words: usize) -> u32 {
    assert!(code.len() == words && planes.len() == QUERY_BITS * words);
    let lut128 = _mm_loadu_si128(NIBBLE_POPCOUNT.as_ptr().cast());
    let lut = _mm256_broadcastsi128_si256(lut128);
    let low = _mm256_set1_epi8(0x0f);
    let mut acc = _mm256_setzero_si256();
    let mut k = 0;
    // SAFETY (all loads in both loops): the assert above gives `code.len() == words` and
    // plane `p` = `planes[p * words..(p + 1) * words]`, so `k + 4 <= words` (`k + 2` in the
    // second loop) keeps each 32-byte (16-byte) unaligned load inside `code` and inside
    // plane `p`. Per byte the weighted sum is at most 8 * (1 + 2 + 4 + 8) = 120, so the u8
    // additions cannot overflow.
    while k + 4 <= words {
        let x = _mm256_loadu_si256(code.as_ptr().add(k).cast());
        let mut s = _mm256_setzero_si256();
        for p in (0..QUERY_BITS).rev() {
            let y = _mm256_loadu_si256(planes.as_ptr().add(p * words + k).cast());
            let v = _mm256_and_si256(x, y);
            let lo = _mm256_shuffle_epi8(lut, _mm256_and_si256(v, low));
            let hi = _mm256_shuffle_epi8(lut, _mm256_and_si256(_mm256_srli_epi16::<4>(v), low));
            s = _mm256_add_epi8(_mm256_add_epi8(s, s), _mm256_add_epi8(lo, hi));
        }
        acc = _mm256_add_epi64(acc, _mm256_sad_epu8(s, _mm256_setzero_si256()));
        k += 4;
    }
    let low = _mm_set1_epi8(0x0f);
    let mut acc128 = _mm_add_epi64(
        _mm256_castsi256_si128(acc),
        _mm256_extracti128_si256::<1>(acc),
    );
    while k + 2 <= words {
        let x = _mm_loadu_si128(code.as_ptr().add(k).cast());
        let mut s = _mm_setzero_si128();
        for p in (0..QUERY_BITS).rev() {
            let y = _mm_loadu_si128(planes.as_ptr().add(p * words + k).cast());
            let v = _mm_and_si128(x, y);
            let lo = _mm_shuffle_epi8(lut128, _mm_and_si128(v, low));
            let hi = _mm_shuffle_epi8(lut128, _mm_and_si128(_mm_srli_epi16::<4>(v), low));
            s = _mm_add_epi8(_mm_add_epi8(s, s), _mm_add_epi8(lo, hi));
        }
        acc128 = _mm_add_epi64(acc128, _mm_sad_epu8(s, _mm_setzero_si128()));
        k += 2;
    }
    // At most 64 * 15 per word, and `words` rows of a slice fit in memory: no truncation
    // below 2^32 for any row the index builds (16 planes x 64 words at most).
    let mut total = (_mm_cvtsi128_si64(acc128) + _mm_extract_epi64::<1>(acc128)) as u32;
    if k < words {
        total += tail_popcnt(code, planes, words, k);
    }
    total
}

/// [`raw_one_avx2`] over rows of `words` u64 each.
///
/// # Safety
/// The CPU supports `avx2` and `popcnt`.
#[target_feature(enable = "avx2,popcnt")]
pub(super) unsafe fn raw_avx2(codes: &[u64], planes: &[u64], words: usize, out: &mut [u32]) {
    assert_eq!(codes.len(), out.len() * words);
    for (code, o) in codes.chunks_exact(words.max(1)).zip(out.iter_mut()) {
        *o = raw_one_avx2(code, planes, words);
    }
}

/// Words `k..words` of one row with POPCNT.
#[inline(always)]
fn tail_popcnt(code: &[u64], planes: &[u64], words: usize, k: usize) -> u32 {
    (k..words)
        .map(|w| {
            (0..QUERY_BITS)
                .map(|p| (code[w] & planes[p * words + w]).count_ones() << p)
                .sum::<u32>()
        })
        .sum()
}

/// [`super::raw_one`] with AVX-512 VPOPCNTDQ: per 64 bytes (then 32 with AVX512VL) of
/// code, `vpopcntq` of the code AND each plane, weighted in u64 lanes by Horner doubling;
/// the last `words % 4` words take POPCNT. Not wired into dispatch: no AVX-512 host has
/// timed it (see `docs/performance/platforms.md`).
///
/// # Safety
/// The CPU supports `avx512f`, `avx512vl`, `avx512vpopcntdq` and `popcnt`.
#[cfg(test)]
#[target_feature(enable = "avx512f,avx512vl,avx512vpopcntdq,popcnt")]
#[inline]
pub(super) unsafe fn raw_one_vpopcntdq(code: &[u64], planes: &[u64], words: usize) -> u32 {
    assert!(code.len() == words && planes.len() == QUERY_BITS * words);
    let mut acc = _mm512_setzero_si512();
    let mut k = 0;
    // SAFETY (all loads in both loops): as in `raw_one_avx2`, `k + 8 <= words` (`k + 4`)
    // keeps each 64-byte (32-byte) unaligned load inside `code` and inside plane `p`. Lane
    // sums are at most 64 * 15 per word, far below u64 overflow.
    while k + 8 <= words {
        let x = _mm512_loadu_si512(code.as_ptr().add(k).cast());
        let mut s = _mm512_setzero_si512();
        for p in (0..QUERY_BITS).rev() {
            let y = _mm512_loadu_si512(planes.as_ptr().add(p * words + k).cast());
            let c = _mm512_popcnt_epi64(_mm512_and_si512(x, y));
            s = _mm512_add_epi64(_mm512_add_epi64(s, s), c);
        }
        acc = _mm512_add_epi64(acc, s);
        k += 8;
    }
    let mut acc256 = _mm256_setzero_si256();
    while k + 4 <= words {
        let x = _mm256_loadu_si256(code.as_ptr().add(k).cast());
        let mut s = _mm256_setzero_si256();
        for p in (0..QUERY_BITS).rev() {
            let y = _mm256_loadu_si256(planes.as_ptr().add(p * words + k).cast());
            let c = _mm256_popcnt_epi64(_mm256_and_si256(x, y));
            s = _mm256_add_epi64(_mm256_add_epi64(s, s), c);
        }
        acc256 = _mm256_add_epi64(acc256, s);
        k += 4;
    }
    let acc = _mm512_add_epi64(acc, _mm512_zextsi256_si512(acc256));
    let mut total = _mm512_reduce_add_epi64(acc) as u32;
    if k < words {
        total += tail_popcnt(code, planes, words, k);
    }
    total
}

/// [`raw_one_vpopcntdq`] over rows of `words` u64 each.
///
/// # Safety
/// As [`raw_one_vpopcntdq`].
#[cfg(test)]
#[target_feature(enable = "avx512f,avx512vl,avx512vpopcntdq,popcnt")]
pub(super) unsafe fn raw_vpopcntdq(codes: &[u64], planes: &[u64], words: usize, out: &mut [u32]) {
    assert_eq!(codes.len(), out.len() * words);
    for (code, o) in codes.chunks_exact(words.max(1)).zip(out.iter_mut()) {
        *o = raw_one_vpopcntdq(code, planes, words);
    }
}

/// [`raw_one_bitalg`] over rows of `words` u64 each.
///
/// # Safety
/// As [`raw_one_bitalg`].
#[cfg(test)]
#[target_feature(enable = "avx512f,avx512bw,avx512vl,avx512bitalg,popcnt")]
pub(super) unsafe fn raw_bitalg(codes: &[u64], planes: &[u64], words: usize, out: &mut [u32]) {
    assert_eq!(codes.len(), out.len() * words);
    for (code, o) in codes.chunks_exact(words.max(1)).zip(out.iter_mut()) {
        *o = raw_one_bitalg(code, planes, words);
    }
}

/// [`super::raw_one`] with AVX-512 BITALG: `vpopcntb` per byte of the code AND each plane,
/// weighted in u8 by Horner doubling (at most 120 per byte, as in [`raw_one_avx2`]), then
/// `vpsadbw`; 64 bytes, then 32, then POPCNT for the last `words % 4`. Not wired into
/// dispatch: no AVX-512 host has timed it.
///
/// # Safety
/// The CPU supports `avx512f`, `avx512bw`, `avx512vl`, `avx512bitalg` and `popcnt`.
#[cfg(test)]
#[target_feature(enable = "avx512f,avx512bw,avx512vl,avx512bitalg,popcnt")]
#[inline]
pub(super) unsafe fn raw_one_bitalg(code: &[u64], planes: &[u64], words: usize) -> u32 {
    assert!(code.len() == words && planes.len() == QUERY_BITS * words);
    let mut acc = _mm512_setzero_si512();
    let mut k = 0;
    // SAFETY (all loads in both loops): as in `raw_one_vpopcntdq`.
    while k + 8 <= words {
        let x = _mm512_loadu_si512(code.as_ptr().add(k).cast());
        let mut s = _mm512_setzero_si512();
        for p in (0..QUERY_BITS).rev() {
            let y = _mm512_loadu_si512(planes.as_ptr().add(p * words + k).cast());
            let c = _mm512_popcnt_epi8(_mm512_and_si512(x, y));
            s = _mm512_add_epi8(_mm512_add_epi8(s, s), c);
        }
        acc = _mm512_add_epi64(acc, _mm512_sad_epu8(s, _mm512_setzero_si512()));
        k += 8;
    }
    let mut acc256 = _mm256_setzero_si256();
    while k + 4 <= words {
        let x = _mm256_loadu_si256(code.as_ptr().add(k).cast());
        let mut s = _mm256_setzero_si256();
        for p in (0..QUERY_BITS).rev() {
            let y = _mm256_loadu_si256(planes.as_ptr().add(p * words + k).cast());
            let c = _mm256_popcnt_epi8(_mm256_and_si256(x, y));
            s = _mm256_add_epi8(_mm256_add_epi8(s, s), c);
        }
        acc256 = _mm256_add_epi64(acc256, _mm256_sad_epu8(s, _mm256_setzero_si256()));
        k += 4;
    }
    let acc = _mm512_add_epi64(acc, _mm512_zextsi256_si512(acc256));
    let mut total = _mm512_reduce_add_epi64(acc) as u32;
    if k < words {
        total += tail_popcnt(code, planes, words, k);
    }
    total
}

/// Signed i8 dot product of two rows of equal length, a multiple of 16, exact over the
/// whole i8 range: the bytes widen to i16 lanes (`vpmovsxbw`) and `vpmaddwd` sums pairs
/// of products into i32 (the `maddubs` form would misread -128 after the sign trick).
///
/// # Safety
/// The CPU supports AVX2; `a` and `b` have the same length.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn dot_i8_avx2(a: &[i8], b: &[i8]) -> i32 {
    let mut acc = _mm256_setzero_si256();
    let mut i = 0;
    // SAFETY (all loads): `i + 16 <= len` for both slices, whose lengths are equal.
    while i + 16 <= a.len() {
        let x = _mm256_cvtepi8_epi16(_mm_loadu_si128(a.as_ptr().add(i).cast()));
        let y = _mm256_cvtepi8_epi16(_mm_loadu_si128(b.as_ptr().add(i).cast()));
        acc = _mm256_add_epi32(acc, _mm256_madd_epi16(x, y));
        i += 16;
    }
    horizontal_sum(acc)
}

/// [`dot_i8_avx2`] of one row `a` against four rows: `a` is loaded and widened once per
/// 16 bytes and each row keeps its own accumulator, so the four row loads are in flight
/// together. Lane by lane the same integer sums as four single calls.
///
/// # Safety
/// The CPU supports AVX2; `a` and every row have the same length.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn dot_i8_avx2_x4(a: &[i8], rows: [&[i8]; 4]) -> [i32; 4] {
    let mut acc = [_mm256_setzero_si256(); 4];
    let mut i = 0;
    // SAFETY (all loads): `i + 16 <= len` for `a` and every row, whose lengths are equal.
    while i + 16 <= a.len() {
        let x = _mm256_cvtepi8_epi16(_mm_loadu_si128(a.as_ptr().add(i).cast()));
        for (s, row) in acc.iter_mut().zip(rows) {
            let y = _mm256_cvtepi8_epi16(_mm_loadu_si128(row.as_ptr().add(i).cast()));
            *s = _mm256_add_epi32(*s, _mm256_madd_epi16(x, y));
        }
        i += 16;
    }
    acc.map(|s| horizontal_sum(s))
}

/// [`super::dot_i4_inline`]: each 16 packed bytes become the 16 even and the 16 odd
/// coordinates of a 32-wide block, matched with the query laid out by
/// [`super::interleave_for_i4`].
///
/// # Safety
/// The CPU supports AVX2; `q.len() == 2 * p.len()` and `p.len()` is a multiple of 16.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn dot_i4_avx2(q: &[i8], p: &[i8]) -> i32 {
    let mask = _mm_set1_epi8(0x0f);
    let eight = _mm_set1_epi8(8);
    let mut acc = _mm256_setzero_si256();
    let mut i = 0;
    // SAFETY (all loads): the caller checked `q.len() == 2 * p.len()` and `p.len() % 16 == 0`,
    // so `i + 16 <= p.len()` and `2 * i + 32 <= q.len()`.
    while i + 16 <= p.len() {
        let x = _mm_loadu_si128(p.as_ptr().add(i).cast());
        // Sign-extend each nibble: ((n & 15) ^ 8) - 8.
        let lo = _mm_sub_epi8(_mm_xor_si128(_mm_and_si128(x, mask), eight), eight);
        let hi = _mm_sub_epi8(
            _mm_xor_si128(_mm_and_si128(_mm_srli_epi16::<4>(x), mask), eight),
            eight,
        );
        let qe = _mm256_cvtepi8_epi16(_mm_loadu_si128(q.as_ptr().add(2 * i).cast()));
        let qo = _mm256_cvtepi8_epi16(_mm_loadu_si128(q.as_ptr().add(2 * i + 16).cast()));
        acc = _mm256_add_epi32(acc, _mm256_madd_epi16(_mm256_cvtepi8_epi16(lo), qe));
        acc = _mm256_add_epi32(acc, _mm256_madd_epi16(_mm256_cvtepi8_epi16(hi), qo));
        i += 16;
    }
    horizontal_sum(acc)
}

/// [`dot_i4_avx2`] of four rows against one query: the query's widened halves are loaded
/// once per 16 packed bytes and each row keeps its own accumulator, so the four row loads
/// are in flight together. Lane by lane the same integer sums as four single calls.
///
/// # Safety
/// The CPU supports AVX2; the rows have one length, a multiple of 16, and `q.len()` is
/// twice it.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn dot_i4_avx2_x4(q: &[i8], rows: [&[i8]; 4]) -> [i32; 4] {
    let mask = _mm_set1_epi8(0x0f);
    let eight = _mm_set1_epi8(8);
    let mut acc = [_mm256_setzero_si256(); 4];
    let mut i = 0;
    // SAFETY (all loads): the caller checked that every row has the same length, a
    // multiple of 16, and that `q.len()` is twice it, so `i + 16 <= row.len()` and
    // `2 * i + 32 <= q.len()`.
    while i + 16 <= rows[0].len() {
        let qe = _mm256_cvtepi8_epi16(_mm_loadu_si128(q.as_ptr().add(2 * i).cast()));
        let qo = _mm256_cvtepi8_epi16(_mm_loadu_si128(q.as_ptr().add(2 * i + 16).cast()));
        for (a, row) in acc.iter_mut().zip(rows) {
            let x = _mm_loadu_si128(row.as_ptr().add(i).cast());
            let lo = _mm_sub_epi8(_mm_xor_si128(_mm_and_si128(x, mask), eight), eight);
            let hi = _mm_sub_epi8(
                _mm_xor_si128(_mm_and_si128(_mm_srli_epi16::<4>(x), mask), eight),
                eight,
            );
            *a = _mm256_add_epi32(*a, _mm256_madd_epi16(_mm256_cvtepi8_epi16(lo), qe));
            *a = _mm256_add_epi32(*a, _mm256_madd_epi16(_mm256_cvtepi8_epi16(hi), qo));
        }
        i += 16;
    }
    [
        horizontal_sum(acc[0]),
        horizontal_sum(acc[1]),
        horizontal_sum(acc[2]),
        horizontal_sum(acc[3]),
    ]
}

/// [`dot_i8_avx2`] with `vpdpbusd` on 256-bit registers (AVX512-VNNI with VL): `a` is biased
/// by 128 into the unsigned operand, so `sum (a + 128) b = a.b + 128 sum b`, and a second
/// `vpdpbusd` against ones yields `sum b` to subtract. Exact over the whole i8 range. The
/// 512-bit forms are not used: they measured no faster on 128 and 256-wide rows.
///
/// # Safety
/// The CPU supports AVX512F, AVX512VL, AVX512-VNNI and AVX2; `a` and `b` have the same
/// length, a multiple of 16.
#[target_feature(enable = "avx512f,avx512vl,avx512vnni,avx2")]
pub(super) unsafe fn dot_i8_vnni(a: &[i8], b: &[i8]) -> i32 {
    let (flip, ones) = (_mm256_set1_epi8(-128), _mm256_set1_epi8(1));
    let (mut acc, mut sum_b) = (_mm256_setzero_si256(), _mm256_setzero_si256());
    let mut i = 0;
    // SAFETY (all loads): `i + 32 <= len` in the wide loop, `i + 16 <= len` after it.
    while i + 32 <= a.len() {
        let x = _mm256_xor_si256(_mm256_loadu_si256(a.as_ptr().add(i).cast()), flip);
        let y = _mm256_loadu_si256(b.as_ptr().add(i).cast());
        acc = _mm256_dpbusd_epi32(acc, x, y);
        sum_b = _mm256_dpbusd_epi32(sum_b, ones, y);
        i += 32;
    }
    let mut total = horizontal_sum(acc) - 128 * horizontal_sum(sum_b);
    if i < a.len() {
        total += super::x86::dot_i8_avx2(&a[i..], &b[i..]);
    }
    total
}

/// [`dot_i4_avx2`] with `vpdpbusd`: the nibbles are the unsigned operand, offset so that
/// `n ^ 8` is the signed nibble plus 8, in 0..=15; the query is the signed one. Then
/// `sum c q = sum (c + 8) q - 8 sum q`, with `sum q` from `vpdpbusd` against ones.
///
/// # Safety
/// The CPU supports AVX512F, AVX512VL, AVX512-VNNI and AVX2; `q.len() == 2 * p.len()` and
/// `p.len()` is a multiple of 16.
#[target_feature(enable = "avx512f,avx512vl,avx512vnni,avx2")]
pub(super) unsafe fn dot_i4_vnni(q: &[i8], p: &[i8]) -> i32 {
    let mask = _mm_set1_epi8(0x0f);
    let eight = _mm_set1_epi8(8);
    let ones = _mm_set1_epi8(1);
    let (mut acc, mut sum_q) = (_mm_setzero_si128(), _mm_setzero_si128());
    let mut i = 0;
    // SAFETY (all loads): as in `dot_i4_avx2`.
    while i + 16 <= p.len() {
        let x = _mm_loadu_si128(p.as_ptr().add(i).cast());
        let lo = _mm_xor_si128(_mm_and_si128(x, mask), eight);
        let hi = _mm_xor_si128(_mm_and_si128(_mm_srli_epi16::<4>(x), mask), eight);
        let qe = _mm_loadu_si128(q.as_ptr().add(2 * i).cast());
        let qo = _mm_loadu_si128(q.as_ptr().add(2 * i + 16).cast());
        acc = _mm_dpbusd_epi32(acc, lo, qe);
        acc = _mm_dpbusd_epi32(acc, hi, qo);
        sum_q = _mm_dpbusd_epi32(sum_q, ones, qe);
        sum_q = _mm_dpbusd_epi32(sum_q, ones, qo);
        i += 16;
    }
    let h = |v: __m128i| {
        let s = _mm_add_epi32(v, _mm_shuffle_epi32::<0b01_00_11_10>(v));
        _mm_cvtsi128_si32(_mm_add_epi32(s, _mm_shuffle_epi32::<0b10_11_00_01>(s)))
    };
    h(acc) - 8 * h(sum_q)
}

/// # Safety
/// The CPU supports AVX2.
#[target_feature(enable = "avx2")]
unsafe fn horizontal_sum(v: __m256i) -> i32 {
    let s = _mm_add_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b01_00_11_10>(s));
    let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b10_11_00_01>(s));
    _mm_cvtsi128_si32(s)
}
