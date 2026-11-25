// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "experimental-batch-pool")]
use super::*;

/// Indexes of up to this many vertices clear the visited bits with one fill per search
/// (`n / 8` bytes, 128 KiB at the limit) instead of listing the words a search sets: the fill
/// costs less than the per-insert bookkeeping. Larger indexes keep the list, whose clearing cost is the words set.
const FILL_CLEAR_MAX: usize = 1 << 20;

/// Visited marks of one search: a bit per vertex, with an 8-bit epoch for the trace records of
/// the neighbour-block search.
pub(super) struct Marks {
    /// One bit per vertex. The words with a bit set since the last [`Marks::next`] are listed
    /// in `touched` and cleared then, so a search's visited footprint is `n / 8` bytes (a
    /// million vertices fit the second-level cache) and clearing costs the words it set.
    pub(super) words: Vec<u64>,
    pub(super) touched: Vec<u32>,
    /// Clear `words` with one fill instead of through `touched` (see [`FILL_CLEAR_MAX`]).
    pub(super) fill: bool,
    pub(super) n: usize,
    pub(super) epoch: u8,
    /// With neighbour blocks, one record per vertex instead of the bit.
    pub(super) traces: Vec<Trace>,
}

/// What one search knows about a vertex met through neighbour blocks: the precision-weighted
/// sum of the block estimates it received (one per expanded vertex listing it), their
/// weight, and whether it was expanded itself.
#[derive(Clone, Copy, Default)]
pub(super) struct Trace {
    pub(super) epoch: u8,
    pub(super) expanded: bool,
    pub(super) w: f32,
    pub(super) wsum: f32,
    /// The vertex's key in the pool, or `u64::MAX` when it holds none.
    pub(super) key: u64,
}

impl Marks {
    /// Marks for `n` vertices, with trace records when `traces` is set.
    pub(super) fn new(n: usize, traces: bool) -> Self {
        Self {
            words: vec![0; n.div_ceil(64)],
            touched: Vec::new(),
            fill: n <= FILL_CLEAR_MAX,
            n,
            epoch: 0,
            traces: vec![Trace::default(); if traces { n } else { 0 }],
        }
    }

    pub(super) fn next(&mut self) {
        if self.fill {
            self.words.fill(0);
        } else {
            for &w in &self.touched {
                self.words[w as usize] = 0;
            }
            self.touched.clear();
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.traces.fill(Trace::default());
            self.epoch = 1;
        }
    }

    /// The trace of `id` in this epoch, and whether it was just created (first encounter).
    #[inline]
    pub(super) fn trace(&mut self, id: u32) -> (bool, &mut Trace) {
        let t = &mut self.traces[id as usize];
        let fresh = t.epoch != self.epoch;
        if fresh {
            *t = Trace {
                epoch: self.epoch,
                key: u64::MAX,
                ..Trace::default()
            };
        }
        (fresh, t)
    }

    /// True if `id` was visited in this search (no mark is set).
    #[inline]
    pub(super) fn seen(&self, id: u32) -> bool {
        self.words[(id >> 6) as usize] >> (id & 63) & 1 == 1
    }

    /// True if `id` was not yet visited in this search.
    #[inline]
    pub(super) fn insert(&mut self, id: u32) -> bool {
        let w = &mut self.words[(id >> 6) as usize];
        let bit = 1u64 << (id & 63);
        if *w & bit != 0 {
            return false;
        }
        if !self.fill && *w == 0 {
            self.touched.push(id >> 6);
        }
        *w |= bit;
        true
    }
}

/// Best-first candidate pool as one sorted array: the `ef` best keys seen, nearest first,
/// each with an expanded flag, and a cursor at the nearest unexpanded entry. One insert is a
/// binary search and a short move; one expansion step is a scan from the cursor; the pool
/// rule ("the nearest unexpanded candidate is among the `ef` best seen") holds by
/// construction, so the search ends when no unexpanded entry is left. Two binary heaps
/// (queue and bounded pool) cost two sifts per admitted candidate and a pop per expansion,
/// and their results came back unsorted; the array is cache-resident at any practical `ef`.
pub(super) struct Pool {
    pub(super) keys: Vec<u64>,
    /// Expanded flags, bit `i` for `keys[i]`, in words; bits at `keys.len()` and above are 0.
    pub(super) expanded: Vec<u64>,
    #[cfg(feature = "experimental-batch-pool")]
    pub(super) merge_keys: Vec<u64>,
    #[cfg(feature = "experimental-batch-pool")]
    pub(super) merge_expanded: Vec<u64>,
    pub(super) cap: usize,
    pub(super) cursor: usize,
}

impl Pool {
    pub(super) fn reset(&mut self, cap: usize) {
        self.keys.clear();
        self.expanded.clear();
        self.cap = cap.max(1);
        self.cursor = 0;
    }

    pub(super) fn is_full(&self) -> bool {
        self.keys.len() >= self.cap
    }

    /// The worst key held, once the pool is full.
    pub(super) fn worst(&self) -> Option<u64> {
        self.is_full().then(|| self.keys[self.keys.len() - 1])
    }

    #[cfg(test)]
    pub(super) fn is_expanded(&self, i: usize) -> bool {
        self.expanded[i / 64] >> (i % 64) & 1 == 1
    }

    /// Opens an unexpanded flag at `pos` for `len` flags, shifting those at `pos..len` up one.
    #[inline]
    pub(super) fn insert_flag(&mut self, pos: usize, len: usize) {
        let words = (len + 1).div_ceil(64);
        if self.expanded.len() < words {
            self.expanded.resize(words, 0);
        }
        let w = pos / 64;
        for i in (w + 1..words).rev() {
            self.expanded[i] = self.expanded[i] << 1 | self.expanded[i - 1] >> 63;
        }
        let low = (1u64 << (pos % 64)) - 1;
        let x = self.expanded[w];
        self.expanded[w] = x & low | (x & !low) << 1;
    }

    /// Closes the flag at `pos` of `len` flags, shifting those above it down one.
    #[inline]
    pub(super) fn remove_flag(&mut self, pos: usize, len: usize) {
        let words = len.div_ceil(64);
        let w = pos / 64;
        let low = (1u64 << (pos % 64)) - 1;
        let carry = |e: &[u64], i: usize| if i + 1 < words { e[i + 1] << 63 } else { 0 };
        let x = self.expanded[w];
        self.expanded[w] = x & low | (x >> 1) & !low | carry(&self.expanded, w);
        for i in w + 1..words {
            self.expanded[i] = self.expanded[i] >> 1 | carry(&self.expanded, i);
        }
    }

    /// Admits `key` unless the pool is full of better ones; returns its position and the
    /// key evicted to make room.
    #[inline]
    pub(super) fn insert(&mut self, key: u64) -> Option<(usize, Option<u64>)> {
        let n = self.keys.len();
        if n >= self.cap && key >= self.keys[n - 1] {
            return None;
        }
        let pos = self.keys.partition_point(|&t| t < key);
        let mut evicted = None;
        let mut len = n;
        if n >= self.cap {
            evicted = self.keys.pop();
            len -= 1;
            self.expanded[len / 64] &= !(1u64 << (len % 64));
        }
        self.keys.insert(pos, key);
        self.insert_flag(pos, len);
        if pos < self.cursor {
            self.cursor = pos;
        }
        Some((pos, evicted))
    }

    /// `pending` has distinct vertex IDs absent from this pool, enforced by visited marks
    /// before row-code scoring. Re-admission through the fused path uses `insert` instead.
    #[cfg(feature = "experimental-batch-pool")]
    pub(super) fn insert_batch(&mut self, pending: &mut Vec<u64>, k: usize) -> bool {
        debug_assert!(pending.iter().enumerate().all(|(position, &candidate)| {
            let id = key_id(candidate);
            pending[..position].iter().all(|&other| key_id(other) != id)
                && self.keys.iter().all(|&other| key_id(other) != id)
        }));
        if let Some(worst) = self.worst() {
            pending.retain(|&key| key < worst);
        }
        if pending.len() < 8 {
            let mut improved = false;
            for &key in pending.iter() {
                if let Some((pos, _)) = self.insert(key) {
                    improved |= pos < k;
                }
            }
            return improved;
        }
        pending.sort_unstable();
        // Sequential insertions reset patience if any new key enters the initial top-k.
        let improved = k > 0 && (self.keys.len() < k || pending[0] <= self.keys[k - 1]);
        let len = self.cap.min(self.keys.len().saturating_add(pending.len()));
        self.merge_keys.clear();
        self.merge_keys.reserve(len);
        self.merge_expanded.clear();
        self.merge_expanded.resize(len.div_ceil(64), 0);
        let (mut old, mut new) = (0, 0);
        for pos in 0..len {
            if new < pending.len() && (old == self.keys.len() || pending[new] <= self.keys[old]) {
                self.merge_keys.push(pending[new]);
                self.cursor = self.cursor.min(pos);
                new += 1;
            } else {
                self.merge_keys.push(self.keys[old]);
                let flag = self.expanded[old / 64] >> (old % 64) & 1;
                self.merge_expanded[pos / 64] |= flag << (pos % 64);
                old += 1;
            }
        }
        std::mem::swap(&mut self.keys, &mut self.merge_keys);
        std::mem::swap(&mut self.expanded, &mut self.merge_expanded);
        improved
    }

    /// Drops `key` if present (a re-admitted vertex's older entry).
    pub(super) fn remove(&mut self, key: u64) {
        let pos = self.keys.partition_point(|&t| t < key);
        if pos < self.keys.len() && self.keys[pos] == key {
            let len = self.keys.len();
            self.keys.remove(pos);
            self.remove_flag(pos, len);
            if pos < self.cursor {
                self.cursor -= 1;
            }
        }
    }

    /// Index of the `n`-th (from 0) unexpanded key at or after the cursor.
    #[inline]
    pub(super) fn unexpanded(&self, mut n: usize) -> Option<usize> {
        let len = self.keys.len();
        let mut i = self.cursor;
        while i < len {
            let free = !self.expanded[i / 64] >> (i % 64);
            if free == 0 {
                i = (i / 64 + 1) * 64;
                continue;
            }
            i += free.trailing_zeros() as usize;
            if i >= len {
                break;
            }
            if n == 0 {
                return Some(i);
            }
            n -= 1;
            i += 1;
        }
        None
    }

    /// The nearest unexpanded key, now marked expanded.
    #[inline]
    pub(super) fn next(&mut self) -> Option<u64> {
        match self.unexpanded(0) {
            Some(i) => {
                self.expanded[i / 64] |= 1u64 << (i % 64);
                self.cursor = i;
                Some(self.keys[i])
            }
            None => {
                self.cursor = self.keys.len();
                None
            }
        }
    }

    /// The key [`Pool::next`] would return.
    #[inline]
    pub(super) fn peek_next(&self) -> Option<u64> {
        self.peek_unexpanded(0)
    }

    /// The `n`-th (from 0) unexpanded key in pool order.
    #[inline]
    pub(super) fn peek_unexpanded(&self, n: usize) -> Option<u64> {
        self.unexpanded(n).map(|i| self.keys[i])
    }
}

#[derive(Default)]
pub(super) struct Reservoir {
    pub(super) keys: Vec<u64>,
    pub(super) cap: usize,
}

impl Reservoir {
    pub(super) fn reset(&mut self, cap: usize) {
        self.keys.clear();
        self.cap = cap;
    }

    #[inline]
    pub(super) fn offer(&mut self, key: u64) {
        if self.cap == 0 || (self.keys.len() == self.cap && key >= self.keys[self.keys.len() - 1]) {
            return;
        }
        let pos = match self.keys.binary_search(&key) {
            Ok(_) => return,
            Err(pos) => pos,
        };
        if self.keys.len() == self.cap {
            self.keys.pop();
        }
        self.keys.insert(pos, key);
    }
}
