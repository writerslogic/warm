// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

/// Appends `values`, each below `2^bits`, as a contiguous little-endian bit string.
pub(in super::super) fn pack(out: &mut Vec<u8>, values: impl Iterator<Item = u64>, bits: u8) {
    let bits = u32::from(bits);
    let (mut buffer, mut used) = (0u64, 0u32);
    for value in values {
        buffer |= value << used;
        let total = used + bits;
        if total >= 64 {
            out.extend_from_slice(&buffer.to_le_bytes());
            buffer = if used == 0 { 0 } else { value >> (64 - used) };
            used = total - 64;
        } else {
            used = total;
        }
    }
    out.extend_from_slice(&buffer.to_le_bytes()[..used.div_ceil(8) as usize]);
}

/// Widths that one unaligned 64-bit load holds together with its byte shift.
pub(super) const DIRECT_BITS: u8 = 56;

/// Calls `emit(index, value)` for the `count` values of `bits` bits at the start of `bytes`.
pub(super) fn unpack_into(bytes: &[u8], count: usize, bits: u8, mut emit: impl FnMut(usize, u64)) {
    if bits == 0 {
        for index in 0..count {
            emit(index, 0);
        }
        return;
    }
    if bits > DIRECT_BITS {
        let mut reader = BitReader::new(bytes, bits);
        for index in 0..count {
            emit(index, reader.next());
        }
        return;
    }
    let mask = (1u64 << bits) - 1;
    let mut bit = 0usize;
    for index in 0..count {
        let byte = bit >> 3;
        let word = match bytes.get(byte..byte + 8) {
            Some(window) => u64::from_le_bytes(window.try_into().expect("eight bytes")),
            None => {
                let mut window = [0u8; 8];
                let tail = &bytes[byte..];
                window[..tail.len()].copy_from_slice(tail);
                u64::from_le_bytes(window)
            }
        };
        emit(index, (word >> (bit & 7)) & mask);
        bit += usize::from(bits);
    }
}

/// `count` values of `BITS` bits from the start of `bytes` into `out`. With the width a
/// constant, the loop is straight-line loads, shifts and masks; values whose 8-byte window
/// would pass the end of `bytes` read it zero-padded.
#[inline(always)]
pub(super) fn unpack_fixed<const BITS: usize>(bytes: &[u8], count: usize, out: &mut [u64]) {
    let out = &mut out[..count];
    #[cfg(not(any(
        target_arch = "x86_64",
        all(target_arch = "aarch64", target_endian = "little")
    )))]
    let decoded = 0;
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    let decoded = if BITS <= 25 && count >= 32 && std::arch::is_aarch64_feature_detected!("neon") {
        // SAFETY: NEON was detected; the decoder bounds every input load and output store.
        unsafe { unpack_neon::<BITS>(bytes, out) }
    } else {
        0
    };
    #[cfg(target_arch = "x86_64")]
    let decoded = if BITS <= 25 && count >= 32 && is_x86_feature_detected!("avx2") {
        // SAFETY: AVX2 was detected; the decoder bounds every input load and output store.
        unsafe { unpack_avx2::<BITS>(bytes, out) }
    } else {
        0
    };
    let mask = (1u64 << BITS) - 1;
    let whole = match bytes.len().checked_sub(8) {
        Some(spare) => ((spare * 8 + 7) / BITS + 1).min(count),
        None => 0,
    };
    for (index, value) in out[..whole].iter_mut().enumerate().skip(decoded) {
        let bit = index * BITS;
        let window: [u8; 8] = bytes[bit / 8..bit / 8 + 8].try_into().expect("eight bytes");
        *value = (u64::from_le_bytes(window) >> (bit % 8)) & mask;
    }
    let tail = whole.max(decoded);
    if tail < count {
        let first_bit = tail * BITS;
        let mut window = [0u8; 8];
        let rest = &bytes[first_bit / 8..];
        assert!((count - 1) * BITS / 8 <= bytes.len());
        window[..rest.len()].copy_from_slice(rest);
        let word = u64::from_le_bytes(window);
        for (index, value) in out[tail..].iter_mut().enumerate() {
            *value = (word >> (first_bit % 8 + index * BITS)) & mask;
        }
    }
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
#[target_feature(enable = "neon")]
pub(super) unsafe fn unpack_neon<const BITS: usize>(bytes: &[u8], out: &mut [u64]) -> usize {
    use std::arch::aarch64::*;
    assert!((1..=25).contains(&BITS));
    let shuffle_lo = std::array::from_fn::<_, 16, _>(|i| ((i / 4) * BITS / 8 + i % 4) as u8);
    let shuffle_hi =
        std::array::from_fn::<_, 16, _>(|i| ((i / 4 + 4) * BITS / 8 - BITS / 2 + i % 4) as u8);
    let shifts_lo = std::array::from_fn::<_, 4, _>(|i| -((i * BITS % 8) as i32));
    let shifts_hi = std::array::from_fn::<_, 4, _>(|i| -(((i + 4) * BITS % 8) as i32));
    // SAFETY: tables span 16 bytes. Each iteration checks both 16-byte input
    // windows and eight output slots; NEON loads and stores permit unaligned pointers.
    unsafe {
        let shuffle_lo = vld1q_u8(shuffle_lo.as_ptr());
        let shuffle_hi = vld1q_u8(shuffle_hi.as_ptr());
        let shifts_lo = vld1q_s32(shifts_lo.as_ptr());
        let shifts_hi = vld1q_s32(shifts_hi.as_ptr());
        let mask = vdupq_n_u32(((1u64 << BITS) - 1) as u32);
        let mut done = 0;
        while done + 8 <= out.len() && done * BITS / 8 + BITS / 2 + 16 <= bytes.len() {
            let first = bytes.as_ptr().add(done * BITS / 8);
            let lo = vandq_u32(
                vshlq_u32(
                    vreinterpretq_u32_u8(vqtbl1q_u8(vld1q_u8(first), shuffle_lo)),
                    shifts_lo,
                ),
                mask,
            );
            let hi = vandq_u32(
                vshlq_u32(
                    vreinterpretq_u32_u8(vqtbl1q_u8(vld1q_u8(first.add(BITS / 2)), shuffle_hi)),
                    shifts_hi,
                ),
                mask,
            );
            vst1q_u64(out.as_mut_ptr().add(done), vmovl_u32(vget_low_u32(lo)));
            vst1q_u64(out.as_mut_ptr().add(done + 2), vmovl_u32(vget_high_u32(lo)));
            vst1q_u64(out.as_mut_ptr().add(done + 4), vmovl_u32(vget_low_u32(hi)));
            vst1q_u64(out.as_mut_ptr().add(done + 6), vmovl_u32(vget_high_u32(hi)));
            done += 8;
        }
        done
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(super) unsafe fn unpack_avx2<const BITS: usize>(bytes: &[u8], out: &mut [u64]) -> usize {
    use std::arch::x86_64::*;
    assert!((1..=25).contains(&BITS));
    let shuffle = std::array::from_fn::<_, 32, _>(|i| {
        let lane = i / 4;
        (lane * BITS / 8 - (lane / 4) * (BITS / 2) + i % 4) as u8
    });
    let shifts = std::array::from_fn::<_, 8, _>(|i| (i * BITS % 8) as u32);
    // SAFETY: lookup arrays span 32 bytes. Each iteration checks both 16-byte
    // input windows and eight output slots. Unaligned intrinsics impose no alignment.
    unsafe {
        let shuffle = _mm256_loadu_si256(shuffle.as_ptr().cast());
        let shifts = _mm256_loadu_si256(shifts.as_ptr().cast());
        let mask = _mm256_set1_epi32(((1u32 << BITS) - 1) as i32);
        let mut done = 0;
        while done + 8 <= out.len() && done * BITS / 8 + BITS / 2 + 16 <= bytes.len() {
            let first = bytes.as_ptr().add(done * BITS / 8);
            let lo = _mm_loadu_si128(first.cast());
            let hi = _mm_loadu_si128(first.add(BITS / 2).cast());
            let word = _mm256_inserti128_si256::<1>(_mm256_castsi128_si256(lo), hi);
            let values = _mm256_and_si256(
                _mm256_srlv_epi32(_mm256_shuffle_epi8(word, shuffle), shifts),
                mask,
            );
            _mm256_storeu_si256(
                out.as_mut_ptr().add(done).cast(),
                _mm256_cvtepu32_epi64(_mm256_castsi256_si128(values)),
            );
            _mm256_storeu_si256(
                out.as_mut_ptr().add(done + 4).cast(),
                _mm256_cvtepu32_epi64(_mm256_extracti128_si256::<1>(values)),
            );
            done += 8;
        }
        done
    }
}

/// `count` values of `bits` bits from the start of `bytes` into `out`: a constant-width
/// loop for widths up to 32, the general window loop above that.
pub(in super::super) fn unpack_values(bytes: &[u8], count: usize, bits: u8, out: &mut [u64]) {
    match bits {
        0 => out[..count].fill(0),
        1 => unpack_fixed::<1>(bytes, count, out),
        2 => unpack_fixed::<2>(bytes, count, out),
        3 => unpack_fixed::<3>(bytes, count, out),
        4 => unpack_fixed::<4>(bytes, count, out),
        5 => unpack_fixed::<5>(bytes, count, out),
        6 => unpack_fixed::<6>(bytes, count, out),
        7 => unpack_fixed::<7>(bytes, count, out),
        8 => unpack_fixed::<8>(bytes, count, out),
        9 => unpack_fixed::<9>(bytes, count, out),
        10 => unpack_fixed::<10>(bytes, count, out),
        11 => unpack_fixed::<11>(bytes, count, out),
        12 => unpack_fixed::<12>(bytes, count, out),
        13 => unpack_fixed::<13>(bytes, count, out),
        14 => unpack_fixed::<14>(bytes, count, out),
        15 => unpack_fixed::<15>(bytes, count, out),
        16 => unpack_fixed::<16>(bytes, count, out),
        17 => unpack_fixed::<17>(bytes, count, out),
        18 => unpack_fixed::<18>(bytes, count, out),
        19 => unpack_fixed::<19>(bytes, count, out),
        20 => unpack_fixed::<20>(bytes, count, out),
        21 => unpack_fixed::<21>(bytes, count, out),
        22 => unpack_fixed::<22>(bytes, count, out),
        23 => unpack_fixed::<23>(bytes, count, out),
        24 => unpack_fixed::<24>(bytes, count, out),
        25 => unpack_fixed::<25>(bytes, count, out),
        26 => unpack_fixed::<26>(bytes, count, out),
        27 => unpack_fixed::<27>(bytes, count, out),
        28 => unpack_fixed::<28>(bytes, count, out),
        29 => unpack_fixed::<29>(bytes, count, out),
        30 => unpack_fixed::<30>(bytes, count, out),
        31 => unpack_fixed::<31>(bytes, count, out),
        32 => unpack_fixed::<32>(bytes, count, out),
        _ => unpack_into(bytes, count, bits, |index, value| out[index] = value),
    }
}

pub(in super::super) fn unpack(bytes: &[u8], index: usize, bits: u8) -> u64 {
    if bits == 0 {
        return 0;
    }
    let bit = index * usize::from(bits);
    let start = bit / 8;
    let shift = bit % 8;
    if bits <= DIRECT_BITS {
        if let Some(window) = bytes.get(start..start + 8) {
            let word = u64::from_le_bytes(window.try_into().expect("eight bytes"));
            return (word >> shift) & ((1u64 << bits) - 1);
        }
    }
    let size = (shift + usize::from(bits)).div_ceil(8);
    let mut buffer = [0u8; 16];
    buffer[..size].copy_from_slice(&bytes[start..start + size]);
    let value = u128::from_le_bytes(buffer) >> shift;
    (value & ((1u128 << bits) - 1)) as u64
}

pub(super) struct BitReader<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) buffer: u128,
    pub(super) used: u8,
    pub(super) bits: u8,
    pub(super) mask: u128,
}

impl<'a> BitReader<'a> {
    pub(super) fn new(bytes: &'a [u8], bits: u8) -> Self {
        Self {
            bytes,
            buffer: 0,
            used: 0,
            bits,
            mask: (1u128 << bits) - 1,
        }
    }
    pub(super) fn next(&mut self) -> u64 {
        while self.used < self.bits {
            self.buffer |= u128::from(self.bytes[0]) << self.used;
            self.bytes = &self.bytes[1..];
            self.used += 8;
        }
        let value = (self.buffer & self.mask) as u64;
        self.buffer >>= self.bits;
        self.used -= self.bits;
        value
    }
}
