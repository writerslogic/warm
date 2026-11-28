// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Pre-weighted postings of a checkpoint that the unpruned ranker serves.
//!
//! A checkpoint of at most [`PRUNE_SLOT_FLOOR_SERVING`] slots is ranked by summing a BM25 weight
//! per posting. Under the default constants the weight of a posting is a function of the index
//! alone (the list's inverse document frequency, the posting's frequency and its slot's length
//! normalization), so the longest lists keep `(slot, weight)` pairs in the derived section and a
//! query adds them instead of decoding and weighing the list. The weights are computed by the
//! expression the exhaustive path evaluates, and are added in the same order, so scores are
//! bit-identical.
//!
//! Which lists: those with more than [`WEIGH_BLOCK`] postings, longest first, while their bytes
//! (16 for the entry head, 12 per posting) fit `max(DENSE_BYTES, min(DENSE_CAP, index_bytes /
//! DENSE_SHARE))`, the direct tables' budget rule. A list that does not fit, and every list of
//! a query under other BM25 constants, takes the weighing path.

use std::ops::Range;

use super::super::impact::{DENSE_BYTES, DENSE_CAP, DENSE_SHARE};
use super::super::search::{StoredWeights, PRUNE_SLOT_FLOOR_SERVING, WEIGH_BLOCK};

/// Bytes before a stored list's pairs: its postings offset and its inverse document frequency.
pub(super) const ENTRY_HEAD: usize = 16;
/// Bytes per posting: a `u32` slot and an `f64` weight.
pub(super) const POSTING_BYTES: usize = 12;

/// The rule that picks the lists, as one header word: a reader whose rule differs recomputes.
pub(super) fn rule() -> u64 {
    (WEIGH_BLOCK as u64)
        ^ (PRUNE_SLOT_FLOOR_SERVING as u64).rotate_left(17)
        ^ (ENTRY_HEAD as u64).rotate_left(41)
        ^ (POSTING_BYTES as u64).rotate_left(53)
}

/// The byte budget of the stored lists for an index of `index_bytes`.
pub(super) fn budget(index_bytes: usize) -> usize {
    DENSE_BYTES.max(DENSE_CAP.min(index_bytes / DENSE_SHARE))
}

/// The lists to store, from `(postings, offset)` of every list longer than [`WEIGH_BLOCK`]:
/// longest first (ties by offset), the longest prefix that fits [`budget`].
pub(super) fn select(mut lists: Vec<(usize, usize)>, index_bytes: usize) -> Vec<(usize, usize)> {
    lists.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let budget = budget(index_bytes);
    let mut used = 0usize;
    let fit = lists
        .iter()
        .take_while(|(count, _)| {
            let next = used.saturating_add(ENTRY_HEAD + count.saturating_mul(POSTING_BYTES));
            if next > budget {
                return false;
            }
            used = next;
            true
        })
        .count();
    lists.truncate(fit);
    lists
}

/// The slots then the weights of one list, little endian.
pub(super) enum Blob {
    Owned(Box<[u8]>),
    /// A range of the checkpoint's derived section, read in place.
    Stored(Range<usize>),
}

pub(super) struct Weighted {
    blob: Blob,
    count: usize,
}

impl Weighted {
    pub(super) fn computed(slots: &[u32], weights: &[f64]) -> Self {
        let mut bytes = Vec::with_capacity(slots.len() * POSTING_BYTES);
        for slot in slots {
            bytes.extend_from_slice(&slot.to_le_bytes());
        }
        for weight in weights {
            bytes.extend_from_slice(&weight.to_le_bytes());
        }
        Self {
            blob: Blob::Owned(bytes.into_boxed_slice()),
            count: slots.len(),
        }
    }

    pub(super) fn stored(range: Range<usize>, count: usize) -> Self {
        Self {
            blob: Blob::Stored(range),
            count,
        }
    }

    pub(super) fn count(&self) -> usize {
        self.count
    }

    /// The pair bytes; `section` is the derived section a stored list points into.
    pub(super) fn bytes<'a>(&'a self, section: &'a [u8]) -> &'a [u8] {
        match &self.blob {
            Blob::Owned(bytes) => bytes,
            Blob::Stored(range) => &section[range.clone()],
        }
    }

    pub(super) fn view<'a>(&'a self, section: &'a [u8]) -> StoredWeights<'a> {
        let (slots, weights) = self.bytes(section).split_at(4 * self.count);
        StoredWeights {
            slots: slots.as_chunks::<4>().0,
            weights: weights.as_chunks::<8>().0,
        }
    }
}
