// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Term dictionary: sorted base plus overlay, mapping a term to its list id.
#[derive(Default)]
pub(in super::super) struct TermDictionary {
    pub(super) bytes: Vec<u8>,
    /// `base_ids.len() + 1` offsets into `bytes`.
    pub(super) offsets: Vec<u32>,
    /// List id per base term, `NONE` once removed (dropped at the next rebuild).
    pub(super) base_ids: Vec<u32>,
    pub(super) tombstones: usize,
    pub(super) overlay: BTreeMap<Box<str>, u32>,
}

impl TermDictionary {
    pub(super) fn base_term(&self, i: usize) -> &[u8] {
        &self.bytes[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    pub(super) fn base_position(&self, term: &str) -> Result<usize, usize> {
        let (mut lo, mut hi) = (0usize, self.base_ids.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.base_term(mid).cmp(term.as_bytes()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(mid),
            }
        }
        Err(lo)
    }

    pub(in super::super) fn get(&self, term: &str) -> Option<u32> {
        if let Some(&id) = self.overlay.get(term) {
            return Some(id);
        }
        self.base_position(term)
            .ok()
            .map(|i| self.base_ids[i])
            .filter(|&id| id != NONE)
    }

    /// Overlay entries the base can absorb before the next fold.
    pub(super) fn overlay_budget(&self) -> usize {
        self.base_ids.len() / 4 + 1024
    }

    /// Adds a term absent from the dictionary.
    pub(in super::super) fn insert(&mut self, term: &str, id: u32) {
        debug_assert!(self.get(term).is_none(), "term already in the dictionary");
        if let Ok(i) = self.base_position(term) {
            // A tombstoned base term returns.
            self.base_ids[i] = id;
            self.tombstones -= 1;
            return;
        }
        self.overlay.insert(term.into(), id);
        if self.overlay.len() > self.overlay_budget() {
            self.rebuild();
        }
    }

    /// Adds terms absent from the dictionary (and from each other). Few go through
    /// [`Self::insert`]; more than the overlay would take merge into a new base in one pass.
    pub(in super::super) fn insert_many(&mut self, mut terms: Vec<(&str, u32)>) {
        if self.overlay.len() + terms.len() <= self.overlay_budget() {
            for (term, id) in terms {
                self.insert(term, id);
            }
            return;
        }
        terms.par_sort_unstable_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        debug_assert!(
            terms.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "duplicate term in a dictionary batch"
        );
        debug_assert!(
            terms.iter().all(|(term, _)| self.get(term).is_none()),
            "term already in the dictionary"
        );
        // The overlay and the new terms are sorted and disjoint: one merge gives the sorted
        // extra run the base absorbs.
        let mut extra: Vec<(&[u8], u32)> = Vec::with_capacity(self.overlay.len() + terms.len());
        let mut overlay = self
            .overlay
            .iter()
            .map(|(term, &id)| (term.as_bytes(), id))
            .peekable();
        let mut new = terms
            .iter()
            .map(|&(term, id)| (term.as_bytes(), id))
            .peekable();
        loop {
            let take_overlay = match (overlay.peek(), new.peek()) {
                (Some(a), Some(b)) => a.0 < b.0,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if take_overlay {
                overlay.next()
            } else {
                new.next()
            };
            extra.push(next.expect("peeked entry"));
        }
        let (bytes, offsets, ids) = self.merged_base(&extra);
        self.bytes = bytes;
        self.offsets = offsets;
        self.base_ids = ids;
        self.tombstones = 0;
        self.overlay = BTreeMap::new();
    }

    pub(in super::super) fn remove(&mut self, term: &str) -> Option<u32> {
        if let Some(id) = self.overlay.remove(term) {
            return Some(id);
        }
        let i = self.base_position(term).ok()?;
        let id = std::mem::replace(&mut self.base_ids[i], NONE);
        if id == NONE {
            return None;
        }
        self.tombstones += 1;
        if self.tombstones > self.base_ids.len() / 2 + 1024 {
            self.rebuild();
        }
        Some(id)
    }

    /// Live terms.
    #[cfg(test)]
    pub(in super::super) fn len(&self) -> usize {
        self.base_ids.len() - self.tombstones + self.overlay.len()
    }

    /// Folds the overlay into a new sorted base without tombstones.
    pub(in super::super) fn rebuild(&mut self) {
        let overlay: Vec<(&[u8], u32)> = self
            .overlay
            .iter()
            .map(|(t, &id)| (t.as_bytes(), id))
            .collect();
        let (bytes, offsets, ids) = self.merged_base(&overlay);
        self.bytes = bytes;
        self.offsets = offsets;
        self.base_ids = ids;
        self.tombstones = 0;
        self.overlay = BTreeMap::new();
    }

    /// The live base merged with `extra` (sorted, disjoint from the live base; a tombstoned
    /// base term equal to an extra term yields to it) as new base bytes, offsets and ids.
    pub(super) fn merged_base(&self, extra: &[(&[u8], u32)]) -> (Vec<u8>, Vec<u32>, Vec<u32>) {
        let count = self.base_ids.len() - self.tombstones + extra.len();
        let mut bytes = Vec::with_capacity(
            self.bytes.len() + extra.iter().map(|(t, _)| t.len()).sum::<usize>(),
        );
        let mut offsets = Vec::with_capacity(count + 1);
        let mut ids = Vec::with_capacity(count);
        offsets.push(0);
        let mut push = |term: &[u8], id: u32| {
            bytes.extend_from_slice(term);
            offsets.push(u32::try_from(bytes.len()).expect("term dictionary over 4 GiB"));
            ids.push(id);
        };
        let (mut i, mut j) = (0usize, 0usize);
        while i < self.base_ids.len() || j < extra.len() {
            if i < self.base_ids.len() && self.base_ids[i] == NONE {
                i += 1;
                continue;
            }
            let take_base = match (i < self.base_ids.len(), j < extra.len()) {
                (true, true) => {
                    debug_assert_ne!(self.base_term(i), extra[j].0, "live term merged twice");
                    self.base_term(i) < extra[j].0
                }
                (true, false) => true,
                (false, _) => false,
            };
            if take_base {
                push(self.base_term(i), self.base_ids[i]);
                i += 1;
            } else {
                push(extra[j].0, extra[j].1);
                j += 1;
            }
        }
        bytes.shrink_to_fit();
        (bytes, offsets, ids)
    }
}
