// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Padding of a 3-byte neighbour list.
pub(super) const NONE3: u32 = 0x00FF_FFFF;

#[inline]
/// LEB128 varint (7 bits per byte, low bits first).
pub(super) fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Decodes one LEB128 varint; returns the value and the bytes used.
#[inline]
pub(super) fn get_varint(bytes: &[u8]) -> (u64, usize) {
    let mut v = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        v |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return (v, i + 1);
        }
    }
    (v, bytes.len())
}

pub(super) fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

pub(super) fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

pub(super) fn prefetch_u8(bytes: &[u8]) {
    // SAFETY: any initialized `u8` slice is also initialized `i8`s.
    prefetch_bytes(unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<i8>(), bytes.len()) });
}

#[inline]
pub(super) fn prefetch_f32(x: &f32) {
    // SAFETY: an initialized `f32` is four initialized bytes.
    prefetch_bytes(unsafe { std::slice::from_raw_parts((x as *const f32).cast::<i8>(), 4) });
}

#[inline]
pub(super) fn prefetch_ids(ids: &[u32]) {
    // SAFETY: any initialized `u32` slice is also `4 * len` initialized bytes.
    prefetch_bytes(unsafe { std::slice::from_raw_parts(ids.as_ptr().cast::<i8>(), ids.len() * 4) });
}

/// Orders by distance, then id; `f32` order is kept for every non-NaN distance.
#[inline]
pub(super) fn key(dist: f32, id: u32) -> u64 {
    let b = dist.to_bits();
    let o = if b >> 31 == 1 { !b } else { b | 0x8000_0000 };
    (u64::from(o) << 32) | u64::from(id)
}

#[inline]
pub(super) fn key_id(k: u64) -> u32 {
    k as u32
}
