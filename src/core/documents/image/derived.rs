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
//! Format 2 follows the tables with padding to eight bytes, a count of blocks and that many
//! `(kind, offset, len)` words, then the blocks, which end the section (kind 3: champion
//! postings, see `champions`). A reader skips kinds it does not know.
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
use super::{
    word, Backing, ListSpan, Postings, Ranks, Section, ServingImage, CRC_START, FEATURE_DERIVED,
    HEADER_LEN, H_FEATURES, H_SECTIONS,
};

/// Format 2 follows the tables with a directory of extra blocks (see the module comment).
/// Format 1 is #554's, which this build opens with every extra structure off.
const FORMAT: u64 = 2;
const FORMAT_TABLES_ONLY: u64 = 1;
const HEADER: usize = 12 * 8;
/// The section the stored tables take: the slot version 3 gives compressed text blocks, which a
/// version 4 checkpoint never holds.
pub(super) const DERIVED_INDEX: usize = Section::TextBlocks as usize;

/// Environment switch: `0` opens without direct frequency tables, so a reviewer can check that
/// results and timings do not depend on them.
const DENSE_ENV: &str = "WARM_DENSE_TABLES";

fn dense_enabled() -> bool {
    std::env::var_os(DENSE_ENV).is_none_or(|value| value != "0")
}

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
    champions: super::champions::ChampionIndex,
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
        Parts {
            ranks,
            windows,
            word_counts,
            word_classes,
            norms,
            dense,
            champions: Default::default(),
        }
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
        let mut blocks = Vec::new();
        if let Some(block) = self.champion_block() {
            blocks.push((super::champions::KIND, block));
        }
        while out.len() % 8 != 0 {
            out.push(0);
        }
        put(&mut out, blocks.len() as u64);
        let mut at = out.len() + 24 * blocks.len();
        for (kind, block) in &blocks {
            for value in [*kind, at as u64, block.len() as u64] {
                put(&mut out, value);
            }
            at += block.len();
        }
        for (_, block) in &blocks {
            out.extend_from_slice(block);
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
        let matches = (format == FORMAT || format == FORMAT_TABLES_ONLY)
            && header[1..9]
                == [
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
        let champions = if format == FORMAT_TABLES_ONLY {
            ensure!(reader.0.is_empty(), "derived tables have trailing bytes");
            Default::default()
        } else {
            self.stored_blocks(bytes, &mut reader)?
        };
        Ok(Some(Parts {
            ranks: Ranks { average, tables },
            windows: windows.into_boxed_slice(),
            word_counts,
            word_classes,
            norms,
            dense,
            champions,
        }))
    }

    /// The directory and blocks after the tables of a format 2 section: each known kind is
    /// checked and read, an unknown one skipped, and the last block ends the section.
    fn stored_blocks(
        &self,
        bytes: &[u8],
        reader: &mut Reader<'_>,
    ) -> Result<super::champions::ChampionIndex> {
        let pad = (8 - (bytes.len() - reader.0.len()) % 8) % 8;
        ensure!(
            reader.take(pad)?.iter().all(|&byte| byte == 0),
            "derived tables have nonzero padding"
        );
        let listed = usize::try_from(reader.u64()?)?;
        let mut entries = Vec::new();
        for _ in 0..listed {
            entries.push((
                reader.u64()?,
                usize::try_from(reader.u64()?)?,
                usize::try_from(reader.u64()?)?,
            ));
        }
        let mut at = bytes.len() - reader.0.len();
        let mut champions = Default::default();
        let mut seen = HashSet::new();
        for (kind, offset, len) in entries {
            ensure!(
                offset == at && seen.insert(kind),
                "derived blocks are misplaced"
            );
            let block = reader.take(len)?;
            at = offset.checked_add(len).context("derived tables size")?;
            if kind == super::champions::KIND {
                champions = self.parse_champions(block, offset)?;
            }
        }
        ensure!(reader.0.is_empty(), "derived tables have trailing bytes");
        Ok(champions)
    }

    /// The postings offsets of the lists that get direct tables, a function of the index alone.
    fn chosen_tabled(&self) -> Vec<usize> {
        let derived_len = if self.derived {
            self.section(Section::Derived).len()
        } else {
            0
        };
        select_tabled(
            self.tabled
                .iter()
                .map(|span| (span.entry.count as usize, span.posting))
                .collect(),
            self.slots,
            self.bytes.len() - derived_len,
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
    pub(crate) fn with_derived_tables_using(mut self, dense: bool) -> Result<Self> {
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
        let offset = self.section(Section::Postings).as_ptr() as usize;
        self.dense = DenseTables::new(
            std::mem::take(&mut parts.dense)
                .into_iter()
                .map(|(at, table)| (offset + at, table)),
        );
        self.union_windows = parts.windows;
        if super::champions::enabled() {
            self.champions = parts.champions;
        }
        self.norms = parts.norms;
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
    let image = ServingImage::new(Backing::Owned(bytes))?;
    let section = image.derived_section();
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
