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

use super::image::SLOT_BYTES;
use super::lexical::wand::{sum_bound, Bound};
use super::lists::BlockBounds;
use super::postings::{get_varint, unpack, unpack_values, Postings, CHECKPOINT_BLOCK};
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
    /// Offsets of the packed gaps and frequencies.
    data_at: usize,
    frequency_at: usize,
    /// Offset of the next block's header.
    end_at: usize,
    /// Postings in the blocks after this one.
    left_after: usize,
    /// The block's largest frequency and least word count.
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
            frequency_at: at + gap_len,
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
    decoded: bool,
    frequencies_ready: bool,
    position: usize,
    ended: bool,
    /// Blocks this cursor decoded since it was opened.
    decodes: usize,
    /// The index of the current block in the list.
    block: usize,
    /// The list's postings.
    count: usize,
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
            decoded: false,
            frequencies_ready: false,
            position: 0,
            ended: true,
            decodes: 0,
            block: 0,
            count: 0,
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
        self.decoded = false;
        self.position = 0;
        self.block = 0;
        self.count = list.count as usize;
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

    fn decode<const FREQUENCIES: bool>(&mut self) {
        let header = self.current;
        let (mut gaps, mut frequencies) = ([0u64; CHECKPOINT_BLOCK], [0u64; CHECKPOINT_BLOCK]);
        // The decoders read whole 8-byte windows while they stay inside the slice they are
        // given, so each gets the rest of the list and uses only its own values.
        unpack_values(
            &self.bytes[header.data_at..],
            header.count,
            header.gap_bits,
            &mut gaps,
        );
        if FREQUENCIES {
            unpack_values(
                &self.bytes[header.frequency_at..],
                header.count,
                header.frequency_bits,
                &mut frequencies,
            );
        }
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
            if FREQUENCIES {
                *out_frequency = frequency as u32;
            }
        }
        self.decodes += usize::from(!self.decoded);
        self.decoded = true;
        self.frequencies_ready = FREQUENCIES;
    }

    /// The frequency of the current posting.
    fn frequency(&self) -> u32 {
        if self.decoded && self.frequencies_ready {
            return self.frequencies[self.position];
        }
        unpack(
            &self.bytes[self.current.frequency_at..],
            self.position,
            self.current.frequency_bits,
        ) as u32
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
        self.block += 1;
        true
    }

    /// Moves to the first posting at or after `target`, finding its block from the list's
    /// skip table in `blocks` when there is one rather than reading each header between.
    fn seek_in(&mut self, target: u32, blocks: Option<&BlockBounds<'_>>) {
        if self.ended || self.doc() >= target {
            return;
        }
        if self.current.last < target {
            if let Some(found) = blocks.and_then(|blocks| blocks.find(self.block + 1, target)) {
                let Some((block, at, previous)) = found else {
                    self.ended = true;
                    return;
                };
                self.current = Header::parse(
                    self.bytes,
                    at,
                    self.count - block * CHECKPOINT_BLOCK,
                    previous,
                );
                self.block = block;
                self.decoded = false;
                self.position = 0;
            }
        }
        self.seek(target);
    }

    /// Moves to the first posting at or after `target`.
    fn seek(&mut self, target: u32) {
        if self.ended || self.doc() >= target {
            return;
        }
        if self.current.last < target {
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
            self.decode::<false>();
        }
        let from = self.position;
        self.position =
            from + self.slots[from..self.current.count].partition_point(|&s| s < target);
    }
}

/// Asks the CPU to load the cache line holding `table[at]`; a hint, with no effect on results.
#[inline(always)]
pub(super) fn prefetch<T>(table: &[T], at: usize) {
    #[cfg(target_arch = "x86_64")]
    if at < table.len() {
        // SAFETY: `at` is inside `table`, and a prefetch never faults or writes.
        unsafe {
            std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
                table.as_ptr().add(at).cast(),
            )
        };
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = (table, at);
}

/// Slot tables at least this large are prefetched (smaller ones stay in cache).
const PREFETCH_BYTES: usize = 1 << 20;

/// Slots per accumulation window.
const WINDOW: u32 = 4096;
/// The window of a query that wants few chunks: fewer windows mean less per-window work, and the
/// accumulator it touches stays sparse. Offsets stay below `u16::MAX`.
const WIDE_WINDOW: u32 = 8192;
/// The window of a two-term query of few chunks, whose postings are the sparsest per slot.
const PAIR_WINDOW: u32 = 16384;
/// The largest `limit` ranked in [`WIDE_WINDOW`] or [`PAIR_WINDOW`] windows.
const WIDE_LIMIT: usize = 10;
/// The most terms a query ranks through windows; more fall back to the exhaustive ranking.
const MAX_TERMS: usize = 64;
/// Bytes of direct frequency tables one checkpoint may hold (see [`DenseTables`]).
const DENSE_BYTES: usize = 32 << 20;
/// A checkpoint may also hold this many tables, however many bytes they take: the budget grows
/// with the slot count (a table is `slots / 2` bytes), so the lists a large index queries most
/// keep their tables instead of the first few lists to be queried taking the whole budget.
const DENSE_TABLES: usize = 128;
/// A list is tabled when it holds at least this share (1/32) of the slots and this many postings.
const DENSE_MIN_POSTINGS: usize = if cfg!(test) { 8 } else { 512 };
/// Distinct word counts whose length normalization is cached.
const NORM_CACHE: usize = 8192;
/// Tabled quiet terms whose frequencies bound a chunk in the first pass (the largest ceilings).
const FIRST_PASS_TABLES: usize = 4;
/// Postings of the rarest term weighed to find a score the top chunks are sure to reach.
const PILOT_POSTINGS: usize = 512;

/// Word-count classes: bounds over a class use its least word count.
const CLASSES: usize = 64;

/// The least word count of each class and the class of each word count below [`NORM_CACHE`].
fn classes() -> &'static ([u32; CLASSES], Vec<u8>) {
    static TABLE: std::sync::OnceLock<([u32; CLASSES], Vec<u8>)> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut edges = [0u32; CLASSES];
        for class in 1..CLASSES {
            let previous = edges[class - 1];
            edges[class] = if class < 16 {
                class as u32
            } else {
                (previous + 1).max((f64::from(previous) * 1.13).ceil() as u32)
            };
        }
        let of = (0..NORM_CACHE as u32)
            .map(|words| edges.iter().rposition(|&edge| edge <= words).unwrap_or(0) as u8)
            .collect();
        (edges, of)
    })
}

/// Whether a list of `postings` over `slots` slots is long enough for a direct table.
pub(super) fn tabled(postings: usize, slots: usize) -> bool {
    postings >= DENSE_MIN_POSTINGS && postings * 32 >= slots
}

/// Per-slot frequencies of one long posting list, a nibble each: 0 absent, 15 fifteen or more
/// (read the list), else the frequency. A query that must know whether a few chunks hold a
/// frequent term reads one nibble per chunk instead of seeking the list.
pub(super) struct Dense {
    nibbles: Vec<u8>,
    /// Bounds of the weight of a posting by frequency and word-count class, for the scoring
    /// constants they were computed under.
    bounds: std::sync::OnceLock<([u64; 5], Box<[f64]>)>,
}

impl Dense {
    /// The nibble of `slot`: 0 when the list lacks it, 15 when its frequency is fifteen or more.
    #[inline]
    pub(super) fn get(&self, slot: usize) -> u8 {
        (self.nibbles[slot >> 1] >> ((slot & 1) * 4)) & 15
    }

    /// The nibbles, two slots a byte, the even slot in the low half.
    pub(super) fn nibble_bytes(&self) -> &[u8] {
        &self.nibbles
    }

    /// Starts loading the nibble of `slot`; a hint, with no effect on results.
    #[inline(always)]
    pub(super) fn prefetch(&self, slot: usize) {
        prefetch(&self.nibbles, slot >> 1);
    }
}

/// Direct frequency tables of a checkpoint's longest lists, built on first use and bounded by
/// [`DENSE_BYTES`] and [`DENSE_TABLES`].
#[derive(Default)]
pub(super) struct DenseTables {
    tables: std::sync::Mutex<Vec<(usize, std::sync::Arc<Dense>)>>,
    bytes: std::sync::atomic::AtomicUsize,
}

impl DenseTables {
    /// The table of `list`, building it when the budget allows; `None` when it does not.
    pub(super) fn get(&self, slots: usize, list: &Postings<'_>) -> Option<std::sync::Arc<Dense>> {
        use std::sync::atomic::Ordering::Relaxed;
        let key = list.bytes.as_ptr() as usize;
        let lookup = |tables: &[(usize, std::sync::Arc<Dense>)]| {
            tables
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, table)| std::sync::Arc::clone(table))
        };
        if let Some(found) = lookup(self.tables.lock().ok()?.as_slice()) {
            return Some(found);
        }
        let size = slots.div_ceil(2);
        let budget = DENSE_BYTES.max(DENSE_TABLES * size);
        if self.bytes.load(Relaxed).saturating_add(size) > budget {
            return None;
        }
        let mut nibbles = vec![0u8; size];
        list.for_each(|slot, frequency| {
            nibbles[slot >> 1] |= (frequency.min(15) as u8) << ((slot & 1) * 4);
        });
        let table = std::sync::Arc::new(Dense {
            nibbles,
            bounds: std::sync::OnceLock::new(),
        });
        let mut tables = self.tables.lock().ok()?;
        if let Some(found) = lookup(tables.as_slice()) {
            return Some(found);
        }
        if self.bytes.fetch_add(size, Relaxed).saturating_add(size) > budget {
            self.bytes.fetch_sub(size, Relaxed);
            return None;
        }
        tables.push((key, std::sync::Arc::clone(&table)));
        Some(table)
    }
}

/// One query term: its cursor and what scoring it needs.
struct Term {
    idf: f64,
    bound: Bound,
    /// A bound of every weight of the list.
    ceiling: f64,
    /// Whether the list is long enough for a direct table, and the table once it is built.
    tabled: bool,
    dense: Option<std::sync::Arc<Dense>>,
    dense_tried: bool,
    /// Bound of the weight of a chunk by the table's frequency and word-count class.
    ub: Vec<f64>,
}

#[derive(Clone, Copy)]
struct WindowCandidate {
    offset: u32,
    /// Index of the exact weight in the flat list when only one term accumulated.
    single_index: u32,
    upper: f64,
}

/// Working memory of [`rank`], reused across queries.
pub(super) struct PruneScratch {
    terms: Vec<Term>,
    order: Vec<usize>,
    prefix: Vec<f64>,
    /// Chunks scoring at least the floor when read, as `(score, slot)`.
    pub(super) candidates: Vec<(f64, u32)>,
    best: BinaryHeap<Reverse<u64>>,
    /// What the last query read.
    pub(super) stats: PruneStats,
    /// Window sum of the essential weights per offset, zero between windows.
    acc: Vec<f64>,
    /// Offsets with a posting weighed in the window, a bit each.
    mask: [u64; PAIR_WINDOW as usize / 64],
    pilot: Vec<f64>,
    /// Per essential term of the window: its postings' offsets and weights, in slot order.
    logs: Vec<Vec<u16>>,
    /// Per essential term and window offset: the weight of its posting there, else 0.
    wbuf: Vec<f64>,
    lookups: Vec<usize>,
    /// First-pass survivors of a window.
    cand: Vec<WindowCandidate>,
    /// The postings of a window's one accumulated term: offset and weight, in slot order.
    single: Vec<(u32, f64)>,
    weights: [f64; MAX_TERMS],
    /// Length normalization by word count under `norm_key`.
    norms: Vec<f64>,
    norm_key: (u64, u64, u64, u64, u64),
}

impl Default for PruneScratch {
    fn default() -> Self {
        Self {
            terms: Vec::new(),
            order: Vec::new(),
            prefix: Vec::new(),
            candidates: Vec::new(),
            best: BinaryHeap::new(),
            stats: PruneStats::default(),
            acc: vec![0.0; PAIR_WINDOW as usize],
            mask: [0; PAIR_WINDOW as usize / 64],
            pilot: Vec::new(),
            logs: Vec::new(),
            wbuf: Vec::new(),
            lookups: Vec::new(),
            cand: Vec::new(),
            single: Vec::new(),
            weights: [0.0; MAX_TERMS],
            norms: vec![0.0; NORM_CACHE],
            norm_key: (0, 0, 0, 0, 0),
        }
    }
}

/// Work counters of one [`rank`] call, for measuring how much pruning skips.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct PruneStats {
    pub(super) listed: usize,
    pub(super) blocks_listed: usize,
    pub(super) blocks_decoded: usize,
    pub(super) scored: usize,
    pub(super) windows: usize,
    /// Windows no chunk of which could reach the threshold.
    pub(super) skipped_windows: usize,
    /// Essential blocks passed over undecoded: their bound with the other terms' window
    /// bounds stays below the threshold.
    pub(super) skipped_blocks: usize,
    pub(super) accumulated: usize,
    pub(super) survivors: usize,
    pub(super) table_lookups: usize,
    pub(super) cursor_lookups: usize,
}

impl<'a> ImpactCursor<'a> {
    /// Calls `f(slot, frequency)` for the first `count` postings or more (whole blocks).
    fn drain_first(&mut self, count: usize, mut f: impl FnMut(u32, u32)) {
        let mut taken = 0usize;
        while !self.ended && taken < count {
            if !self.decoded || !self.frequencies_ready {
                self.decode::<true>();
            }
            for at in self.position..self.current.count {
                f(self.slots[at], self.frequencies[at]);
            }
            taken += self.current.count - self.position;
            if !self.next_block() {
                self.ended = true;
            }
        }
    }

    /// Calls `f(slots, frequencies, max_frequency, min_words)` for the runs of postings below
    /// `hi` the cursor has not passed, a block at a time (with its block's largest frequency
    /// and least word count), and leaves it at the first posting at or after `hi`. A block
    /// that ends below `hi` and for whose index `pass` holds is passed over undecoded; the
    /// count of those is returned.
    fn drain_below(
        &mut self,
        hi: u32,
        mut pass: impl FnMut(usize) -> bool,
        mut f: impl FnMut(&[u32], &[u32], u32, u32),
    ) -> usize {
        let mut passed = 0usize;
        loop {
            if self.ended || self.doc() >= hi {
                return passed;
            }
            if self.current.last < hi && pass(self.block) {
                passed += 1;
                if !self.next_block() {
                    self.ended = true;
                    return passed;
                }
                continue;
            }
            if !self.decoded || !self.frequencies_ready {
                self.decode::<true>();
            }
            let end = self.current.count;
            let at = self.position;
            // A block that ends below `hi` is drained whole.
            let stop = if self.current.last < hi {
                end
            } else {
                at + self.slots[at..end].partition_point(|&slot| slot < hi)
            };
            f(
                &self.slots[at..stop],
                &self.frequencies[at..stop],
                self.current.max_frequency,
                self.current.min_words,
            );
            self.position = stop;
            if stop < end {
                return passed;
            }
            if !self.next_block() {
                self.ended = true;
                return passed;
            }
        }
    }
}

/// What weighing a posting needs, shared by the window loop and its helpers.
struct Scorer<'a> {
    bm25: Bm25,
    average: f64,
    simple: bool,
    word_counts: &'a [u16],
    slot_table: &'a [u8],
}

impl Scorer<'_> {
    /// Starts loading the word counts of `slots` (sorted), one prefetch per 64-byte line of the
    /// word-count table, when the table is too large to stay in cache; a hint, with no effect on
    /// results.
    #[inline(always)]
    fn prefetch_words(&self, slots: impl Iterator<Item = u32>) {
        if self.word_counts.len() * 2 >= PREFETCH_BYTES {
            let mut line = usize::MAX;
            for slot in slots {
                if slot as usize >> 5 != line {
                    line = slot as usize >> 5;
                    prefetch(self.word_counts, slot as usize);
                }
            }
        }
    }

    #[inline(always)]
    fn weigh(&self, idf: f64, tf: f64, norm: f64) -> f64 {
        if self.simple {
            idf * tf * self.bm25.k1_plus_one / (tf + self.bm25.k1 * norm)
        } else {
            self.bm25.weight(idf, tf, norm)
        }
    }

    /// Length normalization of `slot`'s chunk, cached by word count in `norms`.
    #[inline(always)]
    fn norm(&self, norms: &mut [f64], slot: u32) -> f64 {
        let words = match self.word_counts[slot as usize] {
            u16::MAX => {
                let at = slot as usize * SLOT_BYTES;
                u32::from_le_bytes(self.slot_table[at..at + 4].try_into().expect("four bytes"))
            }
            short => u32::from(short),
        };
        match norms.get_mut(words as usize) {
            Some(cached) => {
                if *cached == 0.0 {
                    *cached = self.bm25.norm(f64::from(words), self.average);
                }
                *cached
            }
            None => self.bm25.norm(f64::from(words), self.average),
        }
    }
}

/// One essential term's postings of one block run, weighed into the window: the accumulator,
/// the touched mask, the term's weight buffer and its log of touched offsets.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn accumulate_run(
    scorer: &Scorer<'_>,
    norms: &mut [f64],
    idf: f64,
    lo: u32,
    slots: &[u32],
    frequencies: &[u32],
    acc: &mut [f64],
    mask: &mut [u64],
    wbuf: &mut [f64],
    log: &mut Vec<u16>,
) {
    scorer.prefetch_words(slots.iter().copied());
    for (&slot, &frequency) in slots.iter().zip(frequencies) {
        let offset = (slot - lo) as usize;
        let norm = scorer.norm(norms, slot);
        let weight = scorer.weigh(idf, f64::from(frequency), norm);
        if weight > 0.0 {
            mask[offset >> 6] |= 1u64 << (offset & 63);
            acc[offset] += weight;
            wbuf[offset] = weight;
            log.push(offset as u16);
        }
    }
}

/// The postings of one block run of a window's only accumulated term, weighed: their offsets
/// and weights, in slot order, those of weight zero or of a frequency below `least` left out.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn collect_run(
    scorer: &Scorer<'_>,
    norms: &mut [f64],
    idf: f64,
    lo: u32,
    slots: &[u32],
    frequencies: &[u32],
    least: u32,
    out: &mut Vec<(u32, f64)>,
) {
    if least > 1 {
        // Only the postings at `least` or more are weighed: find them, load their word counts,
        // then weigh them.
        let mut kept = [0u16; CHECKPOINT_BLOCK];
        let mut n = 0usize;
        for (at, &frequency) in frequencies.iter().enumerate() {
            kept[n] = at as u16;
            n += usize::from(frequency >= least);
        }
        let kept = &kept[..n];
        scorer.prefetch_words(kept.iter().map(|&at| slots[usize::from(at)]));
        for &at in kept {
            let (slot, frequency) = (slots[usize::from(at)], frequencies[usize::from(at)]);
            let norm = scorer.norm(norms, slot);
            let weight = scorer.weigh(idf, f64::from(frequency), norm);
            if weight > 0.0 {
                out.push((slot - lo, weight));
            }
        }
        return;
    }
    scorer.prefetch_words(slots.iter().copied());
    for (&slot, &frequency) in slots.iter().zip(frequencies) {
        let norm = scorer.norm(norms, slot);
        let weight = scorer.weigh(idf, f64::from(frequency), norm);
        if weight > 0.0 {
            out.push((slot - lo, weight));
        }
    }
}

/// The query's top `limit` chunks as [`super::search::LexicalScratch::rank`] selects them, left
/// in `scratch.candidates` unordered: every chunk scoring at least the `limit`-th best score,
/// ties included. `None` when the checkpoint or the weights admit no proven bound, and the
/// caller ranks exhaustively.
///
/// The slot space is walked in windows. A window ends where the first essential term's block
/// does, so each essential term has one block in it, whose bound (with the quiet terms'
/// ceilings) decides whether the window can reach the threshold at all. The essential terms'
/// postings in the window are weighed into a dense accumulator; a chunk whose accumulated
/// weight plus the quiet terms' ceilings falls short of the threshold is dropped, the others
/// are scored exactly (quiet terms looked up by table or cursor, weights summed in query
/// term order).
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
    let slots = image.slot_total();
    scratch.candidates.clear();
    scratch.best.clear();
    scratch.terms.clear();
    scratch.stats = PruneStats::default();
    let mut lists = Vec::new();
    let mut windows = Vec::new();
    for (form, set) in terms.iter().enumerate() {
        for word in set {
            let Some((list, skips)) = image.postings_with_skips(form, word) else {
                continue;
            };
            let idf = Bm25::idf(n, f64::from(list.count));
            let bound = Bound::new(idf, average, bm25)?;
            lists.push((list, idf, bound));
            windows.push(skips.map(|skips| {
                let table = image.rank_table_for(skips.skips, bm25, average);
                BlockBounds::new(skips, table)
            }));
        }
    }
    if limit == 0 || lists.is_empty() {
        return Some(());
    }
    if lists.len() > MAX_TERMS || slots >= u32::MAX as usize - PAIR_WINDOW as usize {
        return None;
    }
    let count = lists.len();
    let mut cursors: Vec<ImpactCursor<'_>> = Vec::with_capacity(count);
    for (list, idf, bound) in &lists {
        let mut cursor = ImpactCursor::new();
        cursor.open(list);
        let postings = list.count as usize;
        scratch.terms.push(Term {
            idf: *idf,
            bound: *bound,
            ceiling: bound.at(cursor.list_max_frequency, cursor.list_min_words),
            tabled: tabled(postings, slots),
            dense: None,
            dense_tried: false,
            ub: Vec::new(),
        });
        cursors.push(cursor);
    }
    scratch.stats.listed = lists.iter().map(|(list, ..)| list.count as usize).sum();
    scratch.stats.blocks_listed = lists
        .iter()
        .map(|(list, ..)| (list.count as usize).div_ceil(CHECKPOINT_BLOCK))
        .sum();
    // Length normalization by word count: the very expression the exhaustive ranking uses.
    let norm_key = (
        bm25.k1.to_bits(),
        bm25.b.to_bits(),
        bm25.one_minus_b.to_bits(),
        average.to_bits(),
        image.slot_total() as u64,
    );
    if scratch.norm_key != norm_key {
        scratch.norm_key = norm_key;
        scratch.norms.fill(0.0);
    }
    let PruneScratch {
        terms,
        order: by_bound,
        prefix,
        candidates,
        best,
        stats,
        acc,
        mask,
        pilot,
        logs,
        wbuf,
        lookups,
        cand,
        single,
        weights,
        norms,
        ..
    } = scratch;
    let slot_table = image.slot_table();
    let word_counts = image.word_counts();
    let classes8 = image.word_classes(&classes().1);
    by_bound.clear();
    by_bound.extend(0..count);
    by_bound.sort_by(|&a, &b| terms[a].ceiling.total_cmp(&terms[b].ceiling));
    prefix.clear();
    prefix.push(0.0);
    for &i in by_bound.iter() {
        prefix.push(prefix[prefix.len() - 1] + terms[i].ceiling);
    }
    if logs.len() < count {
        logs.resize_with(count, Vec::new);
    }
    let window: u32 = match count {
        _ if limit > WIDE_LIMIT => WINDOW,
        2 => PAIR_WINDOW,
        3 => WIDE_WINDOW,
        _ => WINDOW,
    };
    let stride = window as usize;
    if wbuf.len() < count * stride {
        wbuf.resize(count * stride, 0.0);
    }
    let scorer = Scorer {
        bm25,
        average,
        simple: bm25.k1 <= 1.0 || bm25 == Bm25::DEFAULT,
        word_counts,
        slot_table,
    };
    let weigh = |idf: f64, tf: f64, norm: f64| -> f64 { scorer.weigh(idf, tf, norm) };
    let norm_for = |norms: &mut Vec<f64>, slot: u32| -> f64 { scorer.norm(norms, slot) };
    // A score `limit` chunks are sure to reach, from the weights of the rarest term's first
    // chunks alone (a chunk's score is at least any one of its weights).
    let mut pilot_floor = 0.0f64;
    {
        let &top = by_bound.last().expect("a term");
        let mut cursor = ImpactCursor::new();
        cursor.open(&lists[top].0);
        pilot.clear();
        let idf = terms[top].idf;
        cursor.drain_first(PILOT_POSTINGS, |slot, frequency| {
            let norm = norm_for(norms, slot);
            pilot.push(weigh(idf, f64::from(frequency), norm));
        });
        if pilot.len() >= limit {
            let (_, nth, _) = pilot.select_nth_unstable_by(limit - 1, |a, b| b.total_cmp(a));
            pilot_floor = *nth;
        }
    }
    const INFINITY: u64 = 0x7FF0_0000_0000_0000;
    let cut_factor = 1.0 - (2.0 * count as f64 + 4.0) * f64::EPSILON;
    let mut floor = 1u64;
    let mut threshold = 0.0f64;
    let mut quiet = 0usize;
    let mut scored = 0usize;
    let mut window_bounds = [0.0f64; MAX_TERMS];
    let mut demoted: Vec<usize> = Vec::with_capacity(count);
    loop {
        let effective = threshold.max(pilot_floor);
        while quiet < count && sum_bound(prefix[quiet + 1], count) < effective {
            quiet += 1;
        }
        let mut lo = NO_DOC;
        for &i in &by_bound[quiet..] {
            lo = lo.min(cursors[i].doc());
        }
        if lo == NO_DOC {
            break;
        }
        let hi = lo.saturating_add(window);
        stats.windows += 1;
        // Each term's bound in this window, from its blocks there; zero past its postings.
        for i in 0..count {
            let term = &terms[i];
            window_bounds[i] = if cursors[i].doc() >= hi {
                0.0
            } else {
                match windows[i].as_mut() {
                    Some(blocks) => blocks.bound(lo, hi, term.bound, term.ceiling),
                    None => term.ceiling,
                }
            };
        }
        // Essential terms whose window bounds, with the quiet terms', stay below the threshold
        // are demoted for the window: looked up per candidate, not accumulated.
        let mut below = 0.0f64;
        for &i in &by_bound[..quiet] {
            below += window_bounds[i];
        }
        demoted.clear();
        demoted.extend(by_bound[quiet..].iter().copied());
        demoted.sort_by(|&a, &b| window_bounds[a].total_cmp(&window_bounds[b]));
        let mut demote = 0usize;
        while demote < demoted.len()
            && sum_bound(below + window_bounds[demoted[demote]], count) < effective
        {
            below += window_bounds[demoted[demote]];
            demote += 1;
        }
        demoted.truncate(demote);
        if demote == count - quiet {
            // No chunk of the window can reach the threshold.
            stats.skipped_windows += 1;
            for &i in &by_bound[quiet..] {
                cursors[i].seek_in(hi, windows[i].as_ref());
            }
            continue;
        }
        // The quiet terms: those with a table, and the sum of the ceilings of those without.
        lookups.clear();
        let mut untabled = 0.0f64;
        let mut first_pass = [0usize; FIRST_PASS_TABLES];
        let mut first_passes = 0usize;
        for j in (0..quiet).rev() {
            let i = by_bound[j];
            let term = &mut terms[i];
            if term.tabled && !term.dense_tried {
                term.dense_tried = true;
                term.dense = image.dense().get(slots, &lists[i].0);
                if let Some(dense) = term.dense.as_ref() {
                    let key = [
                        bm25.k1.to_bits(),
                        bm25.b.to_bits(),
                        bm25.one_minus_b.to_bits(),
                        average.to_bits(),
                        term.idf.to_bits(),
                    ];
                    let compute = || {
                        let (edges, _) = classes();
                        let mut bounds = vec![0.0f64; 16 * CLASSES];
                        for found in 1..16usize {
                            for (class, &edge) in edges.iter().enumerate() {
                                bounds[found * CLASSES + class] = if found == 15 {
                                    term.ceiling
                                } else {
                                    term.bound.at(found as u32, edge).min(term.ceiling)
                                };
                            }
                        }
                        (key, bounds.into_boxed_slice())
                    };
                    let cached = dense.bounds.get_or_init(compute);
                    term.ub.clear();
                    if cached.0 == key {
                        term.ub.extend_from_slice(&cached.1);
                    } else {
                        term.ub.extend_from_slice(&compute().1);
                    }
                }
            }
            if term.dense.is_some() && first_passes < FIRST_PASS_TABLES {
                first_pass[first_passes] = i;
                first_passes += 1;
            } else {
                untabled += window_bounds[i];
            }
            lookups.push(i);
        }
        for &i in demoted.iter() {
            untabled += window_bounds[i];
            lookups.push(i);
        }
        let mut rest = [0.0f64; FIRST_PASS_TABLES + 1];
        for k in (0..first_passes).rev() {
            rest[k] = rest[k + 1] + window_bounds[first_pass[k]];
        }
        // With the heap full the cut is known before the window is read: a lone accumulated
        // term's posting whose bound (at its frequency and its block's least word count) with
        // the quiet bounds stays below `drop` is one the gather would drop, so it is not weighed.
        let early = threshold.max(pilot_floor);
        let early_drop = early * cut_factor * (1.0 - 8.0 * f64::EPSILON);
        let prefilter = best.len() >= limit && early > 0.0;
        // Accumulate the essential terms' weights over the window. A window with one term to
        // accumulate keeps its postings in a list instead: the accumulated weight of a chunk is
        // then that term's weight alone.
        let alone = count - quiet - demoted.len() == 1;
        single.clear();
        for (position, &i) in by_bound[quiet..].iter().enumerate() {
            let idf = terms[i].idf;
            let log = &mut logs[position];
            log.clear();
            if demoted.contains(&i) {
                continue;
            }
            let mut taken = 0usize;
            let wslice = &mut wbuf[position * stride..(position + 1) * stride];
            // A block whose bound with every other term's window bound is below the threshold
            // holds no chunk that can reach it: its weights are not needed.
            let mut others = 0.0f64;
            for (j, &weight) in window_bounds[..count].iter().enumerate() {
                if j != i {
                    others += weight;
                }
            }
            let term = &terms[i];
            let blocks = windows[i].as_ref();
            let lone = alone && prefilter;
            let pass = |block: usize| {
                effective > 0.0
                    && blocks.is_some_and(|blocks| {
                        sum_bound(
                            others + blocks.block(block, term.bound, term.ceiling),
                            count,
                        ) < effective
                    })
            };
            stats.skipped_blocks +=
                cursors[i].drain_below(hi, pass, |slots, frequencies, most, words| {
                    if alone {
                        let mut least = 1u32;
                        if lone {
                            while least <= most
                                && term.bound.at(least, words) + untabled + rest[0] < early_drop
                            {
                                least += 1;
                            }
                        }
                        collect_run(&scorer, norms, idf, lo, slots, frequencies, least, single);
                    } else {
                        accumulate_run(
                            &scorer,
                            norms,
                            idf,
                            lo,
                            slots,
                            frequencies,
                            acc,
                            mask,
                            wslice,
                            log,
                        );
                    }
                    taken += slots.len();
                });
            stats.accumulated += taken;
        }
        // The least score `limit` chunks of this window are sure to reach, while the heap is
        // not yet full: each chunk's accumulated weight is below its score by no more than the
        // summation error.
        let mut sure = 0.0f64;
        if best.len() < limit {
            pilot.clear();
            if alone {
                pilot.extend(single.iter().map(|&(_, weight)| weight));
            } else {
                for (word, bits) in mask[..stride / 64].iter().enumerate() {
                    let mut bits = *bits;
                    while bits != 0 {
                        let offset = (word << 6) | bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        pilot.push(acc[offset]);
                    }
                }
            }
            if pilot.len() >= limit {
                let (_, nth, _) = pilot.select_nth_unstable_by(limit - 1, |a, b| b.total_cmp(a));
                sure = *nth * (1.0 - 2.0 * count as f64 * f64::EPSILON);
            }
        }
        // First pass: a bound of every touched chunk's score from its accumulated weight, the
        // tables' frequencies and the untabled quiet terms' ceilings.
        let floor_score = threshold.max(pilot_floor).max(sure);
        let cut = floor_score * cut_factor;
        // A chunk whose bound plus the ceilings of the tables still to add is below `drop`
        // cannot reach `cut` whatever those tables say (a table's bound never exceeds its
        // list's ceiling); the margin covers the different summation order.
        let drop = cut * (1.0 - 8.0 * f64::EPSILON);
        cand.clear();
        let mut single_term = None;
        if alone {
            // The accumulated weight `0.0 + weight` is the weight itself.
            let term = by_bound[quiet..]
                .iter()
                .copied()
                .find(|i| !demoted.contains(i))
                .expect("the accumulated term");
            single_term = Some(term);
            for (index, &(offset, weight)) in single.iter().enumerate() {
                let upper = weight + untabled;
                if floor_score > 0.0 && upper + rest[0] < drop {
                    continue;
                }
                cand.push(WindowCandidate {
                    offset,
                    single_index: index as u32,
                    upper,
                });
            }
        } else {
            for (word, word_bits) in mask[..stride / 64].iter_mut().enumerate() {
                let mut bits = std::mem::take(word_bits);
                while bits != 0 {
                    let offset = (word << 6) | bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let partial = std::mem::take(&mut acc[offset]);
                    let upper = partial + untabled;
                    if floor_score > 0.0 && upper + rest[0] < drop {
                        continue;
                    }
                    cand.push(WindowCandidate {
                        offset: offset as u32,
                        single_index: 0,
                        upper,
                    });
                }
            }
        }
        // One table at a time over the window's candidates: each candidate still adds the
        // tables' bounds in the same order, but the loop body is a tight lookup. A candidate
        // that can no longer reach the cut leaves before the next table.
        let window_classes = &classes8[lo as usize..];
        for (k, &i) in first_pass[..first_passes].iter().enumerate() {
            let term = &terms[i];
            if let Some(table) = term.dense.as_ref() {
                let ub = &term.ub[..];
                let cap = window_bounds[i];
                let after = rest[k + 1];
                let mut kept = 0usize;
                for read in 0..cand.len() {
                    let mut entry = cand[read];
                    let offset = entry.offset as usize;
                    let class = usize::from(window_classes[offset]);
                    entry.upper +=
                        ub[usize::from(table.get(lo as usize + offset)) * CLASSES + class].min(cap);
                    if floor_score > 0.0 && entry.upper + after < drop {
                        continue;
                    }
                    cand[kept] = entry;
                    kept += 1;
                }
                cand.truncate(kept);
            }
        }
        if floor_score > 0.0 {
            cand.retain(|entry| entry.upper >= cut);
        }
        // Second pass: the candidates, scored exactly.
        for &WindowCandidate {
            offset,
            single_index,
            upper,
        } in cand.iter()
        {
            let floor_score = threshold.max(pilot_floor).max(sure);
            if floor_score > 0.0 && upper < floor_score * cut_factor {
                continue;
            }
            let slot = lo + offset;
            stats.survivors += 1;
            let norm = norm_for(norms, slot);
            if let Some(i) = single_term {
                weights[i] = single[single_index as usize].1;
            } else {
                for (position, &i) in by_bound[quiet..].iter().enumerate() {
                    let weight = wbuf[position * stride + offset as usize];
                    if weight != 0.0 {
                        weights[i] = weight;
                    }
                }
            }
            for &i in lookups.iter() {
                let term = &terms[i];
                let tf = match term.dense.as_ref().map(|t| t.get(slot as usize)) {
                    Some(15) | None => {
                        stats.cursor_lookups += 1;
                        cursors[i].seek_in(slot, windows[i].as_ref());
                        if cursors[i].doc() == slot {
                            cursors[i].frequency()
                        } else {
                            0
                        }
                    }
                    Some(found) => {
                        stats.table_lookups += 1;
                        u32::from(found)
                    }
                };
                if tf > 0 {
                    weights[i] = weigh(term.idf, f64::from(tf), norm);
                }
            }
            // The sum in query term order, from zero, as the exhaustive ranking adds
            // (a term the chunk lacks adds zero).
            let mut score = 0.0f64;
            for weight in weights[..count].iter_mut() {
                score += *weight;
                *weight = 0.0;
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
        for (position, log) in logs[..count - quiet].iter().enumerate() {
            for &offset in log {
                wbuf[position * stride + usize::from(offset)] = 0.0;
            }
        }
        for &i in demoted.iter() {
            cursors[i].seek_in(hi, windows[i].as_ref());
        }
    }
    stats.scored = scored;
    stats.blocks_decoded = cursors.iter().map(|c| c.decodes).sum();
    let bar = if best.len() < limit {
        0
    } else {
        best.peek().expect("full heap").0
    };
    candidates.retain(|&(score, _)| score.to_bits() >= bar);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::documents::postings::CheckpointPacker;

    #[test]
    fn lexical_impact_cursor_reads_frequencies_after_seeks_and_drains() {
        let count = CHECKPOINT_BLOCK + 9;
        let entries: Vec<_> = (0..count)
            .map(|i| (3 * i as u32 + 2, (7 * i as u32 % 509) + 1))
            .collect();
        let mut bytes = Vec::new();
        let mut packer = CheckpointPacker::with_info(true, &|_| 512, false);
        for &(slot, frequency) in &entries {
            packer.push(slot as usize, frequency, &mut bytes);
        }
        packer.finish(&mut bytes);
        let mut cursor = ImpactCursor::new();
        cursor.open(&Postings {
            bytes: &bytes,
            count: count as u32,
            blocked: true,
            impacts: true,
        });
        assert_eq!((cursor.doc(), cursor.frequency()), entries[0]);
        cursor.seek_in(entries[63].0 + 1, None);
        assert_eq!((cursor.doc(), cursor.frequency()), entries[64]);
        let mut drained = Vec::new();
        cursor.drain_below(
            entries[CHECKPOINT_BLOCK - 1].0,
            |_| false,
            |slots, frequencies, _, _| {
                drained.extend(slots.iter().copied().zip(frequencies.iter().copied()));
            },
        );
        assert_eq!(drained, entries[64..CHECKPOINT_BLOCK - 1]);
        assert_eq!(
            (cursor.doc(), cursor.frequency()),
            entries[CHECKPOINT_BLOCK - 1]
        );
        cursor.seek_in(entries[count - 2].0 + 1, None);
        assert_eq!((cursor.doc(), cursor.frequency()), entries[count - 1]);
        cursor.seek_in(entries[count - 1].0 + 1, None);
        assert_eq!(cursor.doc(), NO_DOC);
    }
}
