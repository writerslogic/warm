// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Postings per block. Blocks are derived memory state, rebuilt from the journal and never
/// persisted (see `docs/file-format/index.md`), so the length is a build-time parameter that
/// no stored byte records: `WARM_POSTINGS_BLOCK_LEN` selects it for sweeps.
pub(in super::super) const BLOCK_LEN: usize = block_len(option_env!("WARM_POSTINGS_BLOCK_LEN"));
/// The fewest postings a block other than the last keeps after in-place removals.
pub(super) const MIN_BLOCK_LEN: usize = BLOCK_LEN / 4;

/// `setting` as a block length: a multiple of 8 from 32 to 4096, or 128 when unset.
pub(super) const fn block_len(setting: Option<&str>) -> usize {
    let Some(setting) = setting else {
        return 128;
    };
    let digits = setting.as_bytes();
    assert!(!digits.is_empty(), "WARM_POSTINGS_BLOCK_LEN is empty");
    let mut value = 0usize;
    let mut at = 0;
    while at < digits.len() {
        assert!(
            digits[at].is_ascii_digit(),
            "WARM_POSTINGS_BLOCK_LEN is not a number"
        );
        value = value * 10 + (digits[at] - b'0') as usize;
        assert!(value <= 4096, "WARM_POSTINGS_BLOCK_LEN above 4096");
        at += 1;
    }
    assert!(
        value >= 32 && value.is_multiple_of(8),
        "WARM_POSTINGS_BLOCK_LEN must be a multiple of 8 from 32"
    );
    value
}

#[derive(Clone, Debug)]
pub(super) struct Block {
    pub(super) offset: usize,
    pub(super) first: usize,
    pub(super) last: usize,
    pub(super) count: u16,
    pub(super) doc_bits: u8,
    pub(super) frequency_bits: u8,
    /// Packed chunk-term ordinal plus one; zero denotes a non-positional posting.
    pub(super) ordinal_bits: u8,
    pub(super) max_frequency: u32,
    pub(super) min_words: u32,
    pub(super) frontier: Option<Box<[(u32, u32)]>>,
}

#[derive(Default)]
pub(in super::super) struct PostingList {
    pub(super) data: Vec<u8>,
    pub(super) blocks: Vec<Block>,
    pub(super) count: u32,
    pub(super) positive_count: u32,
    pub(super) last: u64,
    /// For a list of at least [`SUMMARY_MIN_BLOCKS`] blocks, the frontier of each run of
    /// [`SUPER_LEN`] blocks, computed when the list is written ([`Self::seal`]).
    pub(super) summary: Option<Box<Summary>>,
}

/// Undominated `(frequency, words)` points, by frequency descending.
pub(in super::super) type Frontier = Box<[(u32, u32)]>;

#[cfg(test)]
thread_local! {
    /// Block frontiers computed on this thread, for tests of what an edit recomputes.
    static FRONTIERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Block frontiers computed on this thread so far.
#[cfg(test)]
pub(in super::super) fn frontiers_computed() -> usize {
    FRONTIERS.with(std::cell::Cell::get)
}

/// Facts about a long list's blocks, computed when the list is written (see
/// [`PostingList::seal`]): reading every block's metadata of a long list costs a query more
/// than the result, so queries only read them.
#[derive(Default)]
pub(super) struct Summary {
    /// The blocks changed since the facts were computed; a query then reads the blocks.
    stale: bool,
    /// While stale: the blocks `first..=last` changed, and `shifted` says later blocks moved.
    first: usize,
    last: usize,
    shifted: bool,
    maxima: (u32, u32),
    /// The maxima over every run but the last, kept so that a write to the last run (the usual
    /// append) re-seals in constant time.
    head: (u32, u32),
    runs: Vec<Frontier>,
}

/// The highest frequency and fewest words of `runs`: a frontier lists its highest frequency
/// first and its fewest words last.
fn run_maxima(runs: &[Frontier]) -> (u32, u32) {
    runs.iter()
        .fold((0, u32::MAX), |(frequency, words), frontier| {
            (
                frontier.first().map_or(frequency, |p| frequency.max(p.0)),
                frontier.last().map_or(words, |p| words.min(p.1)),
            )
        })
}

/// Blocks per run that one summary frontier covers.
pub(in super::super) const SUPER_LEN: usize = 32;
/// The fewest blocks a list summarizes: shorter lists are scanned block by block.
pub(super) const SUMMARY_MIN_BLOCKS: usize = 2 * SUPER_LEN;

/// The points of `points` no other point dominates (a larger frequency at no greater
/// length), by frequency descending; never empty for a nonempty input.
pub(in super::super) fn pareto(mut points: Vec<(u32, u32)>) -> Box<[(u32, u32)]> {
    points.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut kept: Vec<(u32, u32)> = Vec::new();
    for point in points {
        if kept.last().is_none_or(|last| point.1 < last.1) {
            kept.push(point);
        }
    }
    kept.into_boxed_slice()
}

pub(super) fn merge_sorted(a: &[Posting], b: &[Posting]) -> Vec<Posting> {
    let mut merged = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => {
                merged.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                merged.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => panic!("duplicate document slot in posting"),
        }
    }
    merged.extend_from_slice(&a[i..]);
    merged.extend_from_slice(&b[j..]);
    merged
}

/// The `(frequency, words)` points of a block no other point dominates (a larger frequency
/// at no greater length), by frequency descending, when there are at least two.
pub(super) fn frontier(points: impl Iterator<Item = (u32, u32)>) -> Option<Box<[(u32, u32)]>> {
    #[cfg(test)]
    FRONTIERS.with(|count| count.set(count.get() + 1));
    // The least length per frequency: a table for the frequencies a block usually holds and
    // a short sorted spill for the rest.
    let mut least = [u32::MAX; BLOCK_LEN + 1];
    let mut spill = [(0u32, 0u32); BLOCK_LEN];
    let mut spilled = 0;
    let mut highest = 0usize;
    for (used, (frequency, words)) in points.filter(|p| p.0 > 0).enumerate() {
        assert!(used < BLOCK_LEN, "frontier exceeds posting block length");
        if let Some(entry) = least.get_mut(frequency as usize) {
            *entry = (*entry).min(words);
            highest = highest.max(frequency as usize);
        } else {
            spill[spilled] = (frequency, words);
            spilled += 1;
        }
    }
    let spill = &mut spill[..spilled];
    spill.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let table = (1..=highest)
        .rev()
        .filter(|&frequency| least[frequency] != u32::MAX)
        .map(|frequency| (frequency as u32, least[frequency]));
    let mut kept = [(0u32, 0u32); BLOCK_LEN];
    let mut count = 0;
    let mut shortest = u32::MAX;
    for point in spill.iter().copied().chain(table) {
        if count == 0 || point.1 < shortest {
            shortest = point.1;
            kept[count] = point;
            count += 1;
        }
    }
    (count > 1).then(|| kept[..count].into())
}

impl PostingList {
    pub(in super::super) fn len(&self) -> usize {
        self.count as usize
    }
    pub(in super::super) fn positive_len(&self) -> usize {
        debug_assert!(self.positive_count == 0 || self.blocks.iter().any(|b| b.max_frequency > 0));
        self.positive_count as usize
    }

    pub(in super::super) fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub(in super::super) fn iter(&self) -> PostingCursor<'_> {
        PostingCursor::new(self)
    }
    pub(in super::super) fn for_each(&self, mut f: impl FnMut(usize, u32)) {
        let mut decoded = [(0, 0); BLOCK_LEN];
        for block in &self.blocks {
            self.decode_values(block, &mut decoded);
            for &(slot, frequency) in &decoded[..usize::from(block.count)] {
                f(slot, frequency);
            }
        }
    }
    /// `f(slot, frequency, ordinal)` for every posting in slot order; `ordinal` is the chunk-term
    /// ordinal plus one of a positional posting, zero otherwise.
    pub(in super::super) fn for_each_with_ordinal(&self, mut f: impl FnMut(usize, u32, u32)) {
        let mut decoded = [(0, 0); BLOCK_LEN];
        let mut ordinals = [0u32; BLOCK_LEN];
        for block in &self.blocks {
            self.decode_values(block, &mut decoded);
            if block.ordinal_bits > 0 {
                self.decode_ordinals(block, &mut ordinals);
            } else {
                ordinals = [0; BLOCK_LEN];
            }
            for (&(slot, frequency), &ordinal) in
                decoded[..usize::from(block.count)].iter().zip(&ordinals)
            {
                f(slot, frequency, ordinal);
            }
        }
    }
    pub(in super::super) fn mark_slots(&self, matched: &mut [u64]) {
        if self.positive_count != self.count {
            self.for_each(|slot, frequency| {
                if frequency > 0 {
                    matched[slot / 64] |= 1 << (slot % 64);
                }
            });
            return;
        }
        let mut gaps = [0u64; BLOCK_LEN];
        for block in &self.blocks {
            let count = usize::from(block.count);
            unpack_values(&self.data[block.offset..], count, block.doc_bits, &mut gaps);
            let mut slot = block.first as u64;
            for &gap in &gaps[..count] {
                slot = slot.wrapping_add(gap);
                matched[slot as usize / 64] |= 1 << (slot % 64);
            }
            assert_eq!(
                slot, block.last as u64,
                "posting gaps do not reach the block end"
            );
        }
    }

    /// The frequency of a block's first posting, read without decoding the block.
    pub(super) fn first_frequency(&self, block: &Block) -> u32 {
        let gap_bytes = (usize::from(block.count) * usize::from(block.doc_bits)).div_ceil(8);
        unpack(
            &self.data[block.offset + gap_bytes..],
            0,
            block.frequency_bits,
        ) as u32
    }

    /// The largest block frequency and least block length bound of the list, which bound
    /// every posting's weight.
    pub(in super::super) fn maxima(&self) -> (u32, u32) {
        let fold = || {
            self.blocks
                .iter()
                .fold((0, u32::MAX), |(frequency, words), block| {
                    (
                        frequency.max(block.max_frequency),
                        words.min(block.min_words),
                    )
                })
        };
        match self.summary.as_deref() {
            Some(summary) if !summary.stale => summary.maxima,
            _ => fold(),
        }
    }
    pub(super) fn decode(&self) -> Vec<Posting> {
        self.iter().collect()
    }

    /// The number of blocks.
    pub(in super::super) fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Block `index`'s largest frequency, least length and frontier, which bound its
    /// postings' weights.
    pub(in super::super) fn block_bounds(&self, index: usize) -> ((u32, u32), &[(u32, u32)]) {
        let block = &self.blocks[index];
        (
            (block.max_frequency, block.min_words),
            block.frontier.as_deref().unwrap_or(&[]),
        )
    }

    /// Decodes block `index` into `out` and returns its posting count.
    pub(in super::super) fn decode_block_into(
        &self,
        index: usize,
        out: &mut [Posting; BLOCK_LEN],
    ) -> usize {
        self.decode_block(index, out);
        usize::from(self.blocks[index].count)
    }

    pub(super) fn decode_block(&self, index: usize, out: &mut [Posting; BLOCK_LEN]) {
        self.decode_values(&self.blocks[index], out);
    }

    pub(super) fn decode_values(&self, block: &Block, out: &mut [Posting; BLOCK_LEN]) {
        let count = usize::from(block.count);
        let bytes = &self.data[block.offset..];
        let gap_bytes = (count * usize::from(block.doc_bits)).div_ceil(8);
        let mut values = [0u64; BLOCK_LEN];
        unpack_values(bytes, count, block.doc_bits, &mut values);
        // Gaps sum to at most the block's slot span, so the running sum cannot overflow for a
        // valid block; the end check catches a corrupt one.
        let mut slot = block.first as u64;
        for (posting, &gap) in out[..count].iter_mut().zip(&values[..count]) {
            slot = slot.wrapping_add(gap);
            posting.0 = slot as usize;
        }
        assert_eq!(
            slot, block.last as u64,
            "posting gaps do not reach the block end"
        );
        unpack_values(
            &bytes[gap_bytes..],
            count,
            block.frequency_bits,
            &mut values,
        );
        for (posting, &frequency) in out[..count].iter_mut().zip(&values[..count]) {
            posting.1 = frequency as u32;
        }
    }

    pub(super) fn ordinal_bytes(&self, block: &Block) -> &[u8] {
        let count = usize::from(block.count);
        let start = block.offset
            + (count * usize::from(block.doc_bits)).div_ceil(8)
            + (count * usize::from(block.frequency_bits)).div_ceil(8);
        &self.data[start..start + (count * usize::from(block.ordinal_bits)).div_ceil(8)]
    }

    pub(super) fn decode_ordinals(&self, block: &Block, out: &mut [u32]) {
        let mut values = [0; BLOCK_LEN];
        unpack_values(
            self.ordinal_bytes(block),
            usize::from(block.count),
            block.ordinal_bits,
            &mut values,
        );
        for (out, value) in out.iter_mut().zip(values) {
            *out = value as u32;
        }
    }

    pub(super) fn append_block(
        &mut self,
        postings: &[Posting],
        ordinals: &[u32],
        min_words: u32,
        frontier: Option<Box<[(u32, u32)]>>,
    ) {
        let mut gap = 0u64;
        for pair in postings.windows(2) {
            gap = gap.max((pair[1].0 - pair[0].0) as u64);
        }
        let max_frequency = postings.iter().map(|p| p.1).max().expect("nonempty block");
        let doc_bits = (64 - gap.leading_zeros()) as u8;
        let frequency_bits = (32 - max_frequency.leading_zeros()) as u8;
        let offset = self.data.len();
        let mut previous = postings[0].0;
        pack(
            &mut self.data,
            postings.iter().map(|p| {
                let delta = p.0 - previous;
                previous = p.0;
                delta as u64
            }),
            doc_bits,
        );
        pack(
            &mut self.data,
            postings.iter().map(|p| u64::from(p.1)),
            frequency_bits,
        );
        let maximum = ordinals.iter().copied().max().unwrap_or(0);
        let ordinal_bits = (32 - maximum.leading_zeros()) as u8;
        if ordinal_bits > 0 {
            assert_eq!(ordinals.len(), postings.len());
            pack(
                &mut self.data,
                ordinals.iter().map(|&n| u64::from(n)),
                ordinal_bits,
            );
        }
        self.blocks.push(Block {
            offset,
            first: postings[0].0,
            last: postings.last().unwrap().0,
            count: postings.len() as u16,
            doc_bits,
            frequency_bits,
            ordinal_bits,
            max_frequency,
            min_words,
            frontier,
        });
        self.touch_range(self.blocks.len() - 1, self.blocks.len() - 1, false);
    }

    /// Rewrites the list as `postings`, with each block's length bounds taken from `words`
    /// as appends take them, so a rewrite keeps block-max pruning as tight as a fresh build.
    #[cfg(test)]
    pub(super) fn encode(&mut self, postings: &[Posting], words: impl Fn(usize) -> u32) {
        self.encode_with_ordinals(postings, &[], words);
    }

    pub(super) fn encode_with_ordinals(
        &mut self,
        postings: &[Posting],
        ordinals: &[u32],
        words: impl Fn(usize) -> u32,
    ) {
        assert!(ordinals.is_empty() || ordinals.len() == postings.len());
        self.data.clear();
        self.blocks.clear();
        self.touch();
        for (index, block) in postings.chunks(BLOCK_LEN).enumerate() {
            let start = index * BLOCK_LEN;
            self.append_block(
                block,
                ordinals.get(start..start + block.len()).unwrap_or(&[]),
                block.iter().map(|p| words(p.0)).min().unwrap_or(0),
                frontier(block.iter().map(|p| (p.1, words(p.0)))),
            );
        }
        self.data.shrink_to_fit();
        self.count = u32::try_from(postings.len()).expect("posting list over u32::MAX");
        self.positive_count = postings.iter().filter(|p| p.1 > 0).count() as u32;
        self.last = postings.last().map_or(0, |p| p.0 as u64);
        self.seal();
    }

    /// See [`PostingLists::permute_slots`].
    pub(super) fn permute_slots(&mut self, map: &[u32], words: &[u32]) {
        if self.is_empty() {
            return;
        }
        let mut moved: Vec<(u32, u32, u32)> = Vec::with_capacity(self.len());
        let mut positional = false;
        self.for_each_with_ordinal(|slot, frequency, ordinal| {
            positional |= ordinal != 0;
            moved.push((map[slot], frequency, ordinal));
        });
        moved.sort_unstable_by_key(|posting| posting.0);
        let postings: Vec<Posting> = moved
            .iter()
            .map(|&(slot, frequency, _)| (slot as usize, frequency))
            .collect();
        let ordinals: Vec<u32> = if positional {
            moved.iter().map(|posting| posting.2).collect()
        } else {
            Vec::new()
        };
        drop(moved);
        self.encode_with_ordinals(&postings, &ordinals, |slot| words[slot]);
    }

    #[cfg(test)]
    pub(in super::super) fn append_run(&mut self, run: &[Posting]) {
        self.append_run_with_lengths(run, |_| 0);
    }

    #[cfg(test)]
    pub(super) fn append_run_with_lengths(
        &mut self,
        run: &[Posting],
        words: impl Fn(usize) -> u32,
    ) {
        self.append_run_with_ordinals(run, &[], words);
    }

    pub(super) fn append_run_with_ordinals(
        &mut self,
        run: &[Posting],
        ordinals: &[u32],
        words: impl Fn(usize) -> u32,
    ) {
        self.append_run_unsealed(run, ordinals, words);
        self.seal();
    }

    fn append_run_unsealed(
        &mut self,
        run: &[Posting],
        ordinals: &[u32],
        words: impl Fn(usize) -> u32,
    ) {
        assert!(ordinals.is_empty() || ordinals.len() == run.len());
        debug_assert!(
            run.windows(2).all(|p| p[0].0 < p[1].0),
            "unsorted posting run"
        );
        let Some(&(first, _)) = run.first() else {
            return;
        };
        if self.blocks.capacity() == 0 {
            self.blocks.reserve_exact(run.len().div_ceil(BLOCK_LEN));
        }
        if self.count > 0 && first as u64 <= self.last {
            if let [posting] = run {
                // A replacement's posting returns to its slot: one block takes it.
                self.insert(*posting, ordinals.first().copied().unwrap_or(0), words);
                return;
            }
            let merged = merge_sorted(&self.decode(), run);
            let mut cursor = self.iter();
            let mut next = 0;
            let merged_ordinals: Vec<_> = merged
                .iter()
                .map(|&(slot, _)| {
                    if next < run.len() && run[next].0 == slot {
                        let ordinal = ordinals.get(next).copied().unwrap_or(0);
                        next += 1;
                        ordinal
                    } else {
                        assert_eq!(cursor.advance(slot).map(|p| p.0), Some(slot));
                        cursor.ordinal()
                    }
                })
                .collect();
            self.encode_with_ordinals(&merged, &merged_ordinals, words);
            return;
        }
        let mut start = 0;
        if let Some(block) = self.blocks.pop_if(|b| usize::from(b.count) < BLOCK_LEN) {
            let mut decoded = [(0, 0); BLOCK_LEN];
            self.decode_values(&block, &mut decoded);
            let mut block_ordinals = [0; BLOCK_LEN];
            self.decode_ordinals(&block, &mut block_ordinals);
            self.data.truncate(block.offset);
            let old_len = usize::from(block.count);
            let take = (BLOCK_LEN - old_len).min(run.len());
            decoded[old_len..old_len + take].copy_from_slice(&run[..take]);
            if !ordinals.is_empty() {
                block_ordinals[old_len..old_len + take].copy_from_slice(&ordinals[..take]);
            }
            let min_words = run[..take]
                .iter()
                .map(|p| words(p.0))
                .min()
                .unwrap_or(0)
                .min(block.min_words);
            let fallback = (block.max_frequency, block.min_words);
            let old_points = block
                .frontier
                .as_deref()
                .unwrap_or(std::slice::from_ref(&fallback));
            let frontier = frontier(
                old_points
                    .iter()
                    .copied()
                    .chain(run[..take].iter().map(|p| (p.1, words(p.0)))),
            );
            self.append_block(
                &decoded[..old_len + take],
                &block_ordinals[..old_len + take],
                min_words,
                frontier,
            );
            start = take;
        }
        for (index, block) in run[start..].chunks(BLOCK_LEN).enumerate() {
            let offset = start + index * BLOCK_LEN;
            self.append_block(
                block,
                ordinals.get(offset..offset + block.len()).unwrap_or(&[]),
                block.iter().map(|p| words(p.0)).min().unwrap_or(0),
                frontier(block.iter().map(|p| (p.1, words(p.0)))),
            );
        }
        self.count = u32::try_from(self.len() + run.len()).expect("posting list over u32::MAX");
        self.positive_count += run.iter().filter(|p| p.1 > 0).count() as u32;
        self.last = run.last().unwrap().0 as u64;
    }

    /// The block holding `slot`, decoded into `out`, and its posting count.
    pub(super) fn block_of(&self, slot: usize, out: &mut [Posting; BLOCK_LEN]) -> (usize, usize) {
        let block = self.blocks.partition_point(|b| b.last < slot);
        assert!(block < self.blocks.len(), "document missing from posting");
        (block, self.decode_block_into(block, out))
    }

    /// Re-encodes block `index` as `postings`: no block when they are empty, two halves when
    /// they overflow a block. The later blocks keep their encoding and bounds and only move,
    /// so an edit recomputes one block's bounds rather than the list's; blocks other than the
    /// last may then hold fewer than [`BLOCK_LEN`] postings.
    pub(super) fn rewrite_block(
        &mut self,
        index: usize,
        postings: &[Posting],
        ordinals: &[u32],
        words: impl Fn(usize) -> u32,
    ) {
        let start = self.blocks[index].offset;
        let end = self
            .blocks
            .get(index + 1)
            .map_or(self.data.len(), |block| block.offset);
        let mut scratch = PostingList::default();
        let size = if postings.len() > BLOCK_LEN {
            postings.len().div_ceil(2)
        } else {
            BLOCK_LEN
        };
        for (part, run) in postings.chunks(size).enumerate() {
            scratch.append_block(
                run,
                &ordinals[part * size..part * size + run.len()],
                run.iter().map(|p| words(p.0)).min().unwrap_or(0),
                frontier(run.iter().map(|p| (p.1, words(p.0)))),
            );
        }
        let length = scratch.data.len();
        self.data.splice(start..end, scratch.data);
        for block in &mut self.blocks[index + 1..] {
            block.offset = block.offset - (end - start) + length;
        }
        for block in &mut scratch.blocks {
            block.offset += start;
        }
        let parts = scratch.blocks.len();
        self.blocks.splice(index..=index, scratch.blocks);
        self.touch_range(index, index + parts.saturating_sub(1), parts != 1);
    }

    /// Inserts one posting before the list's last into its block.
    pub(super) fn insert(&mut self, posting: Posting, ordinal: u32, words: impl Fn(usize) -> u32) {
        let mut decoded = [(0, 0); BLOCK_LEN + 1];
        let block = self.blocks.partition_point(|b| b.last < posting.0);
        let count = {
            let out: &mut [Posting; BLOCK_LEN] =
                (&mut decoded[..BLOCK_LEN]).try_into().expect("block");
            self.decode_block_into(block, out)
        };
        let at = match decoded[..count].binary_search_by_key(&posting.0, |p| p.0) {
            Ok(_) => panic!("duplicate document slot in posting"),
            Err(at) => at,
        };
        let mut ordinals = [0; BLOCK_LEN + 1];
        self.decode_ordinals(&self.blocks[block], &mut ordinals[..BLOCK_LEN]);
        ordinals.copy_within(at..count, at + 1);
        ordinals[at] = ordinal;
        decoded.copy_within(at..count, at + 1);
        decoded[at] = posting;
        self.rewrite_block(block, &decoded[..count + 1], &ordinals[..count + 1], words);
        self.count += 1;
        self.positive_count += u32::from(posting.1 > 0);
        self.seal();
    }

    pub(super) fn remove(&mut self, slot: usize, words: impl Fn(usize) -> u32) {
        self.remove_unsealed(slot, words);
        self.seal();
    }

    fn remove_unsealed(&mut self, slot: usize, words: impl Fn(usize) -> u32) {
        let mut decoded = [(0, 0); BLOCK_LEN];
        let (block, count) = self.block_of(slot, &mut decoded);
        if count <= MIN_BLOCK_LEN && block + 1 < self.blocks.len() {
            // A block this short re-packs the list rather than fragment it further.
            let mut postings = self.decode();
            let index = postings
                .binary_search_by_key(&slot, |p| p.0)
                .expect("document missing from posting");
            let mut cursor = self.iter();
            let mut ordinals = Vec::with_capacity(postings.len());
            while cursor.peek().is_some() {
                ordinals.push(cursor.ordinal());
                cursor.next();
            }
            postings.remove(index);
            ordinals.remove(index);
            self.encode_with_ordinals(&postings, &ordinals, words);
            return;
        }
        let index = decoded[..count]
            .binary_search_by_key(&slot, |p| p.0)
            .expect("document missing from posting");
        let (_, frequency) = decoded[index];
        let mut ordinals = [0; BLOCK_LEN];
        self.decode_ordinals(&self.blocks[block], &mut ordinals);
        ordinals.copy_within(index + 1..count, index);
        decoded.copy_within(index + 1..count, index);
        self.rewrite_block(block, &decoded[..count - 1], &ordinals[..count - 1], words);
        self.count -= 1;
        self.positive_count -= u32::from(frequency > 0);
        self.last = self.blocks.last().map_or(0, |b| b.last as u64);
        if self.count == 0 {
            self.data = Vec::new();
            self.blocks = Vec::new();
            self.touch();
        }
    }

    #[cfg(test)]
    pub(super) fn set_frequency(
        &mut self,
        slot: usize,
        frequency: u32,
        words: impl Fn(usize) -> u32,
    ) {
        self.set_entry(slot, frequency, None, words);
    }

    pub(super) fn set_entry(
        &mut self,
        slot: usize,
        frequency: u32,
        ordinal: Option<u32>,
        words: impl Fn(usize) -> u32,
    ) {
        self.set_entry_unsealed(slot, frequency, ordinal, words);
        self.seal();
    }

    fn set_entry_unsealed(
        &mut self,
        slot: usize,
        frequency: u32,
        ordinal: Option<u32>,
        words: impl Fn(usize) -> u32,
    ) {
        let mut decoded = [(0, 0); BLOCK_LEN];
        let (block, count) = self.block_of(slot, &mut decoded);
        let index = decoded[..count]
            .binary_search_by_key(&slot, |p| p.0)
            .expect("document missing from posting");
        let mut ordinals = [0; BLOCK_LEN];
        self.decode_ordinals(&self.blocks[block], &mut ordinals);
        if frequency == decoded[index].1 && ordinal.is_none_or(|n| n == ordinals[index]) {
            return;
        }
        if let Some(ordinal) = ordinal {
            ordinals[index] = ordinal;
        }
        let previous = std::mem::replace(&mut decoded[index].1, frequency);
        self.positive_count =
            self.positive_count - u32::from(previous > 0) + u32::from(frequency > 0);
        self.rewrite_block(block, &decoded[..count], &ordinals[..count], words);
    }

    /// Whether every block's bounds hold for its postings under the lengths `words`: the
    /// largest frequency, the least length, and a frontier point dominating each posting.
    #[cfg(test)]
    pub(in super::super) fn bounds_cover(&self, words: &[u32]) -> bool {
        self.blocks.iter().enumerate().all(|(index, block)| {
            let mut postings = [(0, 0); BLOCK_LEN];
            self.decode_block(index, &mut postings);
            postings[..usize::from(block.count)]
                .iter()
                .all(|&(slot, frequency)| {
                    let length = words[slot];
                    frequency <= block.max_frequency
                        && block.min_words <= length
                        && (frequency == 0
                            || block.frontier.as_deref().is_none_or(|points| {
                                points.iter().any(|&(f, w)| frequency <= f && w <= length)
                            }))
                })
        })
    }

    /// Whether every block's length bounds are exactly those a fresh build from `words`
    /// gives: no rewrite loosened them.
    #[cfg(test)]
    pub(in super::super) fn bounds_are_tight(&self, words: &[u32]) -> bool {
        self.blocks.iter().enumerate().all(|(index, block)| {
            let mut postings = [(0, 0); BLOCK_LEN];
            self.decode_block(index, &mut postings);
            let postings = &postings[..usize::from(block.count)];
            block.min_words == postings.iter().map(|p| words[p.0]).min().unwrap_or(0)
                && block.frontier == frontier(postings.iter().map(|p| (p.1, words[p.0])))
        })
    }

    pub(in super::super) fn invalidate_length(&mut self, slot: usize) {
        let block = self.blocks.partition_point(|b| b.last < slot);
        self.blocks[block].min_words = 0;
        self.blocks[block].frontier = None;
        self.touch_range(block, block, false);
        self.seal();
    }

    /// Marks the summary stale after a change to every block; a short list has none. Every
    /// public write ends in [`Self::seal`].
    pub(super) fn touch(&mut self) {
        self.touch_range(0, usize::MAX, true);
    }

    /// Marks stale a change to blocks `first..=last`; `shifted` when blocks after `last` moved
    /// to other indices (the block count changed).
    fn touch_range(&mut self, first: usize, last: usize, shifted: bool) {
        if self.blocks.len() < SUMMARY_MIN_BLOCKS {
            self.summary = None;
            return;
        }
        let summary = self.summary.get_or_insert_with(|| {
            Box::new(Summary {
                stale: true,
                first: 0,
                last: usize::MAX,
                shifted: true,
                ..Summary::default()
            })
        });
        if summary.stale {
            summary.first = summary.first.min(first);
            summary.last = summary.last.max(last);
            summary.shifted |= shifted;
        } else {
            (summary.first, summary.last, summary.shifted) = (first, last, shifted);
            summary.stale = true;
        }
    }

    /// Computes the summary of a long list whose blocks changed, so no query does. Only the
    /// runs the change reaches are rebuilt: the one it falls in, or every later one when
    /// blocks moved.
    pub(super) fn seal(&mut self) {
        let Some(summary) = self.summary.as_deref_mut().filter(|summary| summary.stale) else {
            return;
        };
        let total = self.blocks.len().div_ceil(SUPER_LEN);
        let old_len = summary.runs.len();
        let moved = summary.shifted || old_len != total;
        let (from, to) = (summary.first / SUPER_LEN, summary.last / SUPER_LEN);
        let build = |run: usize| {
            let mut points = Vec::new();
            for block in
                &self.blocks[run * SUPER_LEN..((run + 1) * SUPER_LEN).min(self.blocks.len())]
            {
                match block.frontier.as_deref() {
                    Some(frontier) => points.extend_from_slice(frontier),
                    None => points.push((block.max_frequency, block.min_words)),
                }
            }
            pareto(points)
        };
        // The runs are edited in place, so a write to the last run costs one run, not a copy
        // of every run's pointer.
        let runs = &mut summary.runs;
        if moved {
            runs.truncate(from.min(total));
            for run in runs.len()..total {
                runs.push(build(run));
            }
        } else {
            let end = to.min(total - 1) + 1;
            for (run, slot) in runs.iter_mut().enumerate().take(end).skip(from) {
                *slot = build(run);
            }
        }
        // The runs before the last are untouched when the change starts in the last run, or in
        // the one before it when the list grew by one run: then the head is carried, and only the
        // tail is read. Any other change refolds the runs (still 1/SUPER_LEN of the blocks).
        let last = total - 1;
        let carried = if !summary.shifted && old_len == total && from >= last {
            Some(summary.head)
        } else if !summary.shifted && old_len + 1 == total && from + 1 >= last {
            let (frequency, words) = run_maxima(&summary.runs[last - 1..last]);
            Some((summary.head.0.max(frequency), summary.head.1.min(words)))
        } else {
            None
        };
        let head = carried.unwrap_or_else(|| run_maxima(&summary.runs[..last]));
        let (frequency, words) = run_maxima(&summary.runs[last..]);
        summary.head = head;
        summary.maxima = (head.0.max(frequency), head.1.min(words));
        summary.stale = false;
    }

    /// Whether the summary equals one computed from every block (the incremental seals agree
    /// with a full one).
    #[cfg(test)]
    pub(super) fn summary_matches_recompute(&self) -> bool {
        let Some(summary) = self.summary.as_deref() else {
            return self.blocks.len() < SUMMARY_MIN_BLOCKS;
        };
        let maxima = self
            .blocks
            .iter()
            .fold((0, u32::MAX), |(frequency, words), block| {
                (
                    frequency.max(block.max_frequency),
                    words.min(block.min_words),
                )
            });
        let runs: Vec<Frontier> = self
            .blocks
            .chunks(SUPER_LEN)
            .map(|run| {
                let mut points = Vec::new();
                for block in run {
                    match block.frontier.as_deref() {
                        Some(frontier) => points.extend_from_slice(frontier),
                        None => points.push((block.max_frequency, block.min_words)),
                    }
                }
                pareto(points)
            })
            .collect();
        let head = run_maxima(&runs[..runs.len().saturating_sub(1)]);
        !summary.stale && summary.maxima == maxima && *summary.runs == *runs && summary.head == head
    }

    /// The frontier of each run of [`SUPER_LEN`] blocks (a block without a frontier
    /// contributes its largest frequency at its least length), or `None` for a short list.
    pub(in super::super) fn summary(&self) -> Option<&[Frontier]> {
        self.summary
            .as_deref()
            .filter(|summary| !summary.stale)
            .map(|summary| &*summary.runs)
    }
}
