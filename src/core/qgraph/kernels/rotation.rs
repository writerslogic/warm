// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Random orthogonal transform: zero-pad to a power of two (at least 64), then three rounds
/// of random sign flips followed by a fast Walsh-Hadamard transform. Not Haar-distributed,
/// but close enough for the RaBitQ estimator in practice (checked by the bias test).
pub(crate) struct Rotation {
    pub(super) dim: usize,
    pub(super) padded: usize,
    pub(super) signs: Vec<f32>,
    pub(super) scale: f32,
}

impl Rotation {
    /// `bits` is the requested output length (0 = automatic); see `BuildParams::code_bits`.
    pub(crate) fn new(dim: usize, bits: usize, seed: u64) -> Self {
        let padded = dim.max(bits).next_power_of_two().max(64);
        let mut rng = SplitMix(seed);
        let signs = (0..ROUNDS * padded)
            .map(|_| if rng.next_u64() & 1 == 1 { 1.0 } else { -1.0 })
            .collect();
        Self {
            dim,
            padded,
            signs,
            scale: (padded as f32).powf(-0.5 * ROUNDS as f32),
        }
    }

    pub(crate) fn padded(&self) -> usize {
        self.padded
    }

    #[cfg(feature = "experimental")]
    pub(crate) fn index_bytes(&self) -> usize {
        self.signs.len() * std::mem::size_of::<f32>()
    }

    pub(crate) fn apply(&self, x: &[f32], out: &mut [f32]) {
        assert_eq!(x.len(), self.dim);
        assert_eq!(out.len(), self.padded);
        out[..self.dim].copy_from_slice(x);
        out[self.dim..].fill(0.0);
        for signs in self.signs.chunks_exact(self.padded) {
            for (o, s) in out.iter_mut().zip(signs) {
                *o *= s;
            }
            fwht(out);
        }
        for o in out.iter_mut() {
            *o *= self.scale;
        }
    }
}

/// Unnormalized in-place Walsh-Hadamard transform; `x.len()` must be a power of two.
pub(crate) fn fwht<T>(x: &mut [T])
where
    T: Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T>,
{
    let mut h = 1;
    if x.len() >= 8 {
        // Stages 1, 2 and 4 on each block of eight in registers: the same butterflies in the
        // same order as the loop below, without its one-, two- and four-wide inner loops.
        for b in x.as_chunks_mut::<8>().0 {
            let [a0, a1, a2, a3, a4, a5, a6, a7] = *b;
            let (b0, b1, b2, b3) = (a0 + a1, a0 - a1, a2 + a3, a2 - a3);
            let (b4, b5, b6, b7) = (a4 + a5, a4 - a5, a6 + a7, a6 - a7);
            let (c0, c2, c1, c3) = (b0 + b2, b0 - b2, b1 + b3, b1 - b3);
            let (c4, c6, c5, c7) = (b4 + b6, b4 - b6, b5 + b7, b5 - b7);
            *b = [
                c0 + c4,
                c1 + c5,
                c2 + c6,
                c3 + c7,
                c0 - c4,
                c1 - c5,
                c2 - c6,
                c3 - c7,
            ];
        }
        h = 8;
    }
    while h < x.len() {
        for chunk in x.chunks_exact_mut(2 * h) {
            let (a, b) = chunk.split_at_mut(h);
            for (u, v) in a.iter_mut().zip(b.iter_mut()) {
                let t = *u;
                *u = t + *v;
                *v = t - *v;
            }
        }
        h *= 2;
    }
}

#[must_use]
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = [0f32; 16];
    let ((ca, ta), (cb, tb)) = (a.as_chunks::<16>(), b.as_chunks::<16>());
    let tail: f32 = ta.iter().zip(tb).map(|(x, y)| x * y).sum();
    for (x, y) in ca.iter().zip(cb) {
        for ((s, p), q) in acc.iter_mut().zip(x).zip(y) {
            *s += p * q;
        }
    }
    acc.iter().sum::<f32>() + tail
}
