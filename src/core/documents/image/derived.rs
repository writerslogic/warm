//! The index-derived tables a version 4 checkpoint stores (feature bit [`FEATURE_DERIVED`]).
//!
//! Queries read per-slot word counts and classes, per-list rank tables, union window bounds
//! and direct frequency tables of the longest lists. Each is a pure function of the
//! checkpoint, so each is computed when the checkpoint is written (index or compaction time)
//! and stored in one extra section. A checkpoint without the section (an older one) gets the
//! same tables computed once when it is opened. No query builds or keeps any of them.
//!
//! Section layout, little endian: a header of nine `u64`s (format, average chunk length bits,
//! BM25 `k1` and `b` bits, class-edge checksum, the dense table limits, slot count), three
//! `u64` counts (rank tables, window bounds, dense tables), then the per-slot classes (`u8`),
//! per-slot word counts (`u16`), the rank tables, the window bounds and the dense tables, each
//! keyed by its list's offset (skip table offset, or postings offset for dense tables). A
//! reader whose constants differ from the header's computes the tables instead.
//!
//! The section is read in place: the direct frequency tables are views of the mapped file, not
//! copies. Which lists get one is fixed by [`select_tabled`]: the longest lists meeting the
//! length rule, within at most 1/8 of the checkpoint's bytes and 512 MiB (and never fewer
//! bytes than 32 MiB allows).
//!
//! Trust model. The section is covered by the checkpoint CRC, and open also checks every field
//! a query uses to index or size anything against the checkpoint itself: the norms, word
//! counts and classes are recomputed and compared, each rank table's run and block counts and
//! order permutation must fit its list, window bounds must fit the slot count and address
//! ranked lists, and the direct tables must be exactly the set [`select_tabled`] picks. A
//! crafted section is an error at open, never a panic or an out-of-bounds read. The weights
//! the tables carry (rank and window bounds, frequency nibbles) are not recomputed: a writer
//! of a plaintext store file can already rewrite its postings, so they are trusted as far as
//! the postings are.
//!
//! Not stored: checkpoints with compressed text blocks (version 3 layout) compute the tables
//! at open; `WARM_DENSE_TABLES=0` turns off only the direct frequency tables; the dense path
//! recomputes its length normalization per query.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, ensure, Context, Result};

use super::super::impact::{
    classes, select_tabled, Dense, DenseTables, WindowBounds, DENSE_BYTES, DENSE_CAP,
    DENSE_MIN_POSTINGS, DENSE_SHARE, DENSE_TABLES, NORM_CACHE, WINDOW,
};
use super::super::lists::{RankTable, RUN_BLOCKS};
use super::super::search::{serves_unpruned, weigh_list};
use super::weighted::{self, Weighted, POSTING_BYTES};
use super::{
    word, Backing, ListSpan, Postings, Ranks, Section, ServingImage, CRC_START, FEATURE_DERIVED,
    HEADER_LEN, H_FEATURES, H_SECTIONS,
};

/// The layout this build writes: format 1 plus the per-slot norms and the pre-weighted lists
/// after the direct tables. A build that knows only format 1 finds a header it does not match
/// and computes the tables, so it opens such a checkpoint and reads none of the new bytes.
const FORMAT: u64 = 2;
/// The layout of 0.7.5, still read: its tables are stored, the newer ones computed at open.
const FORMAT_1: u64 = 1;
const HEADER: usize = 12 * 8;
const HEADER_2: usize = 15 * 8;
/// The section the stored tables take: the slot version 3 gives compressed text blocks, which a
/// version 4 checkpoint never holds.
pub(super) const DERIVED_INDEX: usize = Section::TextBlocks as usize;

/// Environment switch: `0` opens without direct frequency tables, so a reviewer can check that
/// results and timings do not depend on them.
const DENSE_ENV: &str = "WARM_DENSE_TABLES";

/// Environment switch: `0` opens without the per-slot norms and pre-weighted lists.
const SMALL_ENV: &str = "WARM_SMALL_TABLES";

fn small_enabled() -> bool {
    std::env::var_os(SMALL_ENV).is_none_or(|value| value != "0")
}

fn dense_enabled() -> bool {
    std::env::var_os(DENSE_ENV).is_none_or(|value| value != "0")
}

/// The per-slot norms and pre-weighted lists of a checkpoint.
type Small = (Vec<f64>, Vec<(usize, Weighted)>);

/// Every table, in memory.
pub(super) struct Parts {
    ranks: Ranks,
    windows: Box<[(usize, WindowBounds)]>,
    word_counts: Vec<u16>,
    word_classes: Vec<u8>,
    /// Length normalization by word count under the default constants.
    norms: Vec<f64>,
    /// By postings offset in the postings section.
    dense: Vec<(usize, Dense)>,
    /// Length normalization of every slot; empty for a checkpoint the pruned ranker serves.
    slot_norms: Vec<f64>,
    /// Pre-weighted lists by postings offset, ascending.
    weighted: Vec<(usize, Weighted)>,
}

/// The direct-table budget rule (see [`select_tabled`]) as one header word.
fn dense_rule() -> u64 {
    (DENSE_BYTES as u64) ^ (DENSE_CAP as u64).rotate_left(21) ^ (DENSE_SHARE as u64).rotate_left(43)
}

fn class_checksum() -> u64 {
    let mut hasher = crc32fast::Hasher::new();
    for edge in classes().0 {
        hasher.update(&edge.to_le_bytes());
    }
    u64::from(hasher.finalize())
}

impl ServingImage {
    /// The postings of `span`.
    fn span_postings(&self, span: &ListSpan) -> Postings<'_> {
        Postings {
            bytes: &self.section(Section::Postings)[span.posting..span.posting + span.entry.len],
            count: span.entry.count as u32,
            blocked: self.blocked,
            impacts: self.impacts,
        }
    }

    /// Computes every table from the checkpoint.
    pub(super) fn compute_parts(&self) -> Parts {
        let ranks = self.compute_ranks();
        let windows = self.build_union_windows(&ranks);
        let (_, class_of) = classes();
        let last = class_of.len() - 1;
        let word_classes = (0..self.slots)
            .map(|slot| class_of[(self.slot(slot).0 as usize).min(last)])
            .collect();
        let word_counts = (0..self.slots)
            .map(|slot| self.slot(slot).0.min(u32::from(u16::MAX)) as u16)
            .collect();
        let chosen = self.chosen_tabled();
        let by_offset: HashMap<usize, &ListSpan> = self
            .tabled
            .iter()
            .map(|span| (span.posting, span))
            .collect();
        let dense = chosen
            .into_iter()
            .map(|offset| {
                let list = self.span_postings(by_offset[&offset]);
                (offset, Dense::build(self.slots, &list))
            })
            .collect();
        let norms = (0..NORM_CACHE)
            .map(|words| super::super::Bm25::DEFAULT.norm(words as f64, ranks.average))
            .collect();
        let (slot_norms, weighted) = self.compute_small();
        Parts {
            slot_norms,
            weighted,
            ranks,
            windows,
            word_counts,
            word_classes,
            norms,
            dense,
        }
    }

    /// The per-slot norms and pre-weighted lists, none for a checkpoint the pruned ranker serves.
    fn compute_small(&self) -> Small {
        if !serves_unpruned(self.slots) {
            return (Vec::new(), Vec::new());
        }
        let bm25 = super::super::Bm25::DEFAULT;
        let average = self.norm_average();
        let norms: Vec<f64> = (0..self.slots)
            .map(|slot| bm25.norm(f64::from(self.slot(slot).0), average))
            .collect();
        let by_offset: HashMap<usize, &ListSpan> = self
            .weighable
            .iter()
            .map(|span| (span.posting, span))
            .collect();
        let mut chosen = self.chosen_weighted();
        chosen.sort_unstable();
        let weighted = chosen
            .into_iter()
            .map(|offset| {
                let span = by_offset[&offset];
                let idf = super::super::Bm25::idf(self.chunks as f64, span.entry.count as f64);
                let (slots, weights) = weigh_list(&self.span_postings(span), idf, &norms);
                (offset, Weighted::computed(&slots, &weights))
            })
            .collect();
        (norms, weighted)
    }

    /// The postings offsets of the pre-weighted lists, a function of the index alone.
    fn chosen_weighted(&self) -> Vec<usize> {
        weighted::select(
            self.weighable
                .iter()
                .map(|span| (span.entry.count as usize, span.posting))
                .collect(),
            self.index_bytes(),
        )
        .into_iter()
        .map(|(_, offset)| offset)
        .collect()
    }

    /// The checkpoint's bytes without its derived section.
    fn index_bytes(&self) -> usize {
        let derived_len = if self.derived {
            self.section(Section::Derived).len()
        } else {
            0
        };
        self.bytes.len() - derived_len
    }

    /// The section as 0.7.5 wrote it: format 1, without the per-slot norms and weighted lists.
    #[cfg(test)]
    pub(in super::super) fn derived_section_v1(&self) -> Vec<u8> {
        let parts = self.compute_parts();
        let tail = parts.slot_norms.len() * 8
            + parts
                .weighted
                .iter()
                .map(|(_, list)| weighted::ENTRY_HEAD + POSTING_BYTES * list.count())
                .sum::<usize>();
        let full = self.derived_section();
        let mut out = full[..HEADER].to_vec();
        out[..8].copy_from_slice(&FORMAT_1.to_le_bytes());
        out.extend_from_slice(&full[HEADER_2..full.len() - tail]);
        out
    }

    /// The bytes of the section holding every table of this checkpoint.
    pub(in super::super) fn derived_section(&self) -> Vec<u8> {
        let parts = self.compute_parts();
        let mut out = Vec::new();
        let put = |out: &mut Vec<u8>, value: u64| out.extend_from_slice(&value.to_le_bytes());
        let bm25 = super::super::Bm25::DEFAULT;
        for value in [
            FORMAT,
            parts.ranks.average.to_bits(),
            bm25.k1.to_bits(),
            bm25.b.to_bits(),
            class_checksum(),
            dense_rule(),
            DENSE_TABLES as u64,
            DENSE_MIN_POSTINGS as u64,
            self.slots as u64,
            parts.ranks.tables.len() as u64,
            parts.windows.len() as u64,
            parts.dense.len() as u64,
            parts.slot_norms.len() as u64,
            parts.weighted.len() as u64,
            weighted::rule(),
        ] {
            put(&mut out, value);
        }
        for norm in &parts.norms {
            out.extend_from_slice(&norm.to_le_bytes());
        }
        out.extend_from_slice(&parts.word_classes);
        for count in &parts.word_counts {
            out.extend_from_slice(&count.to_le_bytes());
        }
        let mut tables: Vec<_> = parts.ranks.tables.iter().collect();
        tables.sort_unstable_by_key(|(skip, _)| **skip);
        for (&skip, table) in tables {
            put(&mut out, skip as u64);
            out.extend_from_slice(&(table.runs.len() as u32).to_le_bytes());
            out.extend_from_slice(&(table.blocks.len() as u32).to_le_bytes());
            for value in table.order.iter() {
                out.extend_from_slice(&value.to_le_bytes());
            }
            for value in table.runs.iter().chain(table.blocks.iter()) {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for (skip, bounds) in parts.windows.iter() {
            put(&mut out, *skip as u64);
            out.extend_from_slice(&(bounds.values().len() as u32).to_le_bytes());
            for value in bounds.values() {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for (offset, table) in &parts.dense {
            put(&mut out, *offset as u64);
            out.extend_from_slice(table.nibble_bytes());
        }
        for norm in &parts.slot_norms {
            out.extend_from_slice(&norm.to_le_bytes());
        }
        let n = self.chunks as f64;
        for (offset, list) in &parts.weighted {
            put(&mut out, *offset as u64);
            put(
                &mut out,
                super::super::Bm25::idf(n, list.count() as f64).to_bits(),
            );
            out.extend_from_slice(list.bytes(&[]));
        }
        out
    }

    /// The stored tables, or `None` when this build's constants differ from the ones they were
    /// computed under. Everything a query indexes with is checked against the checkpoint's own
    /// lists, so a crafted section is an error, never an out-of-bounds read. The bounds the
    /// tables carry (rank and window weights, frequency nibbles) are trusted as far as the
    /// postings are: whoever can write this file can already rewrite them.
    fn stored_parts(&self, bytes: &[u8], dense_on: bool) -> Result<Option<Parts>> {
        let mut reader = Reader(bytes);
        ensure!(bytes.len() >= HEADER, "derived tables are truncated");
        let header: Vec<u64> = (0..12).map(|_| reader.u64()).collect::<Result<_>>()?;
        let bm25 = super::super::Bm25::DEFAULT;
        let average = (self.words as f64 / (self.chunks as f64).max(1.0)).max(1.0);
        let format = header[0];
        let matches = matches!(format, FORMAT | FORMAT_1)
            && header[..9]
                == [
                    format,
                    average.to_bits(),
                    bm25.k1.to_bits(),
                    bm25.b.to_bits(),
                    class_checksum(),
                    dense_rule(),
                    DENSE_TABLES as u64,
                    DENSE_MIN_POSTINGS as u64,
                    self.slots as u64,
                ];
        if !matches {
            return Ok(None);
        }
        let small = if format == FORMAT {
            ensure!(bytes.len() >= HEADER_2, "derived tables are truncated");
            let words: Vec<u64> = (0..3).map(|_| reader.u64()).collect::<Result<_>>()?;
            if words[2] != weighted::rule() {
                return Ok(None);
            }
            Some((usize::try_from(words[0])?, usize::try_from(words[1])?))
        } else {
            None
        };
        let (rank_count, window_count, dense_count) = (
            usize::try_from(header[9])?,
            usize::try_from(header[10])?,
            usize::try_from(header[11])?,
        );
        let norms = reader.f64s(NORM_CACHE)?;
        ensure!(
            norms
                .iter()
                .enumerate()
                .all(|(words, norm)| norm.to_bits() == bm25.norm(words as f64, average).to_bits()),
            "derived length normalization does not match the checkpoint"
        );
        let word_classes = reader.take(self.slots)?.to_vec();
        let word_counts: Vec<u16> = reader
            .take(self.slots.checked_mul(2).context("derived tables size")?)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        let (_, class_of) = classes();
        let last = class_of.len() - 1;
        for slot in 0..self.slots {
            let words = self.slot(slot).0;
            ensure!(
                u32::from(word_counts[slot]) == words.min(u32::from(u16::MAX))
                    && word_classes[slot] == class_of[(words as usize).min(last)],
                "derived word counts do not match the checkpoint at slot {slot}"
            );
        }
        let lists: HashMap<usize, &ListSpan> =
            self.ranked.iter().map(|span| (span.skip, span)).collect();
        ensure!(
            rank_count == 0 || rank_count == lists.len(),
            "derived rank tables do not cover the checkpoint's ranked lists"
        );
        let mut tables = HashMap::new();
        for _ in 0..rank_count {
            let skip = usize::try_from(reader.u64()?)?;
            let span = lists
                .get(&skip)
                .context("derived rank table addresses no ranked list")?;
            let list = self.ranked_list(span);
            let (want_blocks, want_runs) = (list.blocks(), list.blocks().div_ceil(RUN_BLOCKS));
            let (runs, blocks) = (reader.u32()? as usize, reader.u32()? as usize);
            ensure!(
                runs == want_runs && blocks == want_blocks,
                "derived rank table does not fit its list"
            );
            let order = reader.u32s(runs)?;
            let mut seen = vec![false; runs];
            for &run in &order {
                let slot = seen
                    .get_mut(run as usize)
                    .context("derived rank order is out of range")?;
                ensure!(!*slot, "derived rank order repeats a run");
                *slot = true;
            }
            let run_bounds = reader.f32s(runs)?;
            let block_bounds = reader.f32s(blocks)?;
            ensure!(
                tables
                    .insert(
                        skip,
                        RankTable {
                            order: order.into_boxed_slice(),
                            runs: run_bounds.into_boxed_slice(),
                            blocks: block_bounds.into_boxed_slice(),
                        },
                    )
                    .is_none(),
                "derived rank tables repeat a list"
            );
        }
        let min_count = self.slots.div_ceil(16);
        let mut windows: Vec<(usize, WindowBounds)> = Vec::new();
        for _ in 0..window_count {
            let skip = usize::try_from(reader.u64()?)?;
            let span = lists
                .get(&skip)
                .context("derived window bounds address no ranked list")?;
            ensure!(
                span.entry.count as usize >= min_count
                    && windows.last().is_none_or(|(last, _)| *last < skip),
                "derived window bounds are misplaced"
            );
            let len = reader.u32()? as usize;
            ensure!(
                len == self.slots.div_ceil(WINDOW as usize),
                "derived window bounds do not fit the slot count"
            );
            windows.push((
                skip,
                WindowBounds::from_values(reader.f64s(len)?.into_boxed_slice()),
            ));
        }
        let table_len = self.slots.div_ceil(2);
        let wanted: HashSet<usize> = self.chosen_tabled().into_iter().collect();
        ensure!(
            dense_count == wanted.len(),
            "derived frequency tables are not the checkpoint's longest lists"
        );
        let mut dense = Vec::new();
        let mut keys = HashSet::new();
        for _ in 0..dense_count {
            let offset = usize::try_from(reader.u64()?)?;
            ensure!(
                wanted.contains(&offset) && keys.insert(offset),
                "derived frequency table addresses a list that is not tabled"
            );
            let nibbles = reader.take(table_len)?;
            if dense_on {
                // SAFETY: the nibbles are bytes of this image's own mapping, which it keeps
                // mapped and unmoved for its whole life and which owns the table.
                dense.push((offset, unsafe { Dense::mapped(nibbles) }));
            }
        }
        let (slot_norms, weighted) = match small {
            None => (Vec::new(), Vec::new()),
            Some((norm_count, weighted_count)) => {
                match self.stored_small(&mut reader, bytes, norm_count, weighted_count, average)? {
                    Some(small) => small,
                    None => return Ok(None),
                }
            }
        };
        ensure!(reader.0.is_empty(), "derived tables have trailing bytes");
        // A checkpoint of the 0.7.5 layout stores neither; they are a function of the index.
        let (slot_norms, weighted) = if format == FORMAT_1 {
            self.compute_small()
        } else {
            (slot_norms, weighted)
        };
        Ok(Some(Parts {
            slot_norms,
            weighted,
            ranks: Ranks { average, tables },
            windows: windows.into_boxed_slice(),
            word_counts,
            word_classes,
            norms,
            dense,
        }))
    }

    /// Reads the per-slot norms and pre-weighted lists after the direct tables. Everything a
    /// query indexes with is checked against the checkpoint; the weights are trusted as far as
    /// the postings are, like the other bounds. Returns nothing stored when the writer's
    /// inverse document frequencies differ from this platform's (a different `ln`), so the
    /// caller computes them.
    fn stored_small(
        &self,
        reader: &mut Reader<'_>,
        section: &[u8],
        norm_count: usize,
        weighted_count: usize,
        average: f64,
    ) -> Result<Option<Small>> {
        let expected = serves_unpruned(self.slots);
        ensure!(
            norm_count == if expected { self.slots } else { 0 },
            "derived slot norms do not cover the checkpoint"
        );
        let slot_norms = reader.f64s(norm_count)?;
        let bm25 = super::super::Bm25::DEFAULT;
        for (slot, norm) in slot_norms.iter().enumerate() {
            ensure!(
                norm.to_bits() == bm25.norm(f64::from(self.slot(slot).0), average).to_bits(),
                "derived slot norms do not match the checkpoint at slot {slot}"
            );
        }
        let counts: HashMap<usize, usize> = self
            .weighable
            .iter()
            .map(|span| (span.posting, span.entry.count as usize))
            .collect();
        let wanted: HashSet<usize> = if expected {
            self.chosen_weighted().into_iter().collect()
        } else {
            HashSet::new()
        };
        ensure!(
            weighted_count == wanted.len(),
            "derived weighted lists are not the checkpoint's longest lists"
        );
        let n = self.chunks as f64;
        let mut weighted = Vec::with_capacity(weighted_count);
        let mut previous = None;
        for _ in 0..weighted_count {
            let offset = usize::try_from(reader.u64()?)?;
            ensure!(
                wanted.contains(&offset) && previous.is_none_or(|last| last < offset),
                "derived weighted list addresses a list that is not weighted"
            );
            previous = Some(offset);
            let count = counts[&offset];
            if reader.u64()? != super::super::Bm25::idf(n, count as f64).to_bits() {
                return Ok(None);
            }
            let start = reader.0.as_ptr() as usize - section.as_ptr() as usize;
            let body = reader.take(
                count
                    .checked_mul(POSTING_BYTES)
                    .context("derived tables size")?,
            )?;
            let (slots, weights) = body.split_at(4 * count);
            ensure!(
                slots
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|slot| (u32::from_le_bytes(*slot) as usize) < self.slots)
                    && weights
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .all(|weight| f64::from_le_bytes(*weight).is_finite()),
                "derived weighted list does not fit the slot count"
            );
            weighted.push((
                offset,
                Weighted::stored(start..start + count * POSTING_BYTES, count),
            ));
        }
        Ok(Some((slot_norms, weighted)))
    }

    /// The postings offsets of the lists that get direct tables, a function of the index alone.
    fn chosen_tabled(&self) -> Vec<usize> {
        select_tabled(
            self.tabled
                .iter()
                .map(|span| (span.entry.count as usize, span.posting))
                .collect(),
            self.slots,
            self.index_bytes(),
        )
        .into_iter()
        .map(|(_, offset)| offset)
        .collect()
    }

    /// Installs every table the queries read: the stored ones, or computed ones for a
    /// checkpoint without them. Called once, when the checkpoint is opened.
    pub(crate) fn with_derived_tables(self) -> Result<Self> {
        self.with_derived_tables_using(dense_enabled())
    }

    /// [`Self::with_derived_tables`] with the direct frequency tables on or off.
    pub(crate) fn with_derived_tables_using(self, dense: bool) -> Result<Self> {
        self.with_derived_tables_options(dense, small_enabled())
    }

    /// [`Self::with_derived_tables`] with the direct frequency tables and the small-store
    /// tables (per-slot norms, pre-weighted lists) each on or off.
    pub(crate) fn with_derived_tables_options(mut self, dense: bool, small: bool) -> Result<Self> {
        let stored = match self.derived_bytes() {
            Some(bytes) => self.stored_parts(bytes, dense)?,
            None => None,
        };
        let mut parts = match stored {
            Some(parts) => parts,
            None => {
                let mut parts = self.compute_parts();
                if !dense {
                    parts.dense.clear();
                }
                parts
            }
        };
        if !small {
            parts.slot_norms.clear();
            parts.weighted.clear();
        }
        let offset = self.section(Section::Postings).as_ptr() as usize;
        self.dense = DenseTables::new(
            std::mem::take(&mut parts.dense)
                .into_iter()
                .map(|(at, table)| (offset + at, table)),
        );
        self.union_windows = parts.windows;
        self.norms = parts.norms;
        let postings = self.section(Section::Postings).as_ptr() as usize;
        self.slot_norms = parts.slot_norms;
        self.weighted = parts
            .weighted
            .into_iter()
            .map(|(at, list)| (postings + at, list))
            .collect();
        let _ = self.ranks.set(parts.ranks);
        let _ = self.word_counts.set(parts.word_counts);
        let _ = self.word_classes.set(parts.word_classes);
        Ok(self)
    }

    pub(super) fn derived_bytes(&self) -> Option<&[u8]> {
        self.derived.then(|| self.section(Section::Derived))
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.0.len() < len {
            bail!("derived tables are truncated");
        }
        let (head, tail) = self.0.split_at(len);
        self.0 = tail;
        Ok(head)
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn u32s(&mut self, count: usize) -> Result<Vec<u32>> {
        let bytes = self.take(count.checked_mul(4).context("derived tables size")?)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect())
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>> {
        Ok(self.u32s(count)?.into_iter().map(f32::from_bits).collect())
    }

    fn f64s(&mut self, count: usize) -> Result<Vec<f64>> {
        let bytes = self.take(count.checked_mul(8).context("derived tables size")?)?;
        Ok(bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| f64::from_le_bytes(*b))
            .collect())
    }
}

/// The header of `base` with the derived section recorded at `at` for `len` bytes, and its
/// feature bit set. The CRC word is left for the caller.
pub(super) fn attached_header(base: &[u8], at: u64, len: u64) -> Vec<u8> {
    let mut header = base[..HEADER_LEN].to_vec();
    let put = |header: &mut [u8], index: usize, value: u64| {
        let at = 8 + 8 * index;
        header[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    put(
        &mut header,
        H_FEATURES,
        word(base, H_FEATURES) | FEATURE_DERIVED,
    );
    put(&mut header, H_SECTIONS + 2 * DERIVED_INDEX, at);
    put(&mut header, H_SECTIONS + 2 * DERIVED_INDEX + 1, len);
    header
}

/// `bytes`, a version 4 checkpoint without derived tables, with them.
pub(super) fn attach(bytes: Vec<u8>) -> Result<Vec<u8>> {
    attach_section(bytes, ServingImage::derived_section)
}

/// [`attach`] with the section `section` makes of the checkpoint.
pub(super) fn attach_section(
    bytes: Vec<u8>,
    section: impl FnOnce(&ServingImage) -> Vec<u8>,
) -> Result<Vec<u8>> {
    let image = ServingImage::new(Backing::Owned(bytes))?;
    let section = section(&image);
    let Backing::Owned(mut out) = image.bytes else {
        unreachable!("the checkpoint was opened owned");
    };
    let at = out.len() as u64;
    let header = attached_header(&out, at, section.len() as u64);
    out[..HEADER_LEN].copy_from_slice(&header);
    out.extend_from_slice(&section);
    let crc = crc32fast::hash(&out[CRC_START..]);
    out[8..16].copy_from_slice(&u64::from(crc).to_le_bytes());
    Ok(out)
}
