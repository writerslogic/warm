// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[inline]
pub(super) fn shift(v: u8) -> i8 {
    (v ^ 0x80) as i8
}

#[inline]
pub(super) fn prefetch_word(v: &u64) {
    // SAFETY: a prefetch hint never faults or writes; the pointer comes from a live reference.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
            std::ptr::from_ref(v).cast::<i8>(),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = v;
}

#[inline]
pub(super) fn prefetch_norm(v: &i32) {
    // SAFETY: a prefetch hint never faults or writes; the pointer comes from a live reference.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
            std::ptr::from_ref(v).cast::<i8>(),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = v;
}

#[inline]
pub(super) fn bit(words: &[u64], id: u32) -> bool {
    words[(id >> 6) as usize] >> (id & 63) & 1 == 1
}

/// The `k` best `(key, id)` pairs seen, kept sorted ascending.
#[derive(Default)]
pub(super) struct TopK {
    pub(super) k: usize,
    pub(super) items: Vec<(i64, u32)>,
}

impl TopK {
    #[inline]
    pub(super) fn worst(&self) -> (i64, u32) {
        if self.items.len() < self.k {
            (i64::MAX, u32::MAX)
        } else {
            self.items[self.k - 1]
        }
    }

    #[inline]
    pub(super) fn push(&mut self, key: i64, id: u32) {
        if self.k == 0 || (key, id) >= self.worst() {
            return;
        }
        let at = self.items.partition_point(|&e| e < (key, id));
        self.items.insert(at, (key, id));
        self.items.truncate(self.k);
    }
}

#[derive(Default)]
pub(super) struct IvfScratch {
    pub(super) order: Vec<(i64, u32)>,
    pub(super) batch: Vec<u32>,
    pub(super) cursors: Vec<usize>,
}

#[derive(Default)]
pub(super) struct SearchScratch {
    pub(super) labels: Vec<u32>,
    pub(super) query: Vec<i8>,
    pub(super) top: TopK,
    pub(super) matches: Vec<u32>,
    /// The sorted labels whose intersection `matches` holds (empty when it holds none).
    pub(super) matches_for: Vec<u32>,
    pub(super) cursors: Vec<usize>,
    pub(super) ivf: IvfScratch,
    pub(super) fs: FsScratch,
}

/// Splitmix64, for deterministic k-means sampling.
pub(super) fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
