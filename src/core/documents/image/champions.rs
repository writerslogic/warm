//! Champion postings of long lists (derived section kind 3, PREREG-W2 item 3).
//!
//! The champions of a list are its first [`CHAMPIONS`] postings in ranking order (score
//! descending, then the exhaustive tie order) under the default BM25 constants at the
//! checkpoint's own chunk count and average chunk length: the exact top of any query whose only
//! scoring term is that list. They are computed when the checkpoint is written (index, flush or
//! compaction time) and read in place; no query builds or keeps one.
//!
//! A list has champions when its table is no larger than its compressed bytes, and the lists
//! that do are taken longest first while their tables stay within 1/8 of the postings section.
//! Both are functions of the checkpoint alone, and open checks that the stored lists are exactly
//! that set.
//!
//! Block layout, little endian: chunk count, average bits and list count `E` (`u64` each), `E`
//! entries of skip-table offset and champion count (`u64` each, ascending offsets), then per
//! list its slots (`u32`, padded to eight bytes) and weights (`f64`), both in ranking order.
//! The block is read bytewise and needs no alignment.
//!
//! Champions are valid for a query only when its constants are the defaults and its chunk count
//! and average equal the block's, which the served paths (an intact checkpoint with no live
//! documents) guarantee by construction and [`ServingImage::champions`] checks.

use anyhow::{ensure, Context, Result};

use super::{ListSpan, Section, ServingImage};
use crate::core::documents::lexical::served::build_champions;

/// Postings kept per champion list: serves any `k` up to 1000 (the pruned path's ceiling) when
/// up to 3/4 of a list's best chunks are excluded, rounded up to a power of two
/// (`ceil(1000 / (1 - 3/4)) = 4000`).
pub(in super::super) const CHAMPIONS: usize = if cfg!(test) { 1024 } else { 4096 };

/// Directory kind of this block in the derived section.
pub(super) const KIND: u64 = 3;

/// Bytes a champion takes: a slot and a weight.
const HIT_BYTES: usize = 12;

/// Environment switch: `0` opens without champions.
const ENV: &str = "WARM_CHAMPIONS";

pub(super) fn enabled() -> bool {
    std::env::var_os(ENV).is_none_or(|value| value != "0")
}

/// Whether a list of `count` postings in `len` compressed bytes has champions: their table is no
/// larger than the list it shadows.
pub(super) fn qualifies(count: u64, len: usize) -> bool {
    HIT_BYTES.saturating_mul(count.min(CHAMPIONS as u64) as usize) <= len
}

/// Bytes of the slots of `len` champions, padded to eight.
fn slot_bytes(len: usize) -> usize {
    (4 * len).next_multiple_of(8)
}

/// Bytes one list takes in the block: its entry and its champions.
fn list_cost(len: usize) -> usize {
    16 + slot_bytes(len) + 8 * len
}

/// Where each list's champions are: skip-table offset, champion count and the offset of its
/// slots in the derived section, ascending by skip-table offset.
#[derive(Default)]
pub(super) struct ChampionIndex {
    chunks: u64,
    average: u64,
    lists: Vec<(usize, usize, usize)>,
}

/// One list's champions, borrowed from the checkpoint.
pub(in super::super) struct Champions<'a> {
    slots: &'a [u8],
    weights: &'a [u8],
}

impl Champions<'_> {
    /// The number of champions.
    pub(in super::super) fn len(&self) -> usize {
        self.slots.len() / 4
    }

    /// The `i`-th champion, as its `(weight, slot)`.
    #[inline]
    pub(in super::super) fn hit(&self, i: usize) -> (f64, u32) {
        let slot = u32::from_le_bytes(self.slots[4 * i..4 * i + 4].try_into().expect("four bytes"));
        let weight = f64::from_le_bytes(
            self.weights[8 * i..8 * i + 8]
                .try_into()
                .expect("eight bytes"),
        );
        (weight, slot)
    }

    /// The champions of ranks `range`.
    pub(in super::super) fn iter(
        &self,
        range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = (f64, u32)> + '_ {
        range.map(|i| self.hit(i))
    }
}

impl ServingImage {
    /// The lists that have champions, ascending by skip-table offset: a function of the
    /// checkpoint alone.
    fn champion_lists(&self) -> Vec<ListSpan> {
        let mut spans: Vec<ListSpan> = self
            .ranked
            .iter()
            .filter(|span| qualifies(span.entry.count, span.entry.len))
            .copied()
            .collect();
        spans.sort_unstable_by(|a, b| b.entry.count.cmp(&a.entry.count).then(a.skip.cmp(&b.skip)));
        let budget = self.section(Section::Postings).len() / 8;
        let mut used = 0usize;
        let kept = spans
            .iter()
            .take_while(|span| {
                used += list_cost((span.entry.count).min(CHAMPIONS as u64) as usize);
                used <= budget
            })
            .count();
        spans.truncate(kept);
        spans.sort_unstable_by_key(|span| span.skip);
        spans
    }

    /// The chunk count and average the champions are computed at.
    fn champion_shape(&self) -> (u64, f64) {
        let average = (self.words as f64 / (self.chunks as f64).max(1.0)).max(1.0);
        (self.chunks as u64, average)
    }

    /// The block of kind [`KIND`], or `None` when a score could not be computed (no champions
    /// are stored then).
    pub(super) fn champion_block(&self) -> Option<Vec<u8>> {
        let spans = self.champion_lists();
        let (chunks, average) = self.champion_shape();
        let n = chunks as f64;
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let found = std::sync::Mutex::new(Vec::with_capacity(spans.len()));
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(span) = spans.get(at) else { break };
                    let idf = super::super::Bm25::idf(n, span.entry.count as f64);
                    let hits = build_champions(self, self.ranked_list(span), idf, average);
                    found
                        .lock()
                        .expect("no panic holds the lock")
                        .push((at, hits));
                });
            }
        });
        let mut found = found.into_inner().expect("no panic holds the lock");
        found.sort_unstable_by_key(|(at, _)| *at);
        let mut out = Vec::new();
        for value in [chunks, average.to_bits(), spans.len() as u64] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        let mut lists = Vec::with_capacity(found.len());
        for (at, hits) in found {
            lists.push(hits.ok()?);
            out.extend_from_slice(&(spans[at].skip as u64).to_le_bytes());
            out.extend_from_slice(&(lists.last()?.len() as u64).to_le_bytes());
        }
        for hits in lists {
            for &(_, slot) in &hits {
                out.extend_from_slice(&slot.to_le_bytes());
            }
            out.resize(out.len().next_multiple_of(8), 0);
            for &(weight, _) in &hits {
                out.extend_from_slice(&weight.to_le_bytes());
            }
        }
        Some(out)
    }

    /// The champions stored in `block`, which starts `block_at` bytes into the derived section.
    /// Everything a query reads is checked against the checkpoint's own lists.
    pub(super) fn parse_champions(&self, block: &[u8], block_at: usize) -> Result<ChampionIndex> {
        let spans = self.champion_lists();
        let (chunks, average) = self.champion_shape();
        let word = |at: usize| -> Result<u64> {
            let bytes = block
                .get(at..at + 8)
                .context("derived champions are truncated")?;
            Ok(u64::from_le_bytes(bytes.try_into()?))
        };
        ensure!(
            word(0)? == chunks && word(8)? == average.to_bits(),
            "derived champions were made for another checkpoint"
        );
        ensure!(
            usize::try_from(word(16)?)? == spans.len(),
            "derived champions are not the checkpoint's longest lists"
        );
        let mut at = 24 + 16 * spans.len();
        let mut lists = Vec::with_capacity(spans.len());
        for (i, span) in spans.iter().enumerate() {
            let len = usize::try_from(word(32 + 16 * i)?)?;
            ensure!(
                word(24 + 16 * i)? == span.skip as u64
                    && len == (span.entry.count).min(CHAMPIONS as u64) as usize,
                "derived champions address a list that has none"
            );
            let slots = block
                .get(at..at + 4 * len)
                .context("derived champions are truncated")?;
            let weights = block
                .get(at + slot_bytes(len)..at + slot_bytes(len) + 8 * len)
                .context("derived champions are truncated")?;
            let mut previous = f64::INFINITY;
            for i in 0..len {
                let slot = u32::from_le_bytes(slots[4 * i..4 * i + 4].try_into()?);
                let weight = f64::from_le_bytes(weights[8 * i..8 * i + 8].try_into()?);
                ensure!(
                    (slot as usize) < self.slots && weight.is_finite() && weight <= previous,
                    "derived champions are out of order"
                );
                previous = weight;
            }
            lists.push((span.skip, len, block_at + at));
            at += list_cost(len) - 16;
        }
        ensure!(at == block.len(), "derived champions have the wrong length");
        Ok(ChampionIndex {
            chunks,
            average: average.to_bits(),
            lists,
        })
    }

    /// The champions of the list whose skip table is `skips`, one of this checkpoint's lists,
    /// when it has them and `average` is the one they were made at.
    pub(in super::super) fn champions(&self, skips: &[u8], average: f64) -> Option<Champions<'_>> {
        if self.champions.average != average.to_bits()
            || self.champions.chunks != self.chunks as u64
        {
            return None;
        }
        let section = self.section(Section::Skips);
        let offset = (skips.as_ptr() as usize).checked_sub(section.as_ptr() as usize)?;
        let found = self
            .champions
            .lists
            .binary_search_by_key(&offset, |entry| entry.0)
            .ok()?;
        let (_, len, at) = self.champions.lists[found];
        let derived = self.derived_bytes()?;
        Some(Champions {
            slots: &derived[at..at + 4 * len],
            weights: &derived[at + slot_bytes(len)..at + slot_bytes(len) + 8 * len],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_never_exceeds_its_list() {
        let full = HIT_BYTES * CHAMPIONS;
        assert!(!qualifies(CHAMPIONS as u64, full - 1));
        assert!(qualifies(CHAMPIONS as u64, full));
        assert!(qualifies(CHAMPIONS as u64, full + 1));
        // Short lists are bounded by their own count.
        assert!(!qualifies(10, 10 * HIT_BYTES - 1));
        assert!(qualifies(10, 10 * HIT_BYTES));
        assert!(qualifies(u64::MAX, usize::MAX));
    }

    #[test]
    fn the_block_cost_matches_its_layout() {
        for len in [0usize, 1, 2, 3, CHAMPIONS] {
            assert_eq!(list_cost(len), 16 + (4 * len).next_multiple_of(8) + 8 * len);
        }
    }
}
