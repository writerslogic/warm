// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Holographic Bloom Memory.
//!
//! Set membership over a fixed-size bit bundle. Items are sparse vectors; the
//! bundle is the OR-union of their active indices, which is exactly a Bloom
//! filter of `dim` bits whose hash functions are the item's own active indices.
//! Readout is density-corrected containment: (raw - density) / (1 - density).
//!
//! # Probes per item
//!
//! A Bloom filter holding `n` items in `m` bits is only usable when the probe
//! count is near `ln 2 * m / n`. An item carries `dim / 256` active indices
//! (64 at `dim = 16384`), which is optimal only near `n = 177`; beyond that the
//! union saturates and membership falls to chance (AUC 0.58 at `n = 1,500`).
//! The memory therefore reads and writes only the first `probes` of each item's
//! active indices, ordered by a hash keyed on the whole item, where
//! `probes = clamp(round(ln 2 * dim / expected_items), 1, item size)`.
//! The choice is a pure function of the item and `probes`, so insert and query
//! agree and members always score 1.0. When `probes` equals the item size the
//! behaviour is identical to the untruncated union.
//!
//! The union is a Bloom filter of `dim` bits. At equal bits and tuned probe
//! count it is the same structure as a plain Bloom filter, not a better one.
//! `probes` is fixed at construction because earlier inserts used it.

use crate::core::entangled::{hash_u64, EntangledHVec};

const PROBE_ORDER_SEED: u64 = 0x5EED_B100_u64;

/// Probe count `round(ln 2 * dim / expected_items)`, at least 1.
pub fn probes_for(dim: usize, expected_items: usize) -> usize {
    let n = expected_items.max(1) as f64;
    ((std::f64::consts::LN_2 * dim as f64 / n).round() as usize).max(1)
}

/// Item count the default constructor sizes for: about 11 bits per item.
pub fn default_expected_items(dim: usize) -> usize {
    (dim / 11).max(1)
}

pub struct HolographicBloomMemory {
    dim: usize,
    bits: Vec<u64>,
    ones: usize,
    probes: usize,
    expected_items: usize,
    item_count: usize,
}

impl HolographicBloomMemory {
    /// Sized for [`default_expected_items`] items (about 1,489 at `dim = 16384`).
    pub fn new(dim: usize) -> Self {
        Self::with_expected_items(dim, default_expected_items(dim))
    }

    /// Chooses the probe count from the number of items expected to be stored.
    /// Membership quality degrades past `expected_items`; see the capacity doc.
    pub fn with_expected_items(dim: usize, expected_items: usize) -> Self {
        Self {
            dim,
            bits: vec![0; dim.div_ceil(64)],
            ones: 0,
            probes: probes_for(dim, expected_items),
            expected_items: expected_items.max(1),
            item_count: 0,
        }
    }

    /// The first `probes` active indices of `item` in hash order.
    fn probe_set(&self, item: &EntangledHVec) -> Vec<u32> {
        let mut ix: Vec<u32> = item
            .indices()
            .iter()
            .copied()
            .filter(|&i| (i as usize) < self.dim)
            .collect();
        if ix.len() > self.probes {
            // Keyed by the whole item so the chosen positions are uniform over the
            // bundle; a per-index key would favour the same low-hash positions
            // for every item and saturate them.
            let fp = item
                .indices()
                .iter()
                .fold(PROBE_ORDER_SEED, |h, &i| hash_u64(h, i as u64));
            let key = |i: &u32| hash_u64(fp, *i as u64);
            ix.select_nth_unstable_by_key(self.probes - 1, key);
            ix.truncate(self.probes);
        }
        ix
    }

    fn set(&mut self, item: &EntangledHVec) {
        for i in self.probe_set(item) {
            let (w, b) = (i as usize / 64, i as usize % 64);
            if self.bits[w] >> b & 1 == 0 {
                self.bits[w] |= 1 << b;
                self.ones += 1;
            }
        }
        self.item_count += 1;
    }

    pub fn insert(&mut self, item: &EntangledHVec) {
        self.set(item);
    }

    pub fn insert_batch(&mut self, items: &[EntangledHVec]) {
        for item in items {
            self.set(item);
        }
    }

    /// Density-corrected containment score.
    pub fn contains(&self, item: &EntangledHVec) -> f64 {
        let ix = self.probe_set(item);
        if ix.is_empty() {
            return 1.0;
        }
        let hit = ix
            .iter()
            .filter(|&&i| self.bits[i as usize / 64] >> (i as usize % 64) & 1 == 1)
            .count();
        let raw = hit as f64 / ix.len() as f64;
        let density = self.density();
        if density >= 1.0 - 1e-15 {
            return 0.0;
        }
        ((raw - density) / (1.0 - density)).max(0.0)
    }

    pub fn query_candidates(
        &self,
        candidates: &[EntangledHVec],
        threshold: f64,
    ) -> Vec<(usize, f64)> {
        candidates
            .iter()
            .enumerate()
            .map(|(i, item)| (i, self.contains(item)))
            .filter(|&(_, score)| score >= threshold)
            .collect()
    }

    pub fn item_count(&self) -> usize {
        self.item_count
    }
    pub fn dim(&self) -> usize {
        self.dim
    }
    pub fn probes(&self) -> usize {
        self.probes
    }
    pub fn expected_items(&self) -> usize {
        self.expected_items
    }

    pub fn density(&self) -> f64 {
        self.ones as f64 / self.dim.max(1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(dim: usize, base: u64, n: usize) -> Vec<EntangledHVec> {
        (0..n as u64)
            .map(|i| EntangledHVec::new_deterministic(dim, base + i * 7919))
            .collect()
    }

    /// Mann-Whitney AUC, ties count 0.5.
    fn auc(pos: &[f64], neg: &[f64]) -> f64 {
        let mut wins = 0.0;
        for &p in pos {
            for &n in neg {
                wins += if p > n {
                    1.0
                } else if p == n {
                    0.5
                } else {
                    0.0
                };
            }
        }
        wins / (pos.len() * neg.len()) as f64
    }

    fn membership_auc(mem: &mut HolographicBloomMemory, n: usize, base: u64) -> f64 {
        let dim = mem.dim();
        let stored = items(dim, base, n);
        mem.insert_batch(&stored);
        let pos: Vec<f64> = stored.iter().take(200).map(|i| mem.contains(i)).collect();
        let neg: Vec<f64> = items(dim, base + 900_000_007, 200)
            .iter()
            .map(|i| mem.contains(i))
            .collect();
        auc(&pos, &neg)
    }

    #[test]
    fn test_exact_member_retrieval() {
        let dim = 16384;
        let mut mem = HolographicBloomMemory::new(dim);
        let items = items(dim, 0, 50);
        mem.insert_batch(&items);
        for (i, item) in items.iter().enumerate() {
            let score = mem.contains(item);
            assert!(
                (score - 1.0).abs() < 1e-10,
                "Member {} should have score 1.0, got {:.6}",
                i,
                score
            );
        }
    }

    #[test]
    fn test_non_member_discrimination() {
        let dim = 16384;
        let mut mem = HolographicBloomMemory::new(dim);
        mem.insert_batch(&items(dim, 0, 100));
        let max_score = items(dim, 999_000, 100)
            .iter()
            .map(|i| mem.contains(i))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max_score < 0.5,
            "Non-member max score should be <0.5, got {:.4}",
            max_score
        );
    }

    #[test]
    fn test_capacity_200_items() {
        let dim = 16384;
        let mut mem = HolographicBloomMemory::new(dim);
        let stored = items(dim, 0, 200);
        mem.insert_batch(&stored);
        assert!(stored.iter().all(|i| mem.contains(i) > 0.5));
        let fp = items(dim, 777_000, 200)
            .iter()
            .filter(|i| mem.contains(i) > 0.5)
            .count();
        assert!(
            fp < 5,
            "False positives at 200 items should be <5, got {fp}"
        );
    }

    #[test]
    fn test_capacity_500_items() {
        let dim = 16384;
        let mut mem = HolographicBloomMemory::new(dim);
        let stored = items(dim, 0, 500);
        mem.insert_batch(&stored);
        let found = stored.iter().filter(|i| mem.contains(i) > 0.3).count();
        assert!(found as f64 / 500.0 > 0.95);
    }

    /// With probes equal to the item size the memory must answer exactly as the
    /// untruncated union of the items did before probes existed.
    #[test]
    fn test_full_probes_match_untruncated_union() {
        let dim = 16384;
        let stored = items(dim, 0, 300);
        let item_k = stored[0].indices().len();
        let mut mem = HolographicBloomMemory::with_expected_items(dim, 1);
        assert!(mem.probes() >= item_k);
        mem.insert_batch(&stored);
        let union = EntangledHVec::bundle_bloom(&stored);
        let queries: Vec<EntangledHVec> = stored
            .iter()
            .take(50)
            .cloned()
            .chain(items(dim, 555_000, 200))
            .collect();
        for q in &queries {
            let legacy = q.corrected_containment(&union);
            assert!((mem.contains(q) - legacy).abs() < 1e-12);
        }
    }

    #[test]
    fn test_probe_count_boundaries() {
        let dim = 16384;
        // ln2 * 16384 / n crosses 7.5 between n = 1514 and n = 1515.
        assert_eq!(probes_for(dim, 1514), 8);
        assert_eq!(probes_for(dim, 1515), 7);
        assert_eq!(probes_for(dim, usize::MAX), 1);
        assert_eq!(probes_for(dim, 0), probes_for(dim, 1));
        // Never more probes than the item has indices.
        let mut mem = HolographicBloomMemory::with_expected_items(dim, 10);
        let it = EntangledHVec::new_deterministic(dim, 1);
        mem.insert(&it);
        assert_eq!(mem.probe_set(&it).len(), it.indices().len());
    }

    /// Acceptance: AUC >= 0.99 at N = 1,500 for the default constructor and for
    /// the N-aware constructor, and at limit-1, limit, limit+1 of the sized N.
    #[test]
    fn test_membership_auc_at_1500() {
        let dim = 16384;
        for seed in 0..3u64 {
            let base = seed * 1_000_003;
            let a = membership_auc(&mut HolographicBloomMemory::new(dim), 1500, base);
            assert!(a >= 0.99, "default constructor, seed {seed}: AUC {a:.4}");
            for n in [1499usize, 1500, 1501] {
                let mut mem = HolographicBloomMemory::with_expected_items(dim, 1500);
                let a = membership_auc(&mut mem, n, base);
                assert!(a >= 0.99, "expected 1500, n={n}, seed {seed}: AUC {a:.4}");
            }
        }
    }

    /// The failure this replaces: 64 probes at N = 1,500 is near chance.
    #[test]
    fn test_untuned_64_probes_is_near_chance_at_1500() {
        let dim = 16384;
        let mut mem = HolographicBloomMemory::with_expected_items(dim, 177);
        assert_eq!(mem.probes(), 64);
        assert!(membership_auc(&mut mem, 1500, 0) < 0.7);
    }
}
