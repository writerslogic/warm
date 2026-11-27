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

use std::collections::HashMap;

use anyhow::{bail, ensure, Context, Result};

use super::super::impact::{
    classes, select_tabled, Dense, DenseTables, WindowBounds, DENSE_BYTES, DENSE_MIN_POSTINGS,
    DENSE_TABLES, NORM_CACHE,
};
use super::super::lists::RankTable;
use super::{
    word, Backing, ListSpan, Postings, Ranks, Section, ServingImage, CRC_START, FEATURE_DERIVED,
    HEADER_LEN, H_FEATURES, H_SECTIONS,
};

const FORMAT: u64 = 1;
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
        let chosen = select_tabled(
            self.tabled
                .iter()
                .map(|span| (span.entry.count as usize, span.posting))
                .collect(),
            self.slots,
        );
        let by_offset: HashMap<usize, &ListSpan> = self
            .tabled
            .iter()
            .map(|span| (span.posting, span))
            .collect();
        let dense = chosen
            .into_iter()
            .map(|(_, offset)| {
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
            DENSE_BYTES as u64,
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
        out
    }

    /// The stored tables, or `None` when this build's constants differ from the ones they were
    /// computed under.
    fn stored_parts(&self, bytes: &[u8], dense_on: bool) -> Result<Option<Parts>> {
        let mut reader = Reader(bytes);
        ensure!(bytes.len() >= HEADER, "derived tables are truncated");
        let header: Vec<u64> = (0..12).map(|_| reader.u64()).collect::<Result<_>>()?;
        let bm25 = super::super::Bm25::DEFAULT;
        let average = (self.words as f64 / (self.chunks as f64).max(1.0)).max(1.0);
        let matches = header[..9]
            == [
                FORMAT,
                average.to_bits(),
                bm25.k1.to_bits(),
                bm25.b.to_bits(),
                class_checksum(),
                DENSE_BYTES as u64,
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
        let word_classes = reader.take(self.slots)?.to_vec();
        let word_counts = reader
            .take(self.slots.checked_mul(2).context("derived tables size")?)?
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let mut tables = HashMap::new();
        for _ in 0..rank_count {
            let skip = usize::try_from(reader.u64()?)?;
            let (runs, blocks) = (reader.u32()? as usize, reader.u32()? as usize);
            let order = reader.u32s(runs)?;
            let run_bounds = reader.f32s(runs)?;
            let block_bounds = reader.f32s(blocks)?;
            tables.insert(
                skip,
                RankTable {
                    order: order.into_boxed_slice(),
                    runs: run_bounds.into_boxed_slice(),
                    blocks: block_bounds.into_boxed_slice(),
                },
            );
        }
        let mut windows = Vec::new();
        for _ in 0..window_count {
            let skip = usize::try_from(reader.u64()?)?;
            let len = reader.u32()? as usize;
            windows.push((
                skip,
                WindowBounds::from_values(reader.f64s(len)?.into_boxed_slice()),
            ));
        }
        let mut dense = Vec::new();
        if dense_on {
            for _ in 0..dense_count {
                let offset = usize::try_from(reader.u64()?)?;
                let nibbles = reader.take(self.slots.div_ceil(2))?.to_vec();
                dense.push((offset, Dense::from_nibbles(nibbles)));
            }
        } else {
            let skipped = dense_count
                .checked_mul(8 + self.slots.div_ceil(2))
                .context("derived tables size")?;
            reader.take(skipped)?;
        }
        ensure!(reader.0.is_empty(), "derived tables have trailing bytes");
        Ok(Some(Parts {
            ranks: Ranks { average, tables },
            windows: windows.into_boxed_slice(),
            word_counts,
            word_classes,
            norms,
            dense,
        }))
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
        self.norms = parts.norms;
        let _ = self.ranks.set(parts.ranks);
        let _ = self.word_counts.set(parts.word_counts);
        let _ = self.word_classes.set(parts.word_classes);
        Ok(self)
    }

    fn derived_bytes(&self) -> Option<&[u8]> {
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
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>> {
        Ok(self.u32s(count)?.into_iter().map(f32::from_bits).collect())
    }

    fn f64s(&mut self, count: usize) -> Result<Vec<f64>> {
        let bytes = self.take(count.checked_mul(8).context("derived tables size")?)?;
        Ok(bytes
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().expect("eight bytes")))
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
