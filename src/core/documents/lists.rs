// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Readers of checkpoint version 4 posting lists: the version 3 block bytes
//! (`postings::Postings`) plus, per list, a skip table and a positions stream
//! (`docs/file-format/CHECKPOINT.md`).
//!
//! A list of `count` postings has `ceil(count / 128)` blocks. With two or more blocks its skip
//! table holds five little-endian `u32` arrays of that length, one after the other: each
//! block's last slot, the offset of its header in the list's bytes, the offset of its positions
//! in the list's positions bytes, its largest frequency and its least word count. A one block
//! list has no table (its block starts after the list header and its positions at zero).
//!
//! A block's positions are one width byte `w`, the byte length of each posting's positions
//! packed at `w` bits, then those positions end to end: each posting's token positions as
//! gaps (the first absolute) in unsigned LEB128, the bytes the chunk's own positional record
//! holds for the term.

use super::postings::{get_varint, unpack_values, CHECKPOINT_BLOCK};

/// The slot of a cursor with no posting left.
pub(super) const NO_DOC: u32 = u32::MAX;

/// Bytes of skip table per block of a list of two or more blocks.
const SKIP_ENTRY: usize = 20;

/// Bytes of skip table a list of `count` postings has.
pub(super) fn skip_len(count: u64) -> usize {
    let blocks = count.div_ceil(CHECKPOINT_BLOCK as u64) as usize;
    if blocks >= 2 {
        blocks * SKIP_ENTRY
    } else {
        0
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
        };
        cursor.load(0, first_at, 0);
        cursor
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
        let mut gaps = [0u64; CHECKPOINT_BLOCK];
        let bytes = self.list.bytes;
        unpack_values(&bytes[self.data_at..], self.count, self.gap_bits, &mut gaps);
        let mut slot = self.first;
        for (out, &gap) in self.slots[..self.count].iter_mut().zip(&gaps[..self.count]) {
            slot = slot.wrapping_add(gap as u32);
            *out = slot;
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
        for i in 0..self.count {
            self.frequencies[i] = frequencies[i] as u32;
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

    /// The largest frequency and least word count of the current block.
    #[inline]
    pub(super) fn block_bounds(&self) -> (u32, u32) {
        if self.blocks >= 2 {
            (self.skip(3, self.block), self.skip(4, self.block))
        } else {
            (self.list_frequency, self.list_words)
        }
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
        // Leapfrogging mostly moves a few postings: scan a short run before searching.
        let end = self.count;
        let mut at = self.index + 1;
        let near = (at + 8).min(end);
        while at < near && self.slots[at] < target {
            at += 1;
        }
        if at == near && at < end && self.slots[at] < target {
            at += self.slots[at..end].partition_point(|&s| s < target);
        }
        self.index = at;
        self.slots[at]
    }

    /// The token positions of the current posting, as the gap bytes the chunk holds.
    pub(super) fn position_bytes(&mut self) -> &'a [u8] {
        if !self.decoded {
            self.decode();
        }
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
            for i in 0..self.count {
                total += lengths[i] as u32;
                self.starts[i + 1] = total;
            }
            self.positions_at = at + 1 + (self.count * usize::from(width)).div_ceil(8);
            self.positions_ready = true;
        }
        let base = self.positions_at;
        let (start, end) = (self.starts[self.index], self.starts[self.index + 1]);
        &self.list.positions[base + start as usize..base + end as usize]
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
}

impl Iterator for PositionIter<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        let (&first, rest) = self.bytes.split_first()?;
        if first < 0x80 {
            self.bytes = rest;
            self.position += u32::from(first);
            return Some(self.position);
        }
        let mut at = 0;
        let gap = get_varint(self.bytes, &mut at) as u32;
        self.bytes = &self.bytes[at..];
        self.position += gap;
        Some(self.position)
    }
}
