// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Postings per block in the checkpoint's block encoding. The checkpoint format fixes it,
/// independent of the build-time [`BLOCK_LEN`] of the live lists.
pub(in super::super) const CHECKPOINT_BLOCK: usize = 128;

/// One term's postings read from a mapped checkpoint, in one of its two encodings.
///
/// Blocked (checkpoint version 2): blocks of [`CHECKPOINT_BLOCK`] postings, the last one
/// shorter. Per block, a varint of its first slot minus the previous block's last slot (the
/// first block's is the slot itself), a byte of gap bits and a byte of frequency bits, then the
/// block's slot gaps (the first one zero) and its frequencies, each packed as a little-endian
/// bit string at its width.
///
/// Blocked with impacts (checkpoint version 3): the list starts with two varints, the largest
/// frequency of the list and the smallest word count of any chunk it holds. Each block then
/// carries, after its two width bytes, three varints: its last slot minus its first (`span`),
/// its largest frequency and the smallest word count among its chunks. A weight of any
/// posting in the block is at most the BM25 weight of that frequency at that word count, so a
/// reader may skip a block it cannot need without decoding it (see `impact.rs`).
///
/// Flat (version 1): per posting one varint `gap << 1 | (frequency != 1)`, followed by the
/// frequency as a varint only when it is not 1 (the first gap is the slot itself).
#[derive(Clone, Copy)]
pub(in super::super) struct Postings<'a> {
    pub(in super::super) bytes: &'a [u8],
    pub(in super::super) count: u32,
    pub(in super::super) blocked: bool,
    /// Checkpoint version 3: the blocked encoding plus the impact fields (see below).
    pub(in super::super) impacts: bool,
}

/// Packs postings pushed in ascending slot order into the checkpoint's block encoding; with
/// `impacts`, the version 3 form whose bounds come from `words`, the word count of a slot. One
/// packer writes one list at a time, whether its postings come from one in-memory list or from a
/// checkpoint's list merged with a live one, so both produce the same bytes.
pub(in super::super) struct CheckpointPacker<'w> {
    pub(super) block: Vec<Posting>,
    pub(super) last: usize,
    pub(super) impacts: bool,
    pub(super) words: &'w dyn Fn(usize) -> u32,
    pub(super) list_start: Option<usize>,
    pub(super) list_frequency: u32,
    pub(super) list_words: u32,
    /// The skip-table facts of each block of the current list, when asked for.
    pub(super) infos: Option<Vec<BlockInfo>>,
}

impl<'w> CheckpointPacker<'w> {
    /// A packer that also records each block's [`BlockInfo`] (see [`Self::take_infos`]).
    pub(in super::super) fn with_info(
        impacts: bool,
        words: &'w dyn Fn(usize) -> u32,
        collect: bool,
    ) -> Self {
        Self {
            block: Vec::with_capacity(CHECKPOINT_BLOCK),
            last: 0,
            impacts,
            words,
            list_start: None,
            list_frequency: 0,
            list_words: u32::MAX,
            infos: collect.then(Vec::new),
        }
    }

    /// The block facts of the list just finished, in block order.
    pub(in super::super) fn take_infos(&mut self) -> Vec<BlockInfo> {
        self.infos.as_mut().map(std::mem::take).unwrap_or_default()
    }

    pub(in super::super) fn push(&mut self, slot: usize, frequency: u32, out: &mut Vec<u8>) {
        self.block.push((slot, frequency));
        if self.block.len() == CHECKPOINT_BLOCK {
            self.flush(out);
        }
    }

    /// Writes the last, shorter block and the list header; the packer then starts a new list.
    pub(in super::super) fn finish(&mut self, out: &mut Vec<u8>) {
        if !self.block.is_empty() {
            self.flush(out);
        }
        if let (true, Some(start)) = (self.impacts, self.list_start) {
            let mut header = Vec::with_capacity(8);
            put_varint(&mut header, u64::from(self.list_frequency));
            put_varint(&mut header, u64::from(self.list_words));
            if let Some(infos) = self.infos.as_mut() {
                for entry in infos.iter_mut() {
                    entry.offset += header.len() as u32;
                }
            }
            out.splice(start..start, header);
        }
        self.last = 0;
        self.list_start = None;
        self.list_frequency = 0;
        self.list_words = u32::MAX;
    }

    pub(super) fn flush(&mut self, out: &mut Vec<u8>) {
        let list_start = *self.list_start.get_or_insert(out.len());
        let block_offset = out.len() - list_start;
        let block = &self.block;
        let first = block[0].0;
        put_varint(out, (first - self.last) as u64);
        let gap = block
            .windows(2)
            .map(|pair| (pair[1].0 - pair[0].0) as u64)
            .max()
            .unwrap_or(0);
        let frequency = block.iter().map(|p| p.1).max().expect("nonempty block");
        let gap_bits = (64 - gap.leading_zeros()) as u8;
        let frequency_bits = (32 - frequency.leading_zeros()) as u8;
        out.extend_from_slice(&[gap_bits, frequency_bits]);
        if self.impacts {
            let mut lengths = [0u32; CHECKPOINT_BLOCK];
            for (length, p) in lengths.iter_mut().zip(block) {
                *length = (self.words)(p.0);
            }
            let lengths = &lengths[..block.len()];
            let least = lengths.iter().copied().min().expect("nonempty block");
            put_varint(out, (block[block.len() - 1].0 - first) as u64);
            put_varint(out, u64::from(frequency));
            put_varint(out, u64::from(least));
            self.list_frequency = self.list_frequency.max(frequency);
            self.list_words = self.list_words.min(least);
            if let Some(infos) = self.infos.as_mut() {
                infos.push(BlockInfo {
                    last: block[block.len() - 1].0 as u32,
                    offset: block_offset as u32,
                    max_frequency: frequency,
                    min_words: least,
                    frontier: frontier(block.iter().map(|p| p.1).zip(lengths.iter().copied())),
                });
            }
        }
        let mut previous = first;
        pack(
            out,
            block.iter().map(|p| {
                let gap = p.0 - previous;
                previous = p.0;
                gap as u64
            }),
            gap_bits,
        );
        pack(out, block.iter().map(|p| u64::from(p.1)), frequency_bits);
        self.last = previous;
        self.block.clear();
    }
}

/// Appends `list`'s postings to `out` in the checkpoint's block encoding; with `impacts`, the
/// version 3 form whose bounds come from `words`, the word count of a slot.
pub(in super::super) fn pack_checkpoint(
    list: &PostingList,
    words: &dyn Fn(usize) -> u32,
    impacts: bool,
    out: &mut Vec<u8>,
    info: Option<&mut Vec<BlockInfo>>,
) {
    let mut packer = CheckpointPacker::with_info(impacts, words, info.is_some());
    list.for_each(|slot, frequency| packer.push(slot, frequency, out));
    packer.finish(out);
    if let Some(info) = info {
        info.extend(packer.take_infos());
    }
}

/// What the checkpoint's skip table records of one posting block: its last slot, the offset of
/// its header within the list's bytes, its largest frequency, its least word count and its
/// frontier (`None` when the block's postings have one undominated point, the two maxima).
#[derive(Clone)]
pub(in super::super) struct BlockInfo {
    pub(in super::super) last: u32,
    pub(in super::super) offset: u32,
    pub(in super::super) max_frequency: u32,
    pub(in super::super) min_words: u32,
    pub(in super::super) frontier: Option<Box<[(u32, u32)]>>,
}

/// Appends `list`'s postings to `out` in the version 1 flat encoding, for tests of reading it.
#[cfg(test)]
pub(in super::super) fn pack_checkpoint_flat(list: &PostingList, out: &mut Vec<u8>) {
    let mut previous = 0usize;
    let mut first = true;
    list.for_each(|slot, frequency| {
        let gap = if first { slot } else { slot - previous };
        first = false;
        previous = slot;
        put_varint(out, (gap as u64) << 1 | u64::from(frequency != 1));
        if frequency != 1 {
            put_varint(out, u64::from(frequency));
        }
    });
}

/// The smallest byte length of a list of `count` postings: three bytes per block when blocked,
/// six and a two byte list header with impacts, one per posting when flat.
pub(in super::super) fn checkpoint_min_len(count: u64, blocked: bool, impacts: bool) -> u64 {
    let blocks = count.div_ceil(CHECKPOINT_BLOCK as u64);
    if impacts {
        2 + 6 * blocks
    } else if blocked {
        3 * blocks
    } else {
        count
    }
}

/// [`get_varint`] for bytes read from a file: fails instead of panicking on a malformed varint.
pub(super) fn checked_varint(bytes: &[u8], at: &mut usize) -> anyhow::Result<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| anyhow::anyhow!("posting list ends inside a varint"))?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    anyhow::bail!("posting varint overflow")
}

impl Postings<'_> {
    pub(in super::super) fn len(&self) -> usize {
        self.count as usize
    }

    /// Calls `f(slot, frequency)` for every posting in slot order, checking every field and
    /// that the list ends exactly after `count` postings; `f` returning false stops early.
    ///
    /// A list with impacts also has every recorded bound checked against its postings; `words`
    /// gives the word count of a slot for the word count bounds.
    pub(in super::super) fn try_for_each(
        &self,
        words: &dyn Fn(usize) -> u32,
        f: impl FnMut(usize, u32) -> bool,
    ) -> anyhow::Result<()> {
        if self.blocked {
            self.try_for_each_blocked(words, f)
        } else {
            self.try_for_each_flat(f)
        }
    }

    pub(super) fn try_for_each_blocked(
        &self,
        words: &dyn Fn(usize) -> u32,
        mut f: impl FnMut(usize, u32) -> bool,
    ) -> anyhow::Result<()> {
        let bytes = self.bytes;
        let (mut at, mut last, mut left) = (0usize, 0u64, self.count as usize);
        let recorded = if self.impacts {
            Some((
                checked_varint(bytes, &mut at)?,
                checked_varint(bytes, &mut at)?,
            ))
        } else {
            None
        };
        let (mut list_frequency, mut list_words) = (0u64, u64::from(u32::MAX));
        let (mut gaps, mut frequencies) = ([0u64; CHECKPOINT_BLOCK], [0u64; CHECKPOINT_BLOCK]);
        while left > 0 {
            let count = left.min(CHECKPOINT_BLOCK);
            let delta = checked_varint(bytes, &mut at)?;
            let header = bytes
                .get(at..at + 2)
                .ok_or_else(|| anyhow::anyhow!("posting block header past the list"))?;
            let (gap_bits, frequency_bits) = (header[0], header[1]);
            anyhow::ensure!(
                gap_bits <= 64 && frequency_bits <= 32,
                "posting block bit widths"
            );
            at += 2;
            let block_impacts = if self.impacts {
                Some((
                    checked_varint(bytes, &mut at)?,
                    checked_varint(bytes, &mut at)?,
                    checked_varint(bytes, &mut at)?,
                ))
            } else {
                None
            };
            let gap_len = (count * usize::from(gap_bits)).div_ceil(8);
            let frequency_len = (count * usize::from(frequency_bits)).div_ceil(8);
            let end = at + gap_len + frequency_len;
            anyhow::ensure!(end <= bytes.len(), "posting block past the list");
            unpack_values(&bytes[at..at + gap_len], count, gap_bits, &mut gaps);
            unpack_values(
                &bytes[at + gap_len..end],
                count,
                frequency_bits,
                &mut frequencies,
            );
            at = end;
            anyhow::ensure!(gaps[0] == 0, "posting block first gap");
            let mut slot = last
                .checked_add(delta)
                .ok_or_else(|| anyhow::anyhow!("posting slot overflow"))?;
            anyhow::ensure!(
                left == self.count as usize || delta > 0,
                "posting blocks out of order"
            );
            let first = slot;
            let (mut most, mut least) = (0u64, u64::from(u32::MAX));
            for (index, (&gap, &frequency)) in
                gaps[..count].iter().zip(&frequencies[..count]).enumerate()
            {
                anyhow::ensure!(index == 0 || gap > 0, "posting gap");
                slot = slot
                    .checked_add(gap)
                    .ok_or_else(|| anyhow::anyhow!("posting slot overflow"))?;
                most = most.max(frequency);
                if block_impacts.is_some() {
                    least = least.min(u64::from(words(usize::try_from(slot)?)));
                }
                if !f(usize::try_from(slot)?, u32::try_from(frequency)?) {
                    return Ok(());
                }
            }
            if let Some((span, frequency, smallest)) = block_impacts {
                anyhow::ensure!(
                    span == slot - first && frequency == most && smallest == least,
                    "posting block impacts"
                );
                list_frequency = list_frequency.max(most);
                list_words = list_words.min(least);
            }
            last = slot;
            left -= count;
        }
        anyhow::ensure!(
            recorded.is_none_or(|bounds| bounds == (list_frequency, list_words)),
            "posting list impacts"
        );
        anyhow::ensure!(at == bytes.len(), "posting list has trailing bytes");
        Ok(())
    }

    pub(super) fn try_for_each_flat(
        &self,
        mut f: impl FnMut(usize, u32) -> bool,
    ) -> anyhow::Result<()> {
        let (mut at, mut slot) = (0usize, 0u64);
        for _ in 0..self.count {
            let word = checked_varint(self.bytes, &mut at)?;
            let frequency = if word & 1 == 0 {
                1
            } else {
                let frequency = checked_varint(self.bytes, &mut at)?;
                anyhow::ensure!(frequency != 1, "packed posting frequency");
                frequency
            };
            slot = slot
                .checked_add(word >> 1)
                .ok_or_else(|| anyhow::anyhow!("posting slot overflow"))?;
            let frequency = u32::try_from(frequency)?;
            if !f(usize::try_from(slot)?, frequency) {
                return Ok(());
            }
        }
        anyhow::ensure!(at == self.bytes.len(), "posting list has trailing bytes");
        Ok(())
    }

    /// Calls `f(slot, frequency)` for every posting in slot order.
    pub(in super::super) fn for_each(&self, f: impl FnMut(usize, u32)) {
        if self.blocked {
            self.for_each_blocked(f);
        } else {
            self.for_each_flat(f);
        }
    }

    /// [`Self::for_each`] over the block encoding: each block's gaps and frequencies unpack
    /// with the live lists' constant-width (and SIMD) decoders.
    pub(super) fn for_each_blocked(&self, mut f: impl FnMut(usize, u32)) {
        let bytes = self.bytes;
        let (mut at, mut last, mut left) = (0usize, 0u64, self.count as usize);
        if self.impacts {
            get_varint(bytes, &mut at);
            get_varint(bytes, &mut at);
        }
        let (mut gaps, mut frequencies) = ([0u64; CHECKPOINT_BLOCK], [0u64; CHECKPOINT_BLOCK]);
        while left > 0 {
            let count = left.min(CHECKPOINT_BLOCK);
            let delta = get_varint(bytes, &mut at);
            let (gap_bits, frequency_bits) = (bytes[at], bytes[at + 1]);
            at += 2;
            if self.impacts {
                for _ in 0..3 {
                    get_varint(bytes, &mut at);
                }
            }
            let gap_len = (count * usize::from(gap_bits)).div_ceil(8);
            // The decoders read whole 8-byte windows while they stay inside the slice they
            // are given, so each gets the rest of the list and uses only its own values.
            unpack_values(&bytes[at..], count, gap_bits, &mut gaps);
            unpack_values(
                &bytes[at + gap_len..],
                count,
                frequency_bits,
                &mut frequencies,
            );
            at += gap_len + (count * usize::from(frequency_bits)).div_ceil(8);
            let mut slot = last + delta;
            for (&gap, &frequency) in gaps[..count].iter().zip(&frequencies[..count]) {
                slot += gap;
                f(slot as usize, frequency as u32);
            }
            last = slot;
            left -= count;
        }
    }

    /// [`Self::for_each`] over the flat encoding; the loop form keeps the
    /// decoder state in registers where the iterator cannot. Postings decode from 64-bit
    /// loads: eight one-byte postings of frequency 1 (gaps below 64) at once, and otherwise
    /// every posting whose word and frequency are one or two bytes each (gaps below 8,192,
    /// frequencies below 16,384) without leaving the loaded word. Longer varints and the last
    /// bytes of the list take the byte-wise path.
    pub(super) fn for_each_flat(&self, mut f: impl FnMut(usize, u32)) {
        let bytes = self.bytes;
        let mut at = 0usize;
        let mut slot = 0u64;
        let mut left = self.count as usize;
        while left > 0 {
            if let Some(window) = bytes.get(at..at + 8) {
                let window = u64::from_le_bytes(window.try_into().expect("eight bytes"));
                if left >= 8 && window & 0x8181_8181_8181_8181 == 0 {
                    for byte in 0..8 {
                        slot += (window >> (8 * byte + 1)) & 0x7f;
                        f(slot as usize, 1);
                    }
                    at += 8;
                    left -= 8;
                    continue;
                }
                // A varint of one or two bytes starting `used` bytes into the window, and its
                // length; None when it is longer or runs past the window.
                let short = |used: usize| -> Option<(u64, usize)> {
                    let first = (window >> (8 * used)) & 0xff;
                    if first < 0x80 {
                        return Some((first, 1));
                    }
                    if used >= 7 {
                        return None;
                    }
                    let second = (window >> (8 * (used + 1))) & 0xff;
                    (second < 0x80).then_some(((first & 0x7f) | second << 7, 2))
                };
                let mut used = 0usize;
                while left > 0 && used < 8 {
                    let Some((word, word_len)) = short(used) else {
                        break;
                    };
                    let (frequency, frequency_len) = if word & 1 == 0 {
                        (1, 0)
                    } else if used + word_len < 8 {
                        match short(used + word_len) {
                            Some(decoded) => decoded,
                            None => break,
                        }
                    } else {
                        break;
                    };
                    slot += word >> 1;
                    f(slot as usize, frequency as u32);
                    used += word_len + frequency_len;
                    left -= 1;
                }
                at += used;
                if used > 0 {
                    continue;
                }
            }
            let word = get_varint(bytes, &mut at);
            slot += word >> 1;
            let frequency = if word & 1 == 0 {
                1
            } else {
                get_varint(bytes, &mut at)
            };
            debug_assert!(slot <= usize::MAX as u64 && frequency <= u64::from(u32::MAX));
            f(slot as usize, frequency as u32);
            left -= 1;
        }
    }
}
