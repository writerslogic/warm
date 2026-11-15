// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Exact block-max MaxScore over checkpoint postings with impacts (checkpoint version 3).
//!
//! [`rank`] returns the chunks [`super::search::LexicalScratch::rank`] would, with the same
//! scores to the bit, without decoding the posting blocks that cannot hold a top-`limit`
//! score. A chunk's score is the sum, in query term order starting from zero, of each matching
//! term's `Bm25::weight` at that chunk's word count: the very expression and order the
//! exhaustive ranking evaluates. Pruning only ever drops a chunk whose upper bound is strictly
//! below the `limit`-th best exact score found so far, so every chunk that scores at least the
//! final `limit`-th best score (ties included) is scored exactly and kept.
//!
//! Bounds come from [`Bound`], the conservative weight bound the explicit-query WAND uses, at
//! the largest frequency and smallest word count of a list or of one block. A block's header
//! (first slot, last slot, bit widths, bounds) is read without decoding its postings, so a
//! term cursor skips whole blocks by their last slot and by their bounds.

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use super::lexical::wand::{sum_bound, Bound};
use super::postings::{get_varint, unpack_values, Postings, CHECKPOINT_BLOCK};
use super::search::LexicalSource;
use super::Bm25;

/// The slot of a cursor with no posting left.
const NO_DOC: u32 = u32::MAX;

/// What one block's header says, read without decoding the block.
#[derive(Clone, Copy, Default)]
struct Header {
    first: u32,
    last: u32,
    count: usize,
    gap_bits: u8,
    frequency_bits: u8,
    /// Offset of the packed gaps; the packed frequencies follow them.
    data_at: usize,
    /// Offset of the next block's header.
    end_at: usize,
    /// Postings in the blocks after this one.
    left_after: usize,
    max_frequency: u32,
    min_words: u32,
}

impl Header {
    /// The header at `at` of a block among the `left` postings still to come, `previous` being
    /// the last slot of the block before it (0 for the first block).
    fn parse(bytes: &[u8], at: usize, left: usize, previous: u32) -> Self {
        let mut at = at;
        let count = left.min(CHECKPOINT_BLOCK);
        let delta = get_varint(bytes, &mut at);
        let (gap_bits, frequency_bits) = (bytes[at], bytes[at + 1]);
        at += 2;
        let span = get_varint(bytes, &mut at);
        let max_frequency = get_varint(bytes, &mut at) as u32;
        let min_words = get_varint(bytes, &mut at) as u32;
        let gap_len = (count * usize::from(gap_bits)).div_ceil(8);
        let frequency_len = (count * usize::from(frequency_bits)).div_ceil(8);
        let first = previous + delta as u32;
        Self {
            first,
            last: first + span as u32,
            count,
            gap_bits,
            frequency_bits,
            data_at: at,
            end_at: at + gap_len + frequency_len,
            left_after: left - count,
            max_frequency,
            min_words,
        }
    }
}

/// A forward cursor over one version 3 posting list.
pub(super) struct ImpactCursor<'a> {
    bytes: &'a [u8],
    /// The block the cursor's posting is in.
    current: Header,
    /// A block ahead of it, found by [`Self::bound_at`] without decoding.
    ahead: Header,
    ahead_valid: bool,
    /// The bound of `ahead`'s block and the offset it was computed for.
    ahead_bound: f64,
    ahead_key: usize,
    /// The bound of `current`'s block and the offset it was computed for.
    current_bound: f64,
    current_key: usize,
    decoded: bool,
    position: usize,
    ended: bool,
    /// Blocks this cursor decoded since it was opened.
    decodes: usize,
    list_max_frequency: u32,
    list_min_words: u32,
    slots: [u32; CHECKPOINT_BLOCK],
    frequencies: [u32; CHECKPOINT_BLOCK],
}

impl<'a> ImpactCursor<'a> {
    fn new() -> Self {
        Self {
            bytes: &[],
            current: Header::default(),
            ahead: Header::default(),
            ahead_valid: false,
            ahead_bound: 0.0,
            ahead_key: usize::MAX,
            current_bound: 0.0,
            current_key: usize::MAX,
            decoded: false,
            position: 0,
            ended: true,
            decodes: 0,
            list_max_frequency: 0,
            list_min_words: 0,
            slots: [0; CHECKPOINT_BLOCK],
            frequencies: [0; CHECKPOINT_BLOCK],
        }
    }

    /// Points the cursor at the first posting of `list`, a nonempty version 3 list.
    fn open(&mut self, list: &Postings<'a>) {
        debug_assert!(list.impacts && list.count > 0);
        let bytes = list.bytes;
        let mut at = 0;
        self.list_max_frequency = get_varint(bytes, &mut at) as u32;
        self.list_min_words = get_varint(bytes, &mut at) as u32;
        self.bytes = bytes;
        self.current = Header::parse(bytes, at, list.count as usize, 0);
        self.ahead_valid = false;
        self.ahead_key = usize::MAX;
        self.current_key = usize::MAX;
        self.decoded = false;
        self.position = 0;
        self.ended = false;
    }

    /// The slot of the current posting, [`NO_DOC`] at the end.
    #[inline]
    fn doc(&self) -> u32 {
        if self.ended {
            NO_DOC
        } else if self.decoded {
            self.slots[self.position]
        } else {
            self.current.first
        }
    }

    fn decode(&mut self) {
        let header = self.current;
        let (mut gaps, mut frequencies) = ([0u64; CHECKPOINT_BLOCK], [0u64; CHECKPOINT_BLOCK]);
        let gap_len = (header.count * usize::from(header.gap_bits)).div_ceil(8);
        // The decoders read whole 8-byte windows while they stay inside the slice they are
        // given, so each gets the rest of the list and uses only its own values.
        unpack_values(
            &self.bytes[header.data_at..],
            header.count,
            header.gap_bits,
            &mut gaps,
        );
        unpack_values(
            &self.bytes[header.data_at + gap_len..],
            header.count,
            header.frequency_bits,
            &mut frequencies,
        );
        let mut slot = u64::from(header.first);
        let block = gaps[..header.count]
            .iter()
            .zip(&frequencies[..header.count]);
        for ((out_slot, out_frequency), (&gap, &frequency)) in self
            .slots
            .iter_mut()
            .zip(self.frequencies.iter_mut())
            .zip(block)
        {
            slot += gap;
            *out_slot = slot as u32;
            *out_frequency = frequency as u32;
        }
        self.decoded = true;
        self.decodes += 1;
    }

    /// The frequency of the current posting.
    fn frequency(&mut self) -> u32 {
        if !self.decoded {
            self.decode();
        }
        self.frequencies[self.position]
    }

    /// Moves to the next block; false at the end of the list.
    fn next_block(&mut self) -> bool {
        if self.current.left_after == 0 {
            return false;
        }
        self.current = Header::parse(
            self.bytes,
            self.current.end_at,
            self.current.left_after,
            self.current.last,
        );
        self.decoded = false;
        self.position = 0;
        true
    }

    fn advance(&mut self) {
        if self.ended {
            return;
        }
        if !self.decoded {
            self.decode();
        }
        self.position += 1;
        if self.position >= self.current.count && !self.next_block() {
            self.ended = true;
        }
    }

    /// Moves to the first posting at or after `target`.
    fn seek(&mut self, target: u32) {
        if self.ended || self.doc() >= target {
            return;
        }
        if self.current.last < target {
            // A block found ahead by `bound_at` is on the way when every block before it ends
            // below `target`, as they do when it starts at or below it.
            if self.ahead_valid
                && self.ahead.first > self.current.first
                && self.ahead.first <= target
            {
                self.current = self.ahead;
                self.decoded = false;
                self.position = 0;
            }
            while self.current.last < target {
                if !self.next_block() {
                    self.ended = true;
                    return;
                }
            }
        }
        if target <= self.current.first {
            return;
        }
        if !self.decoded {
            self.decode();
        }
        let from = self.position;
        self.position =
            from + self.slots[from..self.current.count].partition_point(|&s| s < target);
    }

    /// The bound of any weight in the block the cursor is in.
    fn block_bound(&mut self, bound: Bound) -> f64 {
        if self.current_key != self.current.data_at {
            self.current_key = self.current.data_at;
            self.current_bound = bound.at(self.current.max_frequency, self.current.min_words);
        }
        self.current_bound
    }

    /// The bound of any weight in the block holding the first posting at or after `target`
    /// (and that block's first and last slots), read without decoding: `None` when no posting at or
    /// after `target` remains.
    fn bound_at(&mut self, target: u32, bound: Bound) -> Option<(f64, u32, u32)> {
        if self.ended {
            return None;
        }
        if !self.ahead_valid
            || self.ahead.first < self.current.first
            || (self.ahead.first > self.current.first && target < self.ahead.first)
        {
            self.ahead = self.current;
            self.ahead_valid = true;
        }
        while self.ahead.last < target {
            if self.ahead.left_after == 0 {
                return None;
            }
            self.ahead = Header::parse(
                self.bytes,
                self.ahead.end_at,
                self.ahead.left_after,
                self.ahead.last,
            );
        }
        if self.ahead_key != self.ahead.data_at {
            self.ahead_key = self.ahead.data_at;
            self.ahead_bound = bound.at(self.ahead.max_frequency, self.ahead.min_words);
        }
        Some((self.ahead_bound, self.ahead.first, self.ahead.last))
    }
}

/// One query term: its cursor and what scoring it needs.
struct Term {
    idf: f64,
    bound: Bound,
    /// A bound of every weight of the list.
    ceiling: f64,
}

/// Working memory of [`rank`], reused across queries.
#[derive(Default)]
pub(super) struct PruneScratch {
    terms: Vec<Term>,
    order: Vec<usize>,
    prefix: Vec<f64>,
    weights: Vec<f64>,
    /// Chunks scoring at least the floor when read, as `(score, slot)`.
    pub(super) candidates: Vec<(f64, u32)>,
    best: BinaryHeap<Reverse<u64>>,
    /// What the last query read: postings in its lists, blocks decoded, chunks scored.
    pub(super) stats: PruneStats,
}

/// Work counters of one [`rank`] call, for measuring how much pruning skips.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct PruneStats {
    pub(super) listed: usize,
    pub(super) blocks_listed: usize,
    pub(super) blocks_decoded: usize,
    pub(super) scored: usize,
}

/// The query's top `limit` chunks as [`super::search::LexicalScratch::rank`] selects them, left
/// in `scratch.candidates` unordered: every chunk scoring at least the `limit`-th best score,
/// ties included. `None` when the checkpoint or the weights admit no proven bound, and the
/// caller ranks exhaustively.
pub(super) fn rank<S: LexicalSource>(
    source: &S,
    terms: &[BTreeSet<String>; 2],
    bm25: Bm25,
    limit: usize,
    scratch: &mut PruneScratch,
) -> Option<()> {
    let image = source.impact_image()?;
    let n = image.chunk_count() as f64;
    let average = (image.word_count() as f64 / n).max(1.0);
    scratch.candidates.clear();
    scratch.best.clear();
    scratch.terms.clear();
    scratch.stats = PruneStats::default();
    let mut lists = Vec::new();
    for (form, set) in terms.iter().enumerate() {
        for word in set {
            let Some(list) = image.postings(form, word) else {
                continue;
            };
            let idf = Bm25::idf(n, f64::from(list.count));
            let bound = Bound::new(idf, average, bm25)?;
            lists.push((list, idf, bound));
        }
    }
    if limit == 0 || lists.is_empty() {
        return Some(());
    }
    let mut cursors: Vec<ImpactCursor<'_>> = Vec::with_capacity(lists.len());
    for (list, idf, bound) in &lists {
        let mut cursor = ImpactCursor::new();
        cursor.open(list);
        scratch.terms.push(Term {
            idf: *idf,
            bound: *bound,
            ceiling: bound.at(cursor.list_max_frequency, cursor.list_min_words),
        });
        cursors.push(cursor);
    }
    let count = lists.len();
    scratch.stats.listed = lists.iter().map(|(list, ..)| list.count as usize).sum();
    scratch.stats.blocks_listed = lists
        .iter()
        .map(|(list, ..)| (list.count as usize).div_ceil(CHECKPOINT_BLOCK))
        .sum();
    let mut scored = 0usize;
    let terms = &scratch.terms;
    // Terms by ascending bound: the first `quiet` of them cannot make a chunk reach the
    // threshold together, so only the rest (the essential terms) pick the chunks to score and
    // the quiet ones are looked up, block by block, for those chunks alone.
    let by_bound = &mut scratch.order;
    by_bound.clear();
    by_bound.extend(0..count);
    by_bound.sort_by(|&a, &b| terms[a].ceiling.total_cmp(&terms[b].ceiling));
    let prefix = &mut scratch.prefix;
    prefix.clear();
    prefix.push(0.0);
    for &i in by_bound.iter() {
        prefix.push(prefix[prefix.len() - 1] + terms[i].ceiling);
    }
    let weights = &mut scratch.weights;
    weights.clear();
    weights.resize(count, 0.0);
    let (candidates, best) = (&mut scratch.candidates, &mut scratch.best);
    // The least admissible score as bits, as in the exhaustive selection: any positive one
    // until the heap is full, then the heap's floor.
    const INFINITY: u64 = 0x7FF0_0000_0000_0000;
    let mut floor = 1u64;
    let mut threshold = 0.0f64;
    let mut quiet = 0usize;
    let mut window_end = 0u32;
    loop {
        while quiet < count && sum_bound(prefix[quiet + 1], count) < threshold {
            quiet += 1;
        }
        let mut slot = NO_DOC;
        for &i in &by_bound[quiet..] {
            slot = slot.min(cursors[i].doc());
        }
        if slot == NO_DOC {
            break;
        }
        if threshold > 0.0 && slot > window_end {
            // The blocks the essential terms are in up to the first of their ends: when their
            // bounds and the quiet terms' together stay below the threshold, no chunk in that
            // window can reach it, and every essential cursor moves past it undecoded.
            let mut window_bound = prefix[quiet];
            let mut end = u32::MAX;
            for &i in &by_bound[quiet..] {
                if let Some((_, _, last)) = cursors[i].bound_at(slot, terms[i].bound) {
                    end = end.min(last);
                }
            }
            for &i in &by_bound[quiet..] {
                if let Some((block, first, _)) = cursors[i].bound_at(slot, terms[i].bound) {
                    if first <= end {
                        window_bound += block;
                    }
                }
            }
            if sum_bound(window_bound, count) < threshold && end < NO_DOC {
                for &i in &by_bound[quiet..] {
                    cursors[i].seek(end + 1);
                }
                continue;
            }
            window_end = end;
        }
        if threshold > 0.0 {
            // The chunk's best possible score from the blocks its essential postings are in
            // and the quiet terms' bounds; below the threshold it is not read at all.
            let mut best_case = prefix[quiet];
            for &i in &by_bound[quiet..] {
                if cursors[i].doc() == slot {
                    best_case += cursors[i].block_bound(terms[i].bound);
                }
            }
            if sum_bound(best_case, count) < threshold {
                for &i in &by_bound[quiet..] {
                    if cursors[i].doc() == slot {
                        cursors[i].advance();
                    }
                }
                continue;
            }
        }
        // A weight of a term absent from the chunk is zero, which adds nothing to the sum.
        let norm = bm25.norm(f64::from(image.slot_word_count(slot as usize)), average);
        let mut partial = 0.0f64;
        for &i in &by_bound[quiet..] {
            if cursors[i].doc() == slot {
                let tf = f64::from(cursors[i].frequency());
                let weight = bm25.weight(terms[i].idf, tf, norm);
                weights[i] = weight;
                partial += weight;
            }
        }
        let mut reachable = true;
        for j in (0..quiet).rev() {
            let i = by_bound[j];
            if threshold > 0.0 {
                if sum_bound(partial + prefix[j + 1], count) < threshold {
                    reachable = false;
                    break;
                }
                match cursors[i].bound_at(slot, terms[i].bound) {
                    None => continue,
                    Some((block, ..)) => {
                        if sum_bound(partial + prefix[j] + block, count) < threshold {
                            reachable = false;
                            break;
                        }
                    }
                }
            }
            cursors[i].seek(slot);
            if cursors[i].doc() == slot {
                let tf = f64::from(cursors[i].frequency());
                let weight = bm25.weight(terms[i].idf, tf, norm);
                weights[i] = weight;
                partial += weight;
            }
        }
        // The sum in query term order, from zero: the exhaustive ranking's own.
        let mut score = 0.0f64;
        for weight in weights.iter_mut() {
            score += *weight;
            *weight = 0.0;
        }
        for &i in &by_bound[quiet..] {
            if cursors[i].doc() == slot {
                cursors[i].advance();
            }
        }
        if !reachable {
            continue;
        }
        scored += 1;
        let bits = score.to_bits();
        if bits.wrapping_sub(floor) > INFINITY - floor {
            continue;
        }
        candidates.push((score, slot));
        if best.len() < limit {
            best.push(Reverse(bits));
        } else if bits > floor {
            *best.peek_mut().expect("full heap") = Reverse(bits);
        } else {
            continue;
        }
        if best.len() == limit {
            floor = best.peek().expect("full heap").0;
            threshold = f64::from_bits(floor);
        }
    }
    scratch.stats.scored = scored;
    scratch.stats.blocks_decoded = cursors.iter().map(|c| c.decodes).sum();
    let bar = if best.len() < limit {
        0
    } else {
        best.peek().expect("full heap").0
    };
    candidates.retain(|&(score, _)| score.to_bits() >= bar);
    Some(())
}
