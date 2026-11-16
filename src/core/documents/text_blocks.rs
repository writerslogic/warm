// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Compressed chunk text of a document checkpoint (`docs/file-format/CHECKPOINT.md`).
//!
//! The text of every chunk is laid end to end in document order and cut into blocks of about
//! [`BLOCK_TARGET`] bytes at chunk boundaries (a chunk is never split; a chunk larger than the
//! target is a block of its own). Each block is an LZ4 block compressed against one shared
//! dictionary of text sampled from the whole stream, so a block compresses well although it is
//! small enough to decode per hit. A block that does not shrink is stored as it is. A chunk
//! records its offset in the stream; its length is its byte span.
//!
//! Three sections: the dictionary, a table of `u32` pairs (the block's offset in the stream,
//! its offset in the block bytes) with one more pair at the end, and the block bytes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{ensure, Context, Result};

/// Stream bytes after which a new block starts.
pub(super) const BLOCK_TARGET: usize = 1;
/// The most dictionary bytes LZ4 can address.
const MAX_DICT: usize = 64 * 1024;
/// Streams shorter than this carry no dictionary.
const MIN_DICT_STREAM: usize = 16 * 1024;
/// Bytes sampled per dictionary piece.
const PIECE: usize = 512;
/// A block is a handful of chunks; this bounds what a corrupt table can make a reader allocate.
const MAX_BLOCK: usize = 16 << 20;

/// The three sections of the compressed text.
#[derive(Default)]
pub(super) struct Encoded {
    pub dict: Vec<u8>,
    pub table: Vec<u8>,
    pub blocks: Vec<u8>,
}

/// Collects chunk text in stream order.
#[derive(Default)]
pub(super) struct TextBuilder {
    stream: Vec<u8>,
    starts: Vec<u32>,
}

impl TextBuilder {
    /// Appends one chunk's text and returns its block and its offset in the block's text,
    /// `None` when the stream would no longer fit the `u32` offsets.
    pub(super) fn add(&mut self, text: &str) -> Option<(u64, u64)> {
        let end = u32::try_from(self.stream.len().checked_add(text.len())?).ok()?;
        let at = self.stream.len();
        // A block ends once it holds a target's worth, and a chunk of a target or more starts
        // a block of its own: decoding one hit never decodes a neighbour's large text.
        if self.starts.last().is_none_or(|&start| {
            at > start as usize
                && (at - start as usize >= BLOCK_TARGET || text.len() >= BLOCK_TARGET)
        }) {
            self.starts.push(at as u32);
        }
        self.stream.extend_from_slice(text.as_bytes());
        debug_assert_eq!(self.stream.len(), end as usize);
        let start = *self.starts.last().expect("a block was started") as usize;
        Some(((self.starts.len() - 1) as u64, (at - start) as u64))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.stream.is_empty()
    }

    /// Compresses the blocks. `None` when the compressed bytes would not fit `u32` offsets.
    pub(super) fn finish(self) -> Option<Encoded> {
        let dict = sample_dictionary(&self.stream);
        let (mut table, mut blocks) = (Vec::new(), Vec::new());
        for (i, &start) in self.starts.iter().enumerate() {
            let end = self
                .starts
                .get(i + 1)
                .map_or(self.stream.len(), |&next| next as usize);
            let raw = &self.stream[start as usize..end];
            table.extend_from_slice(&start.to_le_bytes());
            table.extend_from_slice(&u32::try_from(blocks.len()).ok()?.to_le_bytes());
            let packed = lz4_flex::block::compress_with_dict(raw, &dict);
            blocks.extend_from_slice(if packed.len() < raw.len() {
                &packed
            } else {
                raw
            });
        }
        table.extend_from_slice(&u32::try_from(self.stream.len()).ok()?.to_le_bytes());
        table.extend_from_slice(&u32::try_from(blocks.len()).ok()?.to_le_bytes());
        Some(Encoded {
            dict,
            table,
            blocks,
        })
    }
}

/// Evenly spaced pieces of the stream, up to an eighth of it and at most [`MAX_DICT`] bytes.
fn sample_dictionary(stream: &[u8]) -> Vec<u8> {
    if stream.len() < MIN_DICT_STREAM {
        return Vec::new();
    }
    let wanted = (stream.len() / 8).min(MAX_DICT) / PIECE;
    let stride = stream.len() / wanted.max(1);
    let mut dict = Vec::with_capacity(wanted * PIECE);
    for piece in 0..wanted {
        let at = piece * stride;
        dict.extend_from_slice(&stream[at..(at + PIECE).min(stream.len())]);
    }
    dict
}

/// The validated sections of a checkpoint's compressed text.
#[derive(Clone, Copy)]
pub(super) struct TextBlocks<'a> {
    dict: &'a [u8],
    table: &'a [u8],
    blocks: &'a [u8],
}

/// The last block decoded: chunks of a document are neighbours in the stream.
pub(super) struct BlockCache {
    block: usize,
    raw: Vec<u8>,
}

impl Default for BlockCache {
    fn default() -> Self {
        Self {
            block: usize::MAX,
            raw: Vec::new(),
        }
    }
}

/// Decoded blocks kept for the hits that ask for them again (a block cache as a log-structured
/// store keeps one). Filled first come, first kept, up to a byte budget; a block that does not fit
/// is decoded again on each read. Lock-free reads.
#[derive(Default)]
pub(super) struct DecodedBlocks {
    slots: Box<[OnceLock<Arc<str>>]>,
    used: AtomicUsize,
    budget: usize,
}

impl DecodedBlocks {
    pub(super) fn new(blocks: usize, budget: usize) -> Self {
        Self {
            slots: if budget == 0 {
                Box::default()
            } else {
                (0..blocks).map(|_| OnceLock::new()).collect()
            },
            used: AtomicUsize::new(0),
            budget,
        }
    }

    fn get(&self, block: usize) -> Option<&Arc<str>> {
        self.slots.get(block)?.get()
    }

    /// Keeps `text` for `block` when the budget has room.
    fn offer(&self, block: usize, text: &str) {
        let Some(slot) = self.slots.get(block) else {
            return;
        };
        if self.used.fetch_add(text.len(), Ordering::Relaxed) + text.len() > self.budget
            || slot.set(Arc::from(text)).is_err()
        {
            self.used.fetch_sub(text.len(), Ordering::Relaxed);
        }
    }
}

impl<'a> TextBlocks<'a> {
    /// Checks the table once: every block starts where the previous one ended, no block is
    /// empty or larger than [`MAX_BLOCK`], the block bytes are exactly the blocks, and a block
    /// is never longer than the bytes it decodes to.
    pub(super) fn new(dict: &'a [u8], table: &'a [u8], blocks: &'a [u8]) -> Result<Self> {
        let this = Self::unchecked(dict, table, blocks);
        this.check()?;
        Ok(this)
    }

    /// The sections of a checkpoint whose [`Self::new`] succeeded, without checking them again.
    pub(super) fn unchecked(dict: &'a [u8], table: &'a [u8], blocks: &'a [u8]) -> Self {
        Self {
            dict,
            table,
            blocks,
        }
    }

    fn check(&self) -> Result<()> {
        let (dict, table, blocks) = (self.dict, self.table, self.blocks);
        ensure!(dict.len() <= MAX_DICT, "checkpoint text dictionary size");
        ensure!(
            table.len() >= 16 && table.len().is_multiple_of(8),
            "checkpoint text table size"
        );
        let this = self;
        let count = this.count();
        ensure!(this.entry(0) == (0, 0), "checkpoint text table start");
        for i in 0..count {
            let (raw, packed) = (this.entry(i), this.entry(i + 1));
            let (raw_len, packed_len) = (
                u64::from(packed.0.saturating_sub(raw.0)),
                u64::from(packed.1.saturating_sub(raw.1)),
            );
            ensure!(
                packed.0 > raw.0
                    && packed.1 >= raw.1
                    && raw_len as usize <= MAX_BLOCK
                    && packed_len <= raw_len,
                "checkpoint text block bounds"
            );
        }
        ensure!(
            this.entry(count).1 as usize == blocks.len(),
            "checkpoint text blocks have trailing bytes"
        );
        Ok(())
    }

    /// Bytes the compression saves over the raw text: a cache of decoded blocks this large keeps
    /// the text resident no larger than the raw text a store without compression maps.
    pub(super) fn saved(&self) -> usize {
        let (raw, packed) = self.entry(self.count());
        raw.saturating_sub(packed) as usize
    }

    fn count(&self) -> usize {
        self.table.len() / 8 - 1
    }

    /// The dictionary bytes and the whole decoded stream, for tests of what a checkpoint holds.
    #[cfg(test)]
    pub(super) fn contents(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut stream = Vec::new();
        let mut cache = BlockCache::default();
        for block in 0..self.count() {
            let size = (self.entry(block + 1).0 - self.entry(block).0) as usize;
            let text = self.read(block as u64, 0, size, &mut cache)?;
            stream.extend_from_slice(text.as_bytes());
        }
        Ok((self.dict.to_vec(), stream))
    }

    fn entry(&self, i: usize) -> (u32, u32) {
        let at = 8 * i;
        (
            u32::from_le_bytes(self.table[at..at + 4].try_into().expect("four bytes")),
            u32::from_le_bytes(self.table[at + 4..at + 8].try_into().expect("four bytes")),
        )
    }

    /// Checks that `len` bytes at `from` lie in `block`, and returns the block's index.
    fn locate(&self, block: u64, from: u64, len: usize) -> Result<usize> {
        let block = usize::try_from(block)
            .ok()
            .filter(|&block| block < self.count())
            .context("checkpoint text block")?;
        let size = u64::from(self.entry(block + 1).0 - self.entry(block).0);
        ensure!(
            from.checked_add(len as u64).is_some_and(|end| end <= size),
            "checkpoint text crosses a block"
        );
        Ok(block)
    }

    /// [`Self::read`] through `shared`: a block already decoded there costs a copy of the
    /// bytes asked for; a block decoded now is offered to it.
    pub(super) fn read_shared(
        &self,
        block: u64,
        from: u64,
        len: usize,
        shared: &DecodedBlocks,
    ) -> Result<String> {
        let block = self.locate(block, from, len)?;
        let (from, to) = (from as usize, from as usize + len);
        let boundary = "checkpoint text is not on a character boundary";
        if let Some(text) = shared.get(block) {
            return Ok(text.get(from..to).context(boundary)?.to_owned());
        }
        let mut cache = BlockCache::default();
        self.decode(block, &mut cache)?;
        let text = std::str::from_utf8(&cache.raw).context("checkpoint text is not UTF-8")?;
        shared.offer(block, text);
        if from == 0 && len == cache.raw.len() {
            cache.block = usize::MAX;
            return Ok(String::from_utf8(std::mem::take(&mut cache.raw)).expect("validated"));
        }
        Ok(text.get(from..to).context(boundary)?.to_owned())
    }

    /// Decodes `block` into `cache`.
    fn decode(&self, block: usize, cache: &mut BlockCache) -> Result<()> {
        let ((raw_start, packed_start), (raw_end, packed_end)) =
            (self.entry(block), self.entry(block + 1));
        cache.block = usize::MAX;
        let raw_len = (raw_end - raw_start) as usize;
        let packed = &self.blocks[packed_start as usize..packed_end as usize];
        cache.raw.clear();
        if packed.len() == raw_len {
            cache.raw.extend_from_slice(packed);
        } else {
            cache.raw.resize(raw_len, 0);
            let written =
                lz4_flex::block::decompress_into_with_dict(packed, &mut cache.raw, self.dict)
                    .map_err(|error| anyhow::anyhow!("checkpoint text block: {error}"))?;
            ensure!(written == raw_len, "checkpoint text block length");
        }
        cache.block = block;
        Ok(())
    }

    /// The `len` bytes at `from` in `block`, as text.
    pub(super) fn read(
        &self,
        block: u64,
        from: u64,
        len: usize,
        cache: &mut BlockCache,
    ) -> Result<String> {
        let block = self.locate(block, from, len)?;
        let (from, to) = (from as usize, from as usize + len);
        if cache.block != block {
            self.decode(block, cache)?;
        }
        if from == 0 && len == cache.raw.len() {
            // The chunk is the whole block: hand over the buffer instead of copying it.
            cache.block = usize::MAX;
            return String::from_utf8(std::mem::take(&mut cache.raw))
                .context("checkpoint text is not UTF-8");
        }
        Ok(std::str::from_utf8(&cache.raw[from..to])
            .context("checkpoint text is not UTF-8")?
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(chunks: usize) -> Vec<String> {
        (0..chunks)
            .map(|i| {
                format!(
                    "chunk {i}: the quick brown fox {} jumps over the lazy dog, again and again{}",
                    i * 7919 % 1013,
                    "é".repeat(i % 5)
                )
                .repeat(1 + i % 9)
            })
            .collect()
    }

    fn build(texts: &[String]) -> (Vec<(u64, u64)>, Encoded) {
        let mut builder = TextBuilder::default();
        let places = texts
            .iter()
            .map(|text| builder.add(text).expect("fits"))
            .collect();
        (places, builder.finish().expect("fits"))
    }

    #[test]
    fn every_chunk_reads_back_whatever_the_block_boundaries() {
        for chunks in [1usize, 2, 5, 40, 700] {
            let texts = corpus(chunks);
            let (places, encoded) = build(&texts);
            let blocks = TextBlocks::new(&encoded.dict, &encoded.table, &encoded.blocks).unwrap();
            let mut cache = BlockCache::default();
            for (text, &(block, from)) in texts.iter().zip(&places).rev() {
                let read = blocks.read(block, from, text.len(), &mut cache).unwrap();
                assert_eq!(read, *text);
            }
        }
    }

    #[test]
    fn a_chunk_larger_than_a_block_is_one_block_and_never_split() {
        let texts = vec![
            "a".repeat(3 * BLOCK_TARGET),
            "b".repeat(10),
            "c".repeat(BLOCK_TARGET),
        ];
        let (places, encoded) = build(&texts);
        let blocks = TextBlocks::new(&encoded.dict, &encoded.table, &encoded.blocks).unwrap();
        assert_eq!(blocks.count(), 3);
        let mut cache = BlockCache::default();
        for (text, &(block, from)) in texts.iter().zip(&places) {
            let read = blocks.read(block, from, text.len(), &mut cache).unwrap();
            assert_eq!(read, *text);
        }
    }

    #[test]
    fn reads_outside_a_chunk_a_block_or_the_table_are_errors() {
        let texts = corpus(40);
        let (places, encoded) = build(&texts);
        let blocks = TextBlocks::new(&encoded.dict, &encoded.table, &encoded.blocks).unwrap();
        let mut cache = BlockCache::default();
        let last = places.len() - 1;
        let (block, from) = places[last];
        let len = texts[last].len();
        // limit, limit + 1 at the end of a block
        assert!(blocks.read(block, from, len, &mut cache).is_ok());
        assert!(blocks.read(block, from, len + 1, &mut cache).is_err());
        assert!(blocks.read(block, from + 1, len, &mut cache).is_err());
        // the last block and one past it
        assert!(blocks
            .read(blocks.count() as u64 - 1, 0, 1, &mut cache)
            .is_ok());
        assert!(blocks
            .read(blocks.count() as u64, 0, 1, &mut cache)
            .is_err());
        assert!(blocks.read(u64::MAX, 0, 1, &mut cache).is_err());
        assert!(blocks.read(0, u64::MAX, 1, &mut cache).is_err());
    }

    #[test]
    fn corrupt_tables_and_blocks_are_refused_not_trusted() {
        let texts = corpus(40);
        let (_, encoded) = build(&texts);
        assert!(TextBlocks::new(&encoded.dict, &encoded.table[..8], &encoded.blocks).is_err());
        assert!(TextBlocks::new(&encoded.dict, &encoded.table, &encoded.blocks[1..]).is_err());
        let mut damaged = encoded.blocks.clone();
        damaged
            .iter_mut()
            .for_each(|byte| *byte = byte.wrapping_add(0x5b));
        let blocks = TextBlocks::new(&encoded.dict, &encoded.table, &damaged).unwrap();
        let mut cache = BlockCache::default();
        let reads_wrong = (0..blocks.count() as u64).any(|block| {
            blocks
                .read(block, 0, 1, &mut cache)
                .map_or(true, |text| text != "c")
        });
        assert!(reads_wrong);
    }
}
