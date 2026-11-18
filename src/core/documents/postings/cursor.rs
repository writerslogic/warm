// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// [`slice::partition_point`] by exponential search from the front: a cursor's targets are
/// usually near, so the probes stay in the cache lines it just read.
pub(super) fn gallop<T>(items: &[T], before: impl Fn(&T) -> bool) -> usize {
    let mut low = 0;
    let mut step = 1;
    while low + step <= items.len() && before(&items[low + step - 1]) {
        low += step;
        step *= 2;
    }
    let high = (low + step).min(items.len());
    low + items[low..high].partition_point(before)
}

/// A forward cursor over a list. A block is decoded only when the cursor needs a posting
/// after its first, so skipping over blocks by their bounds costs their metadata alone.
pub(in super::super) struct PostingCursor<'a> {
    pub(super) list: &'a PostingList,
    pub(super) block: usize,
    pub(super) index: usize,
    pub(super) decoded: [Posting; BLOCK_LEN],
    pub(super) length: usize,
    pub(super) filled: bool,
}

impl<'a> PostingCursor<'a> {
    pub(super) fn new(list: &'a PostingList) -> Self {
        let mut cursor = Self {
            list,
            block: 0,
            index: 0,
            decoded: [(0, 0); BLOCK_LEN],
            length: 0,
            filled: false,
        };
        cursor.load(0);
        cursor
    }

    pub(super) fn load(&mut self, block: usize) {
        self.block = block;
        self.index = 0;
        self.filled = false;
        match self.list.blocks.get(block) {
            Some(b) => {
                self.length = usize::from(b.count);
                self.decoded[0] = (b.first, self.list.first_frequency(b));
            }
            None => self.length = 0,
        }
    }

    pub(super) fn fill(&mut self) {
        if !self.filled {
            self.list.decode_block(self.block, &mut self.decoded);
            self.filled = true;
        }
    }

    pub(in super::super) fn peek(&self) -> Option<Posting> {
        (self.index < self.length).then(|| self.decoded[self.index])
    }

    pub(super) fn ordinal(&self) -> u32 {
        let block = &self.list.blocks[self.block];
        let bits = usize::from(block.ordinal_bits);
        let start = self.index * bits;
        let bytes = self.list.ordinal_bytes(block);
        let mut value = 0u64;
        for (offset, &byte) in bytes[start / 8..(start + bits).div_ceil(8)]
            .iter()
            .enumerate()
        {
            value |= u64::from(byte) << (offset * 8);
        }
        ((value >> (start % 8)) & ((1u64 << bits) - 1)) as u32
    }

    pub(in super::super) fn term_index(&self) -> usize {
        self.ordinal()
            .checked_sub(1)
            .expect("positional posting lacks term ordinal") as usize
    }

    /// Moves to the first block that can hold `target` without decoding it, so the block's
    /// bounds can be read first; `false` once no block can. The cursor's posting may then
    /// still precede `target` until [`Self::advance`].
    pub(in super::super) fn locate(&mut self, target: usize) -> bool {
        if self.index >= self.length {
            return false;
        }
        if self.list.blocks[self.block].last < target {
            let after = self.block + 1;
            let block = after + gallop(&self.list.blocks[after..], |b| b.last < target);
            self.load(block);
        }
        self.length > 0
    }

    /// The largest frequency and least length over the blocks at or after the cursor's that
    /// overlap slots `first..=last`, read without decoding; `None` when none overlaps.
    pub(in super::super) fn range_maximum(&self, first: usize, last: usize) -> Option<(u32, u32)> {
        if self.index >= self.length {
            return None;
        }
        let blocks = &self.list.blocks[self.block..];
        let start = blocks.partition_point(|b| b.last < first);
        blocks[start..]
            .iter()
            .take_while(|b| b.first <= last)
            .map(|b| (b.max_frequency, b.min_words))
            .reduce(|(f, w), (g, v)| (f.max(g), w.min(v)))
    }

    pub(in super::super) fn block_end(&self) -> Option<usize> {
        self.peek().map(|_| self.list.blocks[self.block].last)
    }

    /// The index of the cursor's block, which changes only when the cursor leaves it.
    pub(in super::super) fn block_index(&self) -> usize {
        self.block
    }

    /// Whether the cursor stands on a block it has not decoded.
    pub(in super::super) fn at_block_start(&self) -> bool {
        self.index == 0 && !self.filled && self.length > 0
    }

    #[cfg(test)]
    pub(in super::super) fn block_impacts(&self) -> &[(u32, u32)] {
        let block = &self.list.blocks[self.block];
        block.frontier.as_deref().unwrap_or(&[])
    }

    pub(in super::super) fn block_maximum(&self) -> (u32, u32) {
        let block = &self.list.blocks[self.block];
        (block.max_frequency, block.min_words)
    }

    pub(in super::super) fn advance(&mut self, target: usize) -> Option<Posting> {
        if self.peek().is_none_or(|p| p.0 >= target) {
            return self.peek();
        }
        // Blocks before the current one end below the target; only those after are searched.
        if !self.locate(target) || self.decoded[0].0 >= target && self.index == 0 {
            return self.peek();
        }
        self.fill();
        self.index += gallop(&self.decoded[self.index..self.length], |p| p.0 < target);
        self.peek()
    }
}

impl Iterator for PostingCursor<'_> {
    type Item = Posting;
    fn next(&mut self) -> Option<Posting> {
        let value = self.peek()?;
        self.index += 1;
        if self.index == self.length {
            self.load(self.block + 1);
        } else {
            self.fill();
        }
        Some(value)
    }
}
