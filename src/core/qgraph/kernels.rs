// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Inner loops of the quantized graph: rotation, float dot product, and the bit-plane
//! popcount that turns 1-bit edge codes into inner-product estimates.

/// Bits per query coordinate in the bit-plane estimator.
const QUERY_BITS: usize = 4;
const ROUNDS: usize = 3;

/// SplitMix64: small, seedable and stable across platforms and crate versions.
pub(crate) struct SplitMix(pub(crate) u64);

impl SplitMix {
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    #[cfg(test)]
    pub(crate) fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

pub(crate) fn as_f32(w: &[u64]) -> &[f32] {
    bytemuck::cast_slice(w)
}

pub(crate) fn as_f32_mut(w: &mut [u64]) -> &mut [f32] {
    bytemuck::cast_slice_mut(w)
}

pub(crate) fn as_u32(w: &[u64]) -> &[u32] {
    bytemuck::cast_slice(w)
}

pub(crate) fn as_u32_mut(w: &mut [u64]) -> &mut [u32] {
    bytemuck::cast_slice_mut(w)
}

mod dots;
#[cfg(target_arch = "aarch64")]
mod neon;
mod popcount;
mod prefetch;
mod rotation;
#[cfg(test)]
mod tests;
#[cfg(target_arch = "x86_64")]
mod tier;
#[cfg(target_arch = "x86_64")]
mod x86;
pub(crate) use self::dots::*;
pub(crate) use self::popcount::*;
pub(crate) use self::prefetch::*;
pub(crate) use self::rotation::*;
