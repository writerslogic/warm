// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Readers of checkpoint version 4 posting lists: the version 3 block bytes
//! (`postings::Postings`) plus, per list, a skip table and a positions stream
//! (`docs/file-format/index.md`).
//!
//! A list of `count` postings has `ceil(count / 128)` blocks. With two or more blocks its skip
//! table holds five little-endian `u32` arrays of that length, one after the other: each
//! block's last slot, the offset of its header in the list's bytes, the offset of its positions
//! in the list's positions bytes, its largest frequency and its least word count. Then come
//! the frontier directory, `blocks + runs + 1` `u32` byte offsets (runs are [`RUN_BLOCKS`]
//! blocks each, the last one shorter), and the frontier bytes: per block, then per run, a
//! varint point count and that many `(frequency, words)` varint pairs, frequency descending.
//! A block with count zero has one undominated point, its two maxima. A one block list has no
//! table (its block starts after the list header and its positions at zero).
//!
//! A dense list's table (one holding at least one of every [`DENSE_SPAN`] slots through its
//! last) ends, after the frontier bytes, with its slot bitmap: `ceil((last + 1) / 64)`
//! little-endian `u64` words, bit `s % 64` of word `s / 64` set for each slot `s` it holds.
//!
//! A block's positions are one width byte `w`, the byte length of each posting's positions
//! packed at `w` bits, then those positions end to end: each posting's token positions as
//! gaps (the first absolute) in unsigned LEB128, the bytes the chunk's own positional record
//! holds for the term.

use super::lexical::wand::Bound;
use super::postings::{get_varint, unpack_values, CHECKPOINT_BLOCK};

/// The slot of a cursor with no posting left.
pub(super) const NO_DOC: u32 = u32::MAX;

/// Bytes of skip table per block of a list of two or more blocks, before the frontiers.
const SKIP_ENTRY: usize = 20;

/// Blocks per run, the unit of a list's coarser frontier.
pub(super) const RUN_BLOCKS: usize = 32;

/// A list of at least this many runs has a [`RankTable`].
pub(super) const RANKED_RUNS: usize = 8;

/// One long list's weight bounds at idf one for one BM25 shape, each rounded up to an `f32`:
/// per run and per block, from their frontiers, and the runs ordered by their bound, highest
/// first (ties by run). A single term ranks its runs from this table instead of decoding every
/// run's frontier per query.
pub(super) struct RankTable {
    pub(super) order: Box<[u32]>,
    pub(super) runs: Box<[f32]>,
    pub(super) blocks: Box<[f32]>,
}

impl RankTable {
    /// The table of `list`, a list of two or more blocks, under `unit`, a bound at idf one.
    pub(super) fn new(list: ListRef<'_>, unit: Bound) -> Self {
        let mut cursor = Cursor::open(list);
        let runs: Box<[f32]> = (0..cursor.runs)
            .map(|run| up_f32(cursor.run_weight(run, unit)))
            .collect();
        let blocks = (0..cursor.blocks)
            .map(|block| {
                cursor.goto_block(block);
                up_f32(cursor.block_weight_uncached(unit))
            })
            .collect();
        let mut order: Vec<u32> = (0..runs.len() as u32).collect();
        order.sort_unstable_by(|&a, &b| {
            runs[b as usize]
                .total_cmp(&runs[a as usize])
                .then(a.cmp(&b))
        });
        Self {
            order: order.into_boxed_slice(),
            runs,
            blocks,
        }
    }

    /// A bound at idf `idf` of the weights a stored bound `unit` covers: the weight is the idf
    /// times its value at idf one, and `16 * EPSILON` covers that product's rounding.
    pub(super) fn scale(unit: f32, idf: f64) -> f64 {
        super::lexical::wand::up(f64::from(unit) * idf * (1.0 + 16.0 * f64::EPSILON))
    }
}

/// Bounds of one list's weights over ranges of slots, from its skip table and, where it has
/// one, its [`RankTable`]: ranges are asked for in ascending order.
pub(super) struct BlockBounds<'a> {
    skips: &'a [u8],
    blocks: usize,
    table: Option<&'a RankTable>,
    /// The first block whose last slot may be at or after the next range's start.
    at: usize,
}

impl<'a> BlockBounds<'a> {
    pub(super) fn new(list: ListRef<'a>, table: Option<&'a RankTable>) -> Self {
        Self {
            skips: list.skips,
            blocks: list.blocks(),
            table,
            at: 0,
        }
    }

    #[inline(always)]
    fn skip(&self, array: usize, block: usize) -> u32 {
        let at = 4 * (array * self.blocks + block);
        u32::from_le_bytes(self.skips[at..at + 4].try_into().expect("four bytes"))
    }

    /// Moves `at` to the first block at or after it whose last slot reaches `lo`, galloping
    /// then bisecting: the ranges a query asks for can jump many blocks at once.
    fn advance(&mut self, lo: u32) {
        if self.at >= self.blocks || self.skip(0, self.at) >= lo {
            return;
        }
        // `below` is a block whose last slot is below `lo`; the answer is in `below + 1..=end`.
        let (mut below, mut end, mut step) = (self.at, self.at + 1, 1usize);
        while end < self.blocks && self.skip(0, end) < lo {
            below = end;
            step *= 2;
            end += step;
        }
        let (mut first, mut last) = (below + 1, end.min(self.blocks));
        while first < last {
            let middle = first + (last - first) / 2;
            if self.skip(0, middle) < lo {
                first = middle + 1;
            } else {
                last = middle;
            }
        }
        self.at = first;
    }

    /// A bound of the weight of every posting in slots `lo..hi` under `bound`, never above
    /// `ceiling`; `lo` never decreases between calls.
    pub(super) fn bound(&mut self, lo: u32, hi: u32, bound: Bound, ceiling: f64) -> f64 {
        if self.blocks < 2 {
            return ceiling;
        }
        self.advance(lo);
        let mut best = 0.0f64;
        let mut block = self.at;
        while block < self.blocks {
            let weight = match self.table {
                Some(table) => RankTable::scale(table.blocks[block], bound.idf()),
                None => bound.at(self.skip(3, block), self.skip(4, block)),
            };
            best = best.max(weight);
            if self.skip(0, block) >= hi.saturating_sub(1) {
                break;
            }
            block += 1;
        }
        best.min(ceiling)
    }

    /// The first block at or after `from` (at least 1) whose last slot reaches `target`, with
    /// the offset of its header and the last slot of the block before it: `Some(None)` when no
    /// block does, `None` when the list has no skip table to search.
    pub(super) fn find(&self, from: usize, target: u32) -> Option<Option<(usize, usize, u32)>> {
        if self.blocks < 2 || from == 0 {
            return None;
        }
        // Gallop from `from`, then binary search the bracket.
        let (mut lo, mut hi, mut step) = (from, from, 1usize);
        while hi < self.blocks && self.skip(0, hi) < target {
            lo = hi + 1;
            hi += step;
            step *= 2;
        }
        let mut hi = hi.min(self.blocks);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.skip(0, mid) < target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Some((lo < self.blocks).then(|| (lo, self.skip(1, lo) as usize, self.skip(0, lo - 1))))
    }

    /// A bound of the weight of every posting in block `block` under `bound`, never above
    /// `ceiling`.
    pub(super) fn block(&self, block: usize, bound: Bound, ceiling: f64) -> f64 {
        if self.blocks < 2 || block >= self.blocks {
            return ceiling;
        }
        let weight = match self.table {
            Some(table) => RankTable::scale(table.blocks[block], bound.idf()),
            None => bound.at(self.skip(3, block), self.skip(4, block)),
        };
        weight.min(ceiling)
    }
}

/// `value` rounded up to an `f32`.
fn up_f32(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) < value {
        rounded.next_up()
    } else {
        rounded
    }
}

/// A list of two or more blocks holding at least one of every this many slots through its last
/// is dense: its skip table ends with its slot bitmap.
pub(super) const DENSE_SPAN: u64 = 8;

/// The fewest bytes of skip table a list of `count` postings has: the arrays and the frontier
/// directory, with no frontier bytes.
pub(super) fn skip_min_len(count: u64) -> usize {
    let blocks = count.div_ceil(CHECKPOINT_BLOCK as u64) as usize;
    if blocks >= 2 {
        blocks * SKIP_ENTRY + 4 * (blocks + blocks.div_ceil(RUN_BLOCKS) + 1)
    } else {
        0
    }
}

/// The points of one frontier, as written: `(frequency, words)` by frequency descending.
pub(super) struct Points<'a> {
    bytes: &'a [u8],
    at: usize,
    left: usize,
}

impl Points<'_> {
    pub(super) fn is_empty(&self) -> bool {
        self.left == 0
    }
}

impl Iterator for Points<'_> {
    type Item = (u32, u32);

    #[inline]
    fn next(&mut self) -> Option<(u32, u32)> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let frequency = get_varint(self.bytes, &mut self.at) as u32;
        let words = get_varint(self.bytes, &mut self.at) as u32;
        Some((frequency, words))
    }
}

#[inline(always)]
fn u32_at(bytes: &[u8], index: usize) -> u32 {
    let at = 4 * index;
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// One checkpoint list: its block bytes, skip table and positions bytes (empty when the
/// checkpoint holds no positions).
#[derive(Clone, Copy)]
pub(super) struct ListRef<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) count: u32,
    pub(super) skips: &'a [u8],
    pub(super) positions: &'a [u8],
}

impl<'a> ListRef<'a> {
    pub(super) fn blocks(&self) -> usize {
        (self.count as usize).div_ceil(CHECKPOINT_BLOCK)
    }

    /// The list's largest frequency, least word count and the offset of its first block.
    pub(super) fn header(&self) -> (u32, u32, usize) {
        let mut at = 0;
        let frequency = get_varint(self.bytes, &mut at) as u32;
        let words = get_varint(self.bytes, &mut at) as u32;
        (frequency, words, at)
    }
}

/// A forward cursor over one version 4 list.
pub(super) struct Cursor<'a> {
    list: ListRef<'a>,
    blocks: usize,
    list_frequency: u32,
    list_words: u32,
    block: usize,
    runs: usize,
    first: u32,
    last: u32,
    count: usize,
    gap_bits: u8,
    frequency_bits: u8,
    data_at: usize,
    end_at: usize,
    left_after: usize,
    decoded: bool,
    frequencies_ready: bool,
    index: usize,
    ended: bool,
    /// Blocks decoded since the cursor was opened.
    pub(super) decodes: usize,
    slots: [u32; CHECKPOINT_BLOCK],
    frequencies: [u32; CHECKPOINT_BLOCK],
    positions_ready: bool,
    positions_at: usize,
    starts: [u32; CHECKPOINT_BLOCK + 1],
    /// The block whose [`Self::block_weight`] is cached, and that weight: a cursor is used with
    /// one bound.
    weight_block: std::cell::Cell<usize>,
    weight: std::cell::Cell<f64>,
    /// A dense list's slot bitmap, empty for any other.
    dense: &'a [u8],
    /// The list's rank table when its bounds hold for the query's BM25 shape: block weights
    /// are then read from it instead of decoded from the frontiers.
    ranks: Option<&'a RankTable>,
}

impl<'a> Cursor<'a> {
    #[inline(always)]
    fn skip(&self, array: usize, block: usize) -> u32 {
        u32_at(self.list.skips, array * self.blocks + block)
    }

    /// A cursor at the first posting of `list`, which has at least one.
    pub(super) fn open(list: ListRef<'a>) -> Self {
        let (list_frequency, list_words, first_at) = list.header();
        let mut cursor = Self {
            list,
            blocks: list.blocks(),
            runs: list.blocks().div_ceil(RUN_BLOCKS),
            list_frequency,
            list_words,
            block: 0,
            first: 0,
            last: 0,
            count: 0,
            gap_bits: 0,
            frequency_bits: 0,
            data_at: 0,
            end_at: 0,
            left_after: 0,
            decoded: false,
            frequencies_ready: false,
            index: 0,
            ended: false,
            decodes: 0,
            slots: [0; CHECKPOINT_BLOCK],
            frequencies: [0; CHECKPOINT_BLOCK],
            positions_ready: false,
            positions_at: 0,
            starts: [0; CHECKPOINT_BLOCK + 1],
            weight_block: std::cell::Cell::new(usize::MAX),
            weight: std::cell::Cell::new(0.0),
            dense: &[],
            ranks: None,
        };
        cursor.dense = cursor.find_dense();
        cursor.load(0, first_at, 0);
        cursor
    }

    /// The slot bitmap after the frontiers, when the table has one of the length its last slot
    /// gives; empty otherwise (a list that is not dense, or a table written without bitmaps).
    fn find_dense(&self) -> &'a [u8] {
        if self.blocks < 2 {
            return &[];
        }
        let skips = self.list.skips;
        let directory = SKIP_ENTRY * self.blocks;
        let entries = self.blocks + self.runs + 1;
        let Some(end) = skips.get(directory + 4 * (entries - 1)..directory + 4 * entries) else {
            return &[];
        };
        let end = u32::from_le_bytes([end[0], end[1], end[2], end[3]]) as usize;
        let at = directory + 4 * entries + end;
        let words = (u64::from(self.skip(0, self.blocks - 1)) + 1).div_ceil(64) as usize;
        match skips.get(at..) {
            Some(bits) if bits.len() == 8 * words => bits,
            _ => &[],
        }
    }

    /// A dense list's slot bitmap as little-endian `u64` words (see [`DENSE_SPAN`]).
    pub(super) fn dense(&self) -> Option<&'a [u8]> {
        (!self.dense.is_empty()).then_some(self.dense)
    }

    /// Reads block `block`'s header at `at`, the block before it ending at slot `previous`.
    fn load(&mut self, block: usize, at: usize, previous: u32) {
        let bytes = self.list.bytes;
        let left = self.list.count as usize - block * CHECKPOINT_BLOCK;
        let count = left.min(CHECKPOINT_BLOCK);
        let mut at = at;
        let delta = get_varint(bytes, &mut at);
        let (gap_bits, frequency_bits) = (bytes[at], bytes[at + 1]);
        at += 2;
        let span = get_varint(bytes, &mut at);
        get_varint(bytes, &mut at);
        get_varint(bytes, &mut at);
        let gap_len = (count * usize::from(gap_bits)).div_ceil(8);
        let frequency_len = (count * usize::from(frequency_bits)).div_ceil(8);
        self.block = block;
        self.first = previous + delta as u32;
        self.last = self.first + span as u32;
        self.count = count;
        self.gap_bits = gap_bits;
        self.frequency_bits = frequency_bits;
        self.data_at = at;
        self.end_at = at + gap_len + frequency_len;
        self.left_after = left - count;
        self.decoded = false;
        self.frequencies_ready = false;
        self.positions_ready = false;
        self.index = 0;
    }

    fn decode(&mut self) {
        const RUN_GAPS: [u8; CHECKPOINT_BLOCK / 8] = {
            let mut bytes = [u8::MAX; CHECKPOINT_BLOCK / 8];
            bytes[0] &= !1;
            bytes
        };
        let bytes = &self.list.bytes[self.data_at..];
        if self.gap_bits == 1 && self.count == CHECKPOINT_BLOCK && bytes.starts_with(&RUN_GAPS) {
            for (offset, slot) in self.slots.iter_mut().enumerate() {
                *slot = self.first.wrapping_add(offset as u32);
            }
        } else {
            let mut gaps = [0u64; CHECKPOINT_BLOCK];
            unpack_values(bytes, self.count, self.gap_bits, &mut gaps);
            let mut slot = self.first;
            for (out, &gap) in self.slots[..self.count].iter_mut().zip(&gaps[..self.count]) {
                slot = slot.wrapping_add(gap as u32);
                *out = slot;
            }
        }
        self.decoded = true;
        self.frequencies_ready = false;
        self.decodes += 1;
    }

    fn decode_frequencies(&mut self) {
        let mut frequencies = [0u64; CHECKPOINT_BLOCK];
        let gap_len = (self.count * usize::from(self.gap_bits)).div_ceil(8);
        unpack_values(
            &self.list.bytes[self.data_at + gap_len..],
            self.count,
            self.frequency_bits,
            &mut frequencies,
        );
        for (slot, &value) in self.frequencies.iter_mut().zip(&frequencies[..self.count]) {
            *slot = value as u32;
        }
        self.frequencies_ready = true;
    }

    /// Calls `f` with the slots from the current posting to the list's end, a block at a time.
    pub(super) fn for_each_block(&mut self, mut f: impl FnMut(&[u32])) {
        while !self.ended {
            if !self.decoded {
                self.decode();
            }
            f(&self.slots[self.index..self.count]);
            if self.left_after == 0 {
                self.ended = true;
            } else {
                self.load(self.block + 1, self.end_at, self.last);
            }
        }
    }

    /// The slot of the current posting, [`NO_DOC`] at the end.
    #[inline]
    pub(super) fn doc(&self) -> u32 {
        if self.ended {
            NO_DOC
        } else if self.decoded {
            self.slots[self.index]
        } else {
            self.first
        }
    }

    pub(super) fn is_end(&self) -> bool {
        self.ended
    }

    /// The frequency of the current posting.
    #[inline]
    pub(super) fn frequency(&mut self) -> u32 {
        if !self.decoded {
            self.decode();
        }
        if !self.frequencies_ready {
            self.decode_frequencies();
        }
        self.frequencies[self.index]
    }

    /// The postings in the list.
    pub(super) fn len(&self) -> usize {
        self.list.count as usize
    }

    /// The largest frequency and least word count of the whole list.
    pub(super) fn list_bounds(&self) -> (u32, u32) {
        (self.list_frequency, self.list_words)
    }

    /// The current block's last slot.
    #[inline]
    pub(super) fn block_last(&self) -> u32 {
        self.last
    }

    /// Every slot of the current block, from its first posting whatever the cursor's position in
    /// it. A cursor that ended has none.
    pub(super) fn block_slots(&mut self) -> &[u32] {
        if self.ended {
            return &[];
        }
        if !self.decoded {
            self.decode();
        }
        &self.slots[..self.count]
    }

    /// The largest frequency and least word count of the current block.
    #[inline]
    pub(super) fn block_bounds(&self) -> (u32, u32) {
        if self.blocks >= 2 {
            (self.skip(3, self.block), self.skip(4, self.block))
        } else {
            (self.list_frequency, self.list_words)
        }
    }

    /// The points of item `item`'s frontier (blocks first, then runs), empty for a malformed
    /// table.
    fn points(&self, item: usize) -> Points<'a> {
        let skips = self.list.skips;
        let directory = SKIP_ENTRY * self.blocks;
        let data = directory + 4 * (self.blocks + self.runs + 1);
        let empty = Points {
            bytes: &[],
            at: 0,
            left: 0,
        };
        let offset = |index: usize| -> Option<usize> {
            let at = directory + 4 * index;
            let word = skips.get(at..at + 4)?;
            Some(u32::from_le_bytes([word[0], word[1], word[2], word[3]]) as usize)
        };
        let (Some(start), Some(end)) = (offset(item), offset(item + 1)) else {
            return empty;
        };
        let Some(bytes) = skips.get(data + start..data + end) else {
            return empty;
        };
        let mut at = 0;
        if bytes.is_empty() {
            return empty;
        }
        let left = get_varint(bytes, &mut at) as usize;
        Points { bytes, at, left }
    }

    /// The current block's frontier, empty when it has the one point of its two maxima (and
    /// for a list of one block).
    #[inline]
    pub(super) fn block_points(&self) -> Points<'a> {
        if self.blocks < 2 {
            return Points {
                bytes: &[],
                at: 0,
                left: 0,
            };
        }
        self.points(self.block)
    }

    /// A bound of the weights in the current block from its frontier, never above the bound
    /// of its two maxima.
    pub(super) fn block_weight(&self, bound: Bound) -> f64 {
        if self.weight_block.get() == self.block {
            return self.weight.get();
        }
        let weight = self.block_weight_uncached(bound);
        self.weight_block.set(self.block);
        self.weight.set(weight);
        weight
    }

    /// Reads block weights from `table`, the list's rank table for the query's BM25 shape.
    pub(super) fn use_ranks(&mut self, table: Option<&'a RankTable>) {
        self.ranks = table;
        self.weight_block.set(usize::MAX);
    }

    fn block_weight_uncached(&self, bound: Bound) -> f64 {
        if let Some(table) = self.ranks {
            return RankTable::scale(table.blocks[self.block], bound.idf());
        }
        let (frequency, words) = self.block_bounds();
        let loose = bound.at(frequency, words);
        let mut points = self.block_points();
        if points.is_empty() {
            return loose;
        }
        let mut best = 0.0f64;
        for (f, w) in &mut points {
            best = best.max(bound.at(f, w));
            if best >= loose {
                break;
            }
        }
        best.min(loose)
    }

    /// A bound of the weights in run `run` from its frontier.
    pub(super) fn run_weight(&self, run: usize, bound: Bound) -> f64 {
        self.run_points(run)
            .fold(0.0f64, |best, (f, w)| best.max(bound.at(f, w)))
    }

    /// The list's skip table, which identifies it within its checkpoint.
    pub(super) fn skips(&self) -> &'a [u8] {
        self.list.skips
    }

    /// The blocks of the list.
    pub(super) fn block_count(&self) -> usize {
        self.blocks
    }

    /// Moves to block `block`, in either direction, without decoding it.
    pub(super) fn goto_block(&mut self, block: usize) {
        if block == 0 {
            let (_, _, first_at) = self.list.header();
            self.ended = false;
            self.load(0, first_at, 0);
        } else {
            let (at, previous) = (self.skip(1, block) as usize, self.skip(0, block - 1));
            self.ended = false;
            self.load(block, at, previous);
        }
    }

    /// The slots and frequencies of the current block.
    pub(super) fn block_postings(&mut self) -> (&[u32], &[u32]) {
        if !self.decoded {
            self.decode();
        }
        if !self.frequencies_ready {
            self.decode_frequencies();
        }
        (&self.slots[..self.count], &self.frequencies[..self.count])
    }

    /// A bound of the weights in the blocks from the current one through the one holding
    /// slot `last`, when they are at most `most` blocks; `None` otherwise.
    pub(super) fn span_weight(
        &self,
        last: u32,
        bound: Bound,
        most: usize,
        use_ranks: bool,
    ) -> Option<f64> {
        if self.blocks < 2 {
            return Some(bound.at(self.list_frequency, self.list_words));
        }
        let end = (self.block + most).min(self.blocks);
        let ranks = self.ranks.filter(|_| use_ranks);
        let mut weight = 0.0f64;
        for block in self.block..end {
            let block_weight = match ranks {
                Some(table) => RankTable::scale(table.blocks[block], bound.idf()),
                None => bound.at(self.skip(3, block), self.skip(4, block)),
            };
            weight = weight.max(block_weight);
            if self.skip(0, block) >= last {
                return Some(weight);
            }
        }
        (end == self.blocks).then_some(weight)
    }

    /// The list the cursor reads.
    pub(super) fn list(&self) -> ListRef<'a> {
        self.list
    }

    /// The list's block bytes, whose address identifies the list.
    pub(super) fn list_bytes(&self) -> &'a [u8] {
        self.list.bytes
    }

    /// The frequencies of the current block's postings, from its first.
    pub(super) fn block_frequencies(&mut self) -> &[u32] {
        self.block_postings().1
    }

    /// The run holding the current block.
    #[inline]
    pub(super) fn run(&self) -> usize {
        self.block / RUN_BLOCKS
    }

    /// The runs of the list.
    pub(super) fn runs(&self) -> usize {
        self.runs
    }

    /// Run `run`'s frontier (a list of at least two blocks).
    pub(super) fn run_points(&self, run: usize) -> Points<'a> {
        self.points(self.blocks + run)
    }

    /// The last slot of run `run`.
    pub(super) fn run_last(&self, run: usize) -> u32 {
        self.skip(0, ((run + 1) * RUN_BLOCKS).min(self.blocks) - 1)
    }

    /// Moves to the first posting after the current one.
    pub(super) fn next(&mut self) {
        if self.ended {
            return;
        }
        if !self.decoded && self.count > 1 {
            self.decode();
        }
        self.index += 1;
        if self.index < self.count {
            return;
        }
        if self.left_after == 0 {
            self.ended = true;
            return;
        }
        self.load(self.block + 1, self.end_at, self.last);
    }

    /// The first slot of the current block.
    #[inline]
    pub(super) fn block_first(&self) -> u32 {
        self.first
    }

    /// Moves to the first block at or after the current one whose last slot reaches `target`,
    /// without decoding it; false (and the cursor ended) when there is none.
    pub(super) fn advance_block(&mut self, target: u32) -> bool {
        if self.ended {
            return false;
        }
        if self.last >= target {
            return true;
        }
        if self.blocks < 2 {
            self.ended = true;
            return false;
        }
        // Gallop from the current block, then binary search the bracket.
        let mut step = 1usize;
        let mut lo = self.block + 1;
        let mut hi = lo;
        while hi < self.blocks && self.skip(0, hi) < target {
            lo = hi + 1;
            hi += step;
            step *= 2;
        }
        let mut hi = hi.min(self.blocks);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.skip(0, mid) < target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == self.blocks {
            self.ended = true;
            return false;
        }
        let at = self.skip(1, lo) as usize;
        let previous = self.skip(0, lo - 1);
        self.load(lo, at, previous);
        true
    }

    /// Moves to the first posting at or after slot `target`; returns its slot.
    pub(super) fn advance_to(&mut self, target: u32) -> u32 {
        if self.ended {
            return NO_DOC;
        }
        if self.doc() >= target {
            return self.doc();
        }
        if !self.advance_block(target) {
            return NO_DOC;
        }
        if self.first >= target {
            return self.first;
        }
        if !self.decoded {
            self.decode();
        }
        // Count the postings below `target`, a block of sixteen at a time: sorted slots make the
        // count the distance to the first posting at or after it, and the compare vectorizes.
        let mut at = self.index + 1;
        let end = self.count;
        while at < end {
            let chunk = (end - at).min(16);
            let below: usize = self.slots[at..at + chunk]
                .iter()
                .map(|&slot| usize::from(slot < target))
                .sum();
            at += below;
            if below < chunk {
                break;
            }
        }
        self.index = at;
        self.slots[at]
    }

    /// The token positions of the current posting, as the gap bytes the chunk holds.
    pub(super) fn position_bytes(&mut self) -> &'a [u8] {
        if !self.decoded {
            self.decode();
        }
        let (start, end) = self.position_range(self.index);
        &self.list.positions[start as usize..end as usize]
    }

    /// Where the positions of the current block's posting `index` lie in
    /// [`Self::list_positions`]; the block's slots must have been decoded.
    pub(super) fn position_range(&mut self, index: usize) -> (u32, u32) {
        if !self.positions_ready {
            let at = if self.blocks >= 2 {
                self.skip(2, self.block) as usize
            } else {
                0
            };
            let region = &self.list.positions[at..];
            let width = region[0];
            let mut lengths = [0u64; CHECKPOINT_BLOCK];
            unpack_values(&region[1..], self.count, width, &mut lengths);
            let mut total = 0u32;
            self.starts[0] = 0;
            for (i, &length) in lengths[..self.count].iter().enumerate() {
                total += length as u32;
                self.starts[i + 1] = total;
            }
            self.positions_at = at + 1 + (self.count * usize::from(width)).div_ceil(8);
            self.positions_ready = true;
        }
        let base = self.positions_at as u32;
        (base + self.starts[index], base + self.starts[index + 1])
    }

    /// The list's positions bytes.
    pub(super) fn list_positions(&self) -> &'a [u8] {
        self.list.positions
    }
}

/// Token positions from a posting's gap bytes, ascending.
pub(super) struct PositionIter<'a> {
    bytes: &'a [u8],
    position: u32,
}

impl<'a> PositionIter<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    #[cold]
    fn long_gap(&mut self) -> u32 {
        let mut at = 0;
        let gap = get_varint(self.bytes, &mut at) as u32;
        self.bytes = &self.bytes[at..];
        gap
    }
}

impl Iterator for PositionIter<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        let (&first, rest) = self.bytes.split_first()?;
        let gap = if first < 0x80 {
            self.bytes = rest;
            u32::from(first)
        } else if let Some((&second, tail)) = rest.split_first().filter(|(byte, _)| **byte < 0x80) {
            self.bytes = tail;
            u32::from(first & 0x7f) | u32::from(second) << 7
        } else {
            self.long_gap()
        };
        self.position += gap;
        Some(self.position)
    }
}

/// Whether the slot bitmap `bits` (see [`Cursor::dense`]) holds `slot`.
#[inline(always)]
pub(super) fn holds(bits: &[u8], slot: u32) -> bool {
    let byte = slot as usize / 8;
    byte < bits.len() && bits[byte] >> (slot % 8) & 1 == 1
}

/// Word `index` of the slot bitmap `bits`, zero past its end.
#[inline(always)]
pub(super) fn dense_word(bits: &[u8], index: usize) -> u64 {
    match bits.get(8 * index..8 * index + 8) {
        Some(word) => u64::from_le_bytes(word.try_into().expect("eight bytes")),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::documents::postings::CheckpointPacker;

    #[test]
    fn lexical_contiguous_slots_require_the_exact_packed_gaps() {
        for count in [CHECKPOINT_BLOCK - 1, CHECKPOINT_BLOCK] {
            let mut bytes = Vec::new();
            let mut packer = CheckpointPacker::with_info(true, &|_| 5, false);
            for slot in 7..7 + count {
                packer.push(slot, 1, &mut bytes);
            }
            packer.finish(&mut bytes);
            let list = ListRef {
                bytes: &bytes,
                count: count as u32,
                skips: &[],
                positions: &[],
            };
            let mut cursor = Cursor::open(list);
            assert!(cursor.block_slots().iter().copied().eq(7..7 + count as u32));
            let data_at = cursor.data_at;
            let mut changed = bytes.clone();
            changed[data_at] |= 1;
            let mut cursor = Cursor::open(ListRef {
                bytes: &changed,
                ..list
            });
            assert!(cursor.block_slots().iter().copied().eq(8..8 + count as u32));
        }
    }
}
