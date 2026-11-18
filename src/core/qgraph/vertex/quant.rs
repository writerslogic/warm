// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Seed of the screen's rotation (fixed: the screen is a property of the encoding).
pub(super) const SCREEN_SEED: u64 = 0x5C12_EE00;
/// Seed of the neighbour-block rotation.
pub(super) const BLOCK_SEED: u64 = 0xB10C_5CA0;

/// Bytes of one block record: codes, then 32 scales, then 32 offsets (little-endian f32).
pub(super) fn block_record_bytes(m: usize) -> usize {
    16 * m + 8 * BLOCK
}

/// Level of coordinate `i` of a block code (see [`encode_block_code`]).
#[inline]
pub(super) fn block_level(nibbles: &[u8], bits: u8, i: usize) -> f32 {
    match bits {
        1 => {
            if nibbles[i / 4] >> (i % 4) & 1 == 1 {
                1.0
            } else {
                -1.0
            }
        }
        2 => 2.0 * f32::from(nibbles[i / 2] >> (2 * (i % 2)) & 3) - 3.0,
        _ => f32::from(nibbles[i]) - 7.0,
    }
}

/// Little-endian f32 number `i` of `bytes`.
#[inline]
pub(super) fn le_f32(bytes: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([
        bytes[4 * i],
        bytes[4 * i + 1],
        bytes[4 * i + 2],
        bytes[4 * i + 3],
    ])
}

/// Encodes a rotated, centred vector as `m` nibbles for a `bits`-per-coordinate block code
/// and returns its scale, so that coordinate `i ~= scale * level(i)` with the levels
/// `{-1, 1}` (1 bit), `{-3, -1, 1, 3}` (2 bits) or `-7..=7` (4 bits; nibble `level + 7`).
/// The levels of the 2- and 4-bit codes are chosen with the scale among eight candidates
/// that minimizes the squared reconstruction error. The returned scale, for every width, is
/// `|x|^2 / <levels, x>`: it makes the code error orthogonal to `x` itself (for one bit this
/// is the RaBitQ factor `|x|^2 / sum |x_i|`), so inner products with queries that are
/// correlated with `x` are unbiased, where the least-squares scale underestimates them by
/// the relative error energy.
pub(super) fn encode_block_code(x: &[f32], bits: u8, nibbles: &mut [u8]) -> f32 {
    nibbles.fill(0);
    let m = x.iter().fold(0f32, |a, &v| a.max(v.abs()));
    if m == 0.0 {
        return 0.0;
    }
    match bits {
        1 => {
            let (mut norm2, mut abs) = (0f32, 0f32);
            for (i, &v) in x.iter().enumerate() {
                norm2 += v * v;
                abs += v.abs();
                if v >= 0.0 {
                    nibbles[i / 4] |= 1 << (i % 4);
                }
            }
            norm2 / abs
        }
        2 => {
            // Levels {-3, -1, 1, 3} * s; index t = (level + 3) / 2.
            let err = |s: f32| -> f32 {
                x.iter()
                    .map(|&v| {
                        let t = ((v / s + 3.0) / 2.0).round().clamp(0.0, 3.0);
                        let d = v - s * (2.0 * t - 3.0);
                        d * d
                    })
                    .sum()
            };
            let (mut best, mut best_err) = (m / 3.0, f32::INFINITY);
            for k in 0..8 {
                let s = m / 3.0 * (1.0 - 0.1 * k as f32);
                let e = err(s);
                if e < best_err {
                    (best, best_err) = (s, e);
                }
            }
            let mut dot = 0f32;
            for (i, &v) in x.iter().enumerate() {
                let t = ((v / best + 3.0) / 2.0).round().clamp(0.0, 3.0) as u8;
                nibbles[i / 2] |= t << (2 * (i % 2));
                dot += v * (2.0 * f32::from(t) - 3.0);
            }
            unbiased_scale(x, dot, best)
        }
        _ => {
            let err = |s: f32| -> f32 {
                x.iter()
                    .map(|&v| {
                        let d = v - s * (v / s).round().clamp(-7.0, 7.0);
                        d * d
                    })
                    .sum()
            };
            let (mut best, mut best_err) = (m / 7.0, f32::INFINITY);
            for k in 0..8 {
                let s = m / 7.0 * (1.0 - 0.06 * k as f32);
                let e = err(s);
                if e < best_err {
                    (best, best_err) = (s, e);
                }
            }
            let mut dot = 0f32;
            for (i, &v) in x.iter().enumerate() {
                let level = (v / best).round().clamp(-7.0, 7.0);
                nibbles[i] = (level as i8 + 7) as u8;
                dot += v * level;
            }
            unbiased_scale(x, dot, best)
        }
    }
}

/// `|x|^2 / <levels, x>` (see [`encode_block_code`]), or the least-squares scale when the
/// levels do not correlate with `x`.
pub(super) fn unbiased_scale(x: &[f32], dot: f32, fallback: f32) -> f32 {
    let norm2: f32 = x.iter().map(|v| v * v).sum();
    if dot > 0.0 {
        norm2 / dot
    } else {
        fallback
    }
}

/// Fills the float lookup table `t` (`16 * m` entries) of a rotated query for `bits`-per-
/// coordinate block codes: `t[16 * j + v] = sum over the coordinates of nibble j of
/// q_i * level_i(v)`.
pub(super) fn block_table(q: &[f32], bits: u8, m: usize, t: &mut [f32]) {
    match bits {
        1 => {
            for j in 0..m {
                let qs = &q[4 * j..4 * j + 4];
                let total: f32 = qs.iter().sum();
                for v in 0..16usize {
                    let on: f32 = (0..4).filter(|b| v >> b & 1 == 1).map(|b| qs[b]).sum();
                    t[16 * j + v] = 2.0 * on - total;
                }
            }
        }
        2 => {
            for j in 0..m {
                let (qa, qb) = (q[2 * j], q[2 * j + 1]);
                for v in 0..16usize {
                    let (ta, tb) = ((v & 3) as f32, (v >> 2) as f32);
                    t[16 * j + v] = qa * (2.0 * ta - 3.0) + qb * (2.0 * tb - 3.0);
                }
            }
        }
        _ => {
            for j in 0..m {
                for v in 0..16usize {
                    t[16 * j + v] = q[j] * (v as f32 - 7.0);
                }
            }
        }
    }
}

/// Per-query block lookup table: `lut` as the scan reads it, with the affine map
/// `entry ~= delta * lut + min_j` so that a code's inner product is
/// `scale * (delta * sum + min_sum)`.
pub struct BlockQuery {
    pub(super) table: Vec<f32>,
    pub(super) lut: Vec<u8>,
    /// Per group, the minimum of its table entries.
    pub(super) mins: Vec<f32>,
    pub(super) delta: f32,
    pub(super) min_sum: f32,
    /// `1 / s_q`, which puts the block estimate in [`VGraph::est`] units.
    pub(super) inv_sq: f32,
}

impl BlockQuery {
    pub(super) fn new(m: usize) -> Self {
        Self {
            table: vec![0.0; 16 * m],
            lut: vec![0; 16 * m],
            mins: vec![0.0; m],
            delta: 1.0,
            min_sum: 0.0,
            inv_sq: 1.0,
        }
    }

    /// Quantizes the float table to bytes: per group, entries are offset by the group's
    /// minimum; one step `delta` serves every group.
    pub(super) fn quantize(&mut self, m: usize) {
        debug_assert_eq!(self.lut.len(), 16 * m);
        debug_assert_eq!(self.mins.len(), m);
        let mut range = 0f32;
        let mut min_sum = 0f32;
        let groups = self.table.as_chunks::<16>().0;
        for (g, lo) in groups.iter().zip(self.mins.iter_mut()) {
            *lo = g.iter().fold(f32::INFINITY, |a, &v| a.min(v));
            let hi = g.iter().fold(f32::NEG_INFINITY, |a, &v| a.max(v));
            min_sum += *lo;
            range = range.max(hi - *lo);
        }
        let delta = if range > 0.0 { range / 255.0 } else { 1.0 };
        let luts = self.lut.as_chunks_mut::<16>().0;
        for ((g, &lo), lut) in groups.iter().zip(&self.mins).zip(luts) {
            for (o, &e) in lut.iter_mut().zip(g) {
                *o = ((e - lo) / delta).round().clamp(0.0, 255.0) as u8;
            }
        }
        self.delta = delta;
        self.min_sum = min_sum;
    }
}

/// Inverse of [`key`] for the distance part.
#[inline]
pub(super) fn key_dist(k: u64) -> f32 {
    let o = (k >> 32) as u32;
    f32::from_bits(if o >> 31 == 1 { o & 0x7FFF_FFFF } else { !o })
}

/// Writes the symmetric 8-bit code of `x` into `code` (zero padded) and returns its scale, so
/// that `x_i ~= scale * code_i`.
pub(in super::super) fn quantize(x: &[f32], code: &mut [i8]) -> f32 {
    let m = x.iter().fold(0f32, |a, &v| a.max(v.abs()));
    code.fill(0);
    if m == 0.0 {
        return 0.0;
    }
    let scale = m / 127.0;
    for (c, &v) in code.iter_mut().zip(x) {
        *c = (v / scale).round().clamp(-127.0, 127.0) as i8;
    }
    scale
}

/// [`quantize`] with 4-bit codes in `-7..=7`, packed two per byte as [`dot_f32_i4`] reads them
/// (`code` holds `code.len() * 2 >= x.len()` values).
pub(in super::super) fn quantize4(x: &[f32], code: &mut [i8]) -> f32 {
    let m = x.iter().fold(0f32, |a, &v| a.max(v.abs()));
    code.fill(0);
    if m == 0.0 {
        return 0.0;
    }
    let scale = m / 7.0;
    let q = |i: usize| {
        x.get(i)
            .map_or(0, |&v| (v / scale).round().clamp(-7.0, 7.0) as i8)
    };
    for (j, c) in code.iter_mut().enumerate() {
        *c = ((q(2 * j) as u8 & 0x0F) | ((q(2 * j + 1) as u8) << 4)) as i8;
    }
    scale
}

/// Coordinate `i` of a code row of `bits` bits (0 past its end).
pub(in super::super) fn code_at(c: &[i8], bits: u8, i: usize) -> i8 {
    if bits == 8 {
        c.get(i).copied().unwrap_or(0)
    } else {
        c.get(i / 2).map_or(0, |&b| {
            if i.is_multiple_of(2) {
                (((b as u8) << 4) as i8) >> 4
            } else {
                b >> 4
            }
        })
    }
}
