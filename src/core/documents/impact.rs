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

use super::image::{CHAMPIONS, SLOT_BYTES};
use super::lexical::served::champions_of;
use super::lexical::wand::{sum_bound, up, Bound};
use super::lists::{BlockBounds, Cursor, ListRef, RankTable};
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

/// Maximum unit bound per aligned window, used only by the union ranker.
pub(super) struct WindowBounds(Box<[f64]>);

impl WindowBounds {
    pub(super) fn new(list: ListRef<'_>, table: &RankTable, slots: usize) -> Option<Self> {
        let mut cursor = Cursor::open(list);
        let mut bounds = vec![0.0f64; slots.div_ceil(WINDOW as usize)];
        for (block, &weight) in table.blocks.iter().enumerate() {
            cursor.goto_block(block);
            let first = (cursor.block_first() / WINDOW) as usize;
            let last = (cursor.block_last() / WINDOW) as usize;
            for bound in bounds.get_mut(first..=last)? {
                *bound = bound.max(f64::from(weight));
            }
        }
        Some(Self(bounds.into_boxed_slice()))
    }

    #[inline]
    fn bound(&self, first: usize, last: usize, idf: f64, ceiling: f64) -> f64 {
        // The aligned windows `first..=last` cover the query window.
        let mut unit = 0.0f64;
        for &bound in self
            .0
            .get(first..=last.min(self.0.len().saturating_sub(1)))
            .unwrap_or(&[])
        {
            unit = unit.max(bound);
        }
        // Scale with the same rounding margin as RankTable::scale.
        up(unit * idf * (1.0 + 16.0 * f64::EPSILON)).min(ceiling)
    }
}

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

/// Adds one first-pass table's bound to every candidate and moves the survivors to the front,
/// returning their number. A candidate leaves when `test` is set and `upper + after < drop`;
/// `BRANCHLESS` writes every entry and counts only the survivors.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn table_pass<const BRANCHLESS: bool>(
    cand: &mut [WindowCandidate],
    table: &Dense,
    classes: &[u8],
    lo: usize,
    bounds: &[f64],
    cap: f64,
    after: f64,
    test: bool,
    drop: f64,
) -> usize {
    let mut kept = 0usize;
    for read in 0..cand.len() {
        let mut entry = cand[read];
        let offset = entry.offset as usize;
        // The absent and saturated rows hold the same bound in every class, so no frequency
        // needs a branch of its own.
        let row = usize::from(table.get(lo + offset)) * CLASSES + usize::from(classes[offset]);
        entry.upper += bounds[row].min(cap);
        let keep = !(test && entry.upper + after < drop);
        if BRANCHLESS {
            cand[kept] = entry;
            kept += usize::from(keep);
        } else if keep {
            cand[kept] = entry;
            kept += 1;
        }
    }
    kept
}

/// Stores the touched chunks `admit` keeps, in offset order. `admit` returns the chunk's upper
/// bound and whether to keep it; every chunk's entry is written and only the kept ones counted,
/// so the data-dependent keep/drop outcome costs no branch. Without `BRANCHLESS` only the kept
/// entries are written, which suits the few-term queries whose outcome is predictable.
#[inline]
fn gather_candidates<const BRANCHLESS: bool>(
    candidates: &mut Vec<WindowCandidate>,
    single: Option<&[(u32, f64)]>,
    mask: &mut [u64],
    acc: &mut [f64],
    mut admit: impl FnMut(u32, f64) -> (f64, bool),
) {
    candidates.clear();
    let touched = match single {
        Some(single) => single.len(),
        None => mask.iter().map(|word| word.count_ones() as usize).sum(),
    };
    candidates.reserve(touched);
    let spare = candidates.spare_capacity_mut();
    let mut kept = 0usize;
    if let Some(single) = single {
        for (index, &(offset, weight)) in single.iter().enumerate() {
            let (upper, keep) = admit(offset, weight);
            spare[kept].write(WindowCandidate {
                offset,
                single_index: index as u32,
                upper,
            });
            if BRANCHLESS || keep {
                kept += usize::from(keep);
            }
        }
    } else {
        for (word, word_bits) in mask.iter_mut().enumerate() {
            let mut bits = std::mem::take(word_bits);
            while bits != 0 {
                let offset = (word << 6) | bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let partial = std::mem::take(&mut acc[offset]);
                let (upper, keep) = admit(offset as u32, partial);
                spare[kept].write(WindowCandidate {
                    offset: offset as u32,
                    single_index: 0,
                    upper,
                });
                if BRANCHLESS || keep {
                    kept += usize::from(keep);
                }
            }
        }
    }
    // SAFETY: `kept` is at most the number of chunks visited, which is `touched` and within the
    // reserved capacity, and each of the first `kept` entries was written above.
    unsafe { candidates.set_len(kept) };
}

#[derive(Default)]
struct WindowFrequencies {
    cells: Vec<u64>,
    // The upper half identifies the window. The lower half holds the full posting frequency.
    stamp: u64,
}

impl WindowFrequencies {
    fn next_stamp(&mut self) -> u64 {
        const STEP: u64 = 1 << u32::BITS;
        self.stamp = self.stamp.wrapping_add(STEP);
        if self.stamp == 0 {
            self.cells.fill(0);
            self.stamp = STEP;
        }
        self.stamp
    }

    #[inline]
    fn get(&self, index: usize, stamp: u64) -> Option<u32> {
        let cell = self.cells[index];
        (cell & !u64::from(u32::MAX) == stamp).then_some(cell as u32)
    }
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
    window_frequencies: WindowFrequencies,
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
            window_frequencies: WindowFrequencies::default(),
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

enum PostingBuffer<'a> {
    Single,
    Weights(&'a mut [f64], &'a mut Vec<u16>),
    Frequencies(&'a mut [u64]),
}

/// Weighs an essential term's block run into the window and retains each contribution.
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
    mut retain: impl FnMut(usize, f64, u32),
) {
    scorer.prefetch_words(slots.iter().copied());
    for (&slot, &frequency) in slots.iter().zip(frequencies) {
        let offset = (slot - lo) as usize;
        let norm = scorer.norm(norms, slot);
        let weight = scorer.weigh(idf, f64::from(frequency), norm);
        if weight > 0.0 {
            mask[offset >> 6] |= 1u64 << (offset & 63);
            acc[offset] += weight;
            retain(offset, weight, frequency);
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

/// Windows ranked first.
const SEED_WINDOWS: usize = if cfg!(test) { 1 } else { 4 };
/// Fewest terms of a union whose ranking starts from its strongest windows.
const SEED_TERMS: usize = 4;
/// Fewest windows of a collection worth seeding.
const SEED_MIN_WINDOWS: usize = if cfg!(test) { 2 } else { 64 };
/// Calls of [`seed_ranges`] that returned more than one range.
#[cfg(test)]
pub(super) static SEEDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The slot ranges a union is ranked in, in order. A union whose threshold would climb slowly
/// through the slot space first ranks the few windows its terms' bounds put highest: the heap
/// fills with strong chunks before the rest is read, so the other windows demote or skip more
/// terms. The ranges are disjoint and cover every slot, so each chunk is scored once and the
/// result does not depend on their order.
fn seed_ranges(
    terms: &[Term],
    aligned: &[Option<&WindowBounds>],
    windows: &[Option<BlockBounds<'_>>],
    slots: usize,
    limit: usize,
) -> Option<Vec<(u32, u32)>> {
    let total = slots.div_ceil(WINDOW as usize);
    if terms.len() < SEED_TERMS
        || limit <= WIDE_LIMIT
        || total < SEED_MIN_WINDOWS
        || total <= SEED_WINDOWS
    {
        return None;
    }
    let mut sums = vec![0.0f64; total];
    for (i, term) in terms.iter().enumerate() {
        if let Some(bounds) = aligned[i] {
            for (window, sum) in sums.iter_mut().enumerate() {
                *sum += bounds.bound(window, window, term.idf, term.ceiling);
            }
        } else if let Some(blocks) = windows[i].as_ref() {
            blocks.add_window_bounds(WINDOW, term.bound, term.ceiling, &mut sums);
        }
    }
    let mut order: Vec<usize> = (0..total).collect();
    order.select_nth_unstable_by(SEED_WINDOWS, |&a, &b| sums[b].total_cmp(&sums[a]));
    order.truncate(SEED_WINDOWS);
    order.sort_unstable();
    let mut strong: Vec<(u32, u32)> = Vec::new();
    for window in order {
        let start = window as u32 * WINDOW;
        match strong.last_mut() {
            Some(last) if last.1 == start => last.1 = start + WINDOW,
            _ => strong.push((start, start + WINDOW)),
        }
    }
    let mut ranges = strong.clone();
    let mut from = 0u32;
    for &(start, end) in &strong {
        if start > from {
            ranges.push((from, start));
        }
        from = end;
    }
    ranges.push((from, u32::MAX));
    #[cfg(test)]
    SEEDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Some(ranges)
}

/// Opens every cursor afresh at the first posting at or after `from`, adding the blocks the
/// old ones decoded to `decodes`.
fn restart<'a>(
    cursors: &mut [ImpactCursor<'a>],
    lists: &[(Postings<'a>, f64, Bound)],
    windows: &mut [Option<BlockBounds<'_>>],
    from: u32,
    decodes: &mut usize,
) {
    for ((cursor, (list, ..)), blocks) in cursors.iter_mut().zip(lists).zip(windows) {
        *decodes += cursor.decodes;
        let mut fresh = ImpactCursor::new();
        fresh.open(list);
        fresh.seek_in(from, blocks.as_ref());
        *cursor = fresh;
        if let Some(blocks) = blocks.as_mut() {
            blocks.rewind();
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
    let mut spans = Vec::new();
    let mut windows = Vec::new();
    let mut aligned = [None; MAX_TERMS];
    for (form, set) in terms.iter().enumerate() {
        for word in set {
            let Some((list, skips)) = image.postings_with_skips(form, word) else {
                continue;
            };
            let idf = Bm25::idf(n, f64::from(list.count));
            let bound = Bound::new(idf, average, bm25)?;
            lists.push((list, idf, bound));
            spans.push(skips);
            let next_aligned = aligned.get_mut(windows.len());
            windows.push(skips.map(|skips| {
                let table = image.rank_table_for(skips.skips, bm25, average);
                if let (Some(entry), Some(_)) = (next_aligned, table) {
                    *entry = image.union_window_bounds(skips.skips);
                }
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
    // The ranked-union shape whose keep/drop outcomes are data dependent and mispredicted.
    let branchless = count >= 4 && limit <= WIDE_LIMIT;
    if count == 1 {
        aligned[0] = None;
    }
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
        window_frequencies,
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
    let window: u32 = match count {
        _ if limit > WIDE_LIMIT => WINDOW,
        2 => PAIR_WINDOW,
        _ => WIDE_WINDOW,
    };
    let stride = window as usize;
    let retain_frequencies = count >= 4 && limit <= WIDE_LIMIT;
    if retain_frequencies {
        if window_frequencies.cells.len() < count * stride {
            window_frequencies.cells.resize(count * stride, 0);
        }
    } else {
        if logs.len() < count {
            logs.resize_with(count, Vec::new);
        }
        if wbuf.len() < count * stride {
            wbuf.resize(count * stride, 0.0);
        }
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
    // A long list's `limit`-th best weight is a score `limit` chunks are sure to reach: a chunk's
    // score is at least any one of its weights.
    if limit <= CHAMPIONS && bm25 == Bm25::DEFAULT {
        for (span, (_, idf, _)) in spans.iter().zip(&lists) {
            let Some(list) = *span else { continue };
            if let Ok(Some(found)) = champions_of(image, list, *idf, average) {
                if let Some(&(weight, _)) = found.hits.get(limit - 1) {
                    pilot_floor = pilot_floor.max(weight);
                }
            }
        }
    }
    const INFINITY: u64 = 0x7FF0_0000_0000_0000;
    let cut_factor = 1.0 - (2.0 * count as f64 + 4.0) * f64::EPSILON;
    let mut floor = 1u64;
    let mut threshold = 0.0f64;
    let mut quiet = 0usize;
    let mut scored = 0usize;
    let mut window_bounds = [0.0f64; MAX_TERMS];
    // Whether a lookup term's window bound is part of a candidate's `upper`.
    let mut deferred = [false; MAX_TERMS];
    let mut demoted: Vec<usize> = Vec::with_capacity(count);
    let seeded = seed_ranges(
        &terms[..count],
        &aligned[..count],
        windows.as_slice(),
        slots,
        limit,
    );
    let ranges: &[(u32, u32)] = seeded.as_deref().unwrap_or(&[(0, u32::MAX)]);
    let mut next_range = 0usize;
    let mut range_end = ranges[0].1;
    let mut decodes = 0usize;
    if ranges[0].0 > 0 {
        restart(
            &mut cursors,
            &lists,
            &mut windows,
            ranges[0].0,
            &mut decodes,
        );
    }
    loop {
        let effective = threshold.max(pilot_floor);
        while quiet < count && sum_bound(prefix[quiet + 1], count) < effective {
            quiet += 1;
        }
        let mut lo = NO_DOC;
        for &i in &by_bound[quiet..] {
            lo = lo.min(cursors[i].doc());
        }
        if lo == NO_DOC || lo >= range_end {
            next_range += 1;
            let Some(&(start, end)) = ranges.get(next_range) else {
                break;
            };
            range_end = end;
            restart(&mut cursors, &lists, &mut windows, start, &mut decodes);
            continue;
        }
        let hi = lo.saturating_add(window).min(range_end);
        let first = (lo / WINDOW) as usize;
        let last = ((hi - 1) / WINDOW) as usize;
        stats.windows += 1;
        // Each term's bound in this window, from its blocks there; zero past its postings.
        for i in 0..count {
            let term = &terms[i];
            window_bounds[i] = if cursors[i].doc() >= hi {
                0.0
            } else if let Some(bounds) = aligned[i] {
                bounds.bound(first, last, term.idf, term.ceiling)
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
                deferred[i] = false;
            } else {
                untabled += window_bounds[i];
                deferred[i] = true;
            }
            lookups.push(i);
        }
        for &i in demoted.iter() {
            untabled += window_bounds[i];
            deferred[i] = true;
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
        let stamp = if retain_frequencies && !alone {
            window_frequencies.next_stamp()
        } else {
            0
        };
        single.clear();
        for (position, &i) in by_bound[quiet..].iter().enumerate() {
            let idf = terms[i].idf;
            let log = (!retain_frequencies).then(|| {
                let log = &mut logs[position];
                log.clear();
                log
            });
            if demoted.contains(&i) {
                continue;
            }
            let mut taken = 0usize;
            let span = position * stride..(position + 1) * stride;
            let mut buffer = if alone {
                PostingBuffer::Single
            } else if let Some(log) = log {
                PostingBuffer::Weights(&mut wbuf[span], log)
            } else {
                PostingBuffer::Frequencies(&mut window_frequencies.cells[span])
            };
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
                    match &mut buffer {
                        PostingBuffer::Single => {
                            let mut least = 1u32;
                            if lone {
                                while least <= most
                                    && term.bound.at(least, words) + untabled + rest[0] < early_drop
                                {
                                    least += 1;
                                }
                            }
                            collect_run(&scorer, norms, idf, lo, slots, frequencies, least, single);
                        }
                        PostingBuffer::Weights(saved, log) => accumulate_run(
                            &scorer,
                            norms,
                            idf,
                            lo,
                            slots,
                            frequencies,
                            acc,
                            mask,
                            |offset, weight, _| {
                                saved[offset] = weight;
                                log.push(offset as u16);
                            },
                        ),
                        PostingBuffer::Frequencies(saved) => accumulate_run(
                            &scorer,
                            norms,
                            idf,
                            lo,
                            slots,
                            frequencies,
                            acc,
                            mask,
                            |offset, _, frequency| saved[offset] = stamp | u64::from(frequency),
                        ),
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
        let single_term = alone.then(|| {
            // The accumulated weight `0.0 + weight` is the weight itself.
            by_bound[quiet..]
                .iter()
                .copied()
                .find(|i| !demoted.contains(i))
                .expect("the accumulated term")
        });
        let single_entries = single_term.map(|_| single.as_slice());
        let window_classes = &classes8[lo as usize..];
        let gather_upper = |weight: f64| {
            let upper = weight + untabled;
            (upper, !(floor_score > 0.0 && upper + rest[0] < drop))
        };
        let gather = if branchless {
            gather_candidates::<true>
        } else {
            gather_candidates::<false>
        };
        gather(
            cand,
            single_entries,
            &mut mask[..stride / 64],
            acc,
            |_, weight| gather_upper(weight),
        );
        // One table at a time over the window's candidates: each candidate still adds the
        // tables' bounds in the same order, but the loop body is a tight lookup. A candidate
        // that can no longer reach the cut leaves before the next table.
        for (k, &i) in first_pass[..first_passes].iter().enumerate() {
            let term = &terms[i];
            if let Some(table) = term.dense.as_ref() {
                let ub = &term.ub[..];
                let cap = window_bounds[i];
                let after = rest[k + 1];
                let kept = if branchless {
                    table_pass::<true>(
                        cand,
                        table,
                        window_classes,
                        lo as usize,
                        ub,
                        cap,
                        after,
                        floor_score > 0.0,
                        drop,
                    )
                } else {
                    table_pass::<false>(
                        cand,
                        table,
                        window_classes,
                        lo as usize,
                        ub,
                        cap,
                        after,
                        floor_score > 0.0,
                        drop,
                    )
                };
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
            } else if stamp != 0 {
                for (position, &i) in by_bound[quiet..].iter().enumerate() {
                    if let Some(frequency) =
                        window_frequencies.get(position * stride + offset as usize, stamp)
                    {
                        weights[i] = weigh(terms[i].idf, f64::from(frequency), norm);
                    }
                }
            } else {
                for (position, &i) in by_bound[quiet..].iter().enumerate() {
                    let weight = wbuf[position * stride + offset as usize];
                    if weight != 0.0 {
                        weights[i] = weight;
                    }
                }
            }
            // `upper` assumes the window bound of every deferred lookup term; resolving one
            // replaces its bound by its weight, and the chunk leaves once that stays below the
            // cut (the margin covers the incremental sums' rounding).
            let mut upper_now = upper;
            let early_cut = floor_score * cut_factor * (1.0 - 64.0 * f64::EPSILON);
            let mut gone = false;
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
                if deferred[i] {
                    upper_now += weights[i] - window_bounds[i];
                    if floor_score > 0.0 && upper_now < early_cut {
                        gone = true;
                        break;
                    }
                }
            }
            if gone {
                weights[..count].fill(0.0);
                continue;
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
        if !retain_frequencies {
            for (position, log) in logs[..count - quiet].iter().enumerate() {
                for &offset in log {
                    wbuf[position * stride + usize::from(offset)] = 0.0;
                }
            }
        }
        for &i in demoted.iter() {
            cursors[i].seek_in(hi, windows[i].as_ref());
        }
    }
    stats.scored = scored;
    stats.blocks_decoded = decodes + cursors.iter().map(|c| c.decodes).sum::<usize>();
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
    fn lexical_table_pass_branchless_keeps_what_branching_keeps_on_constant_rows() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || crate::core::documents::xorshift64(&mut state);
        let ceiling = 12.0;
        let mut bounds = [0.0f64; CLASSES * 16];
        for frequency in 1..15 {
            for class in 0..CLASSES {
                bounds[frequency * CLASSES + class] =
                    frequency as f64 * 0.7 + class as f64 / CLASSES as f64;
            }
        }
        bounds[15 * CLASSES..].fill(ceiling);
        let slots = 512;
        // A third of the slots absent (0) and a third saturated (15): the constant rows.
        let nibble = |r: u64| match r % 3 {
            0 => 0u8,
            1 => 15,
            _ => (1 + r % 14) as u8,
        };
        let nibbles: Vec<u8> = (0..slots / 2)
            .map(|_| nibble(next()) | (nibble(next()) << 4))
            .collect();
        let table = Dense {
            nibbles,
            bounds: std::sync::OnceLock::new(),
        };
        let classes: Vec<u8> = (0..slots)
            .map(|_| (next() % CLASSES as u64) as u8)
            .collect();
        let offsets: Vec<u32> = (0..slots as u32).filter(|_| next() % 4 != 0).collect();
        let start: Vec<WindowCandidate> = offsets
            .into_iter()
            .map(|offset| WindowCandidate {
                offset,
                single_index: 0,
                upper: (next() % 1000) as f64 / 100.0,
            })
            .collect();
        for cap in [0.0, 1.5, 5.5, ceiling, 20.0] {
            for (test, drop) in [(false, 0.0), (true, 9.0), (true, 14.0), (true, 40.0)] {
                let (mut fast, mut slow) = (start.clone(), start.clone());
                let kept = table_pass::<true>(
                    &mut fast, &table, &classes, 0, &bounds, cap, 1.0, test, drop,
                );
                let expected = table_pass::<false>(
                    &mut slow, &table, &classes, 0, &bounds, cap, 1.0, test, drop,
                );
                assert_eq!(kept, expected, "cap {cap} drop {drop}");
                let survivors: Vec<_> = start
                    .iter()
                    .map(|c| {
                        let f = usize::from(table.get(c.offset as usize));
                        let add = match f {
                            0 => 0.0,
                            15 => ceiling.min(cap),
                            _ => bounds[f * CLASSES + usize::from(classes[c.offset as usize])]
                                .min(cap),
                        };
                        (c.offset, c.upper + add)
                    })
                    .filter(|&(_, upper)| !(test && upper + 1.0 < drop))
                    .collect();
                assert_eq!(kept, survivors.len());
                for ((f, s), (offset, upper)) in fast.iter().zip(&slow).zip(&survivors).take(kept) {
                    assert_eq!((f.offset, f.upper.to_bits()), (s.offset, s.upper.to_bits()));
                    assert_eq!((f.offset, f.upper.to_bits()), (*offset, upper.to_bits()));
                }
            }
        }
    }

    #[test]
    fn lexical_impact_window_stamp_expires_frequencies() {
        let mut frequencies = WindowFrequencies {
            cells: vec![0; 2],
            ..Default::default()
        };
        let first = frequencies.next_stamp();
        frequencies.cells[0] = first | u64::from(u32::MAX);
        assert_eq!(frequencies.get(0, first), Some(u32::MAX));
        assert_eq!(frequencies.get(1, first), None);
        let second = frequencies.next_stamp();
        assert_eq!(frequencies.get(0, second), None);
        frequencies.stamp = !u64::from(u32::MAX);
        let after_wrap = frequencies.next_stamp();
        assert_eq!(after_wrap, first);
        assert_eq!(frequencies.get(0, after_wrap), None);
    }

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
