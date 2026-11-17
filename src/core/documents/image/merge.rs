// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Writes the checkpoint of a checkpoint plus the live store written since, or of a live store
//! alone, as a stream: term dictionaries merge in term order, the checkpoint's documents and
//! posting lists of terms the live store lacks are copied as bytes, and every section goes to a
//! block-addressed run ([`BlockWriter`]) instead of a buffer. Nothing larger than the live
//! store, one posting list and one document record is held at once, however large the
//! checkpoint is. The result is byte-for-byte the checkpoint [`super::encode`] writes for the
//! store holding both sets of documents (`tests::merged_checkpoint_equals_the_whole_encoding`).
//! The merge writes a version 4 checkpoint and needs a version 4 base with no dead documents, so it applies to a store
//! written once and extended by inserts; anything else keeps its in-memory path.

use std::path::Path;

use anyhow::{ensure, Context, Result};

use super::{
    append_skips, encode_document, put_bytes, put_varint, DocumentStore, PositionPacker, Reader,
    ServingImage, BLOCK, FLAG_POSITIONAL, FLAG_POSITIVE, FREE, HEADER_LEN, HEADER_WORDS, H_CHUNKS,
    H_DIMENSIONS, H_DOCS, H_FLAGS, H_GENERATION, H_ORDERED, H_SECTIONS, H_SLOTS, H_TERMS, H_WORDS,
    MAGIC, SLOT_BYTES, V4_SECTIONS,
};
use super::{u32_at, Section};
use crate::core::documents::lists::{Cursor, ListRef, NO_DOC};
use crate::core::documents::postings::{BlockInfo, CheckpointPacker};
use crate::core::durable_file::{BlockWriter, Spool};

/// Pages of the mapped checkpoint are released after this many bytes of it were read.
const RELEASE_EVERY: usize = 4 << 20;

/// One term of the checkpoint's dictionary with its posting bytes.
struct BaseTerm<'a> {
    term: Vec<u8>,
    count: u32,
    bytes: &'a [u8],
    skips: &'a [u8],
    positions: &'a [u8],
}

/// Sequential reader of one form's dictionary and posting bytes in the checkpoint.
struct BaseTerms<'a> {
    reader: Reader<'a>,
    postings: &'a [u8],
    skips: &'a [u8],
    positions: &'a [u8],
    at: usize,
    skip_at: usize,
    position_at: usize,
    left: usize,
    index: usize,
    previous: Vec<u8>,
}

impl<'a> BaseTerms<'a> {
    fn new(image: &'a ServingImage, form: usize) -> Self {
        let dict = if form == 0 {
            Section::Dict0
        } else {
            Section::Dict1
        };
        Self {
            reader: Reader::new(image.section(dict)),
            postings: image.section(Section::Postings),
            skips: image.section(Section::Skips),
            positions: image.section(Section::Positions),
            at: 0,
            skip_at: 0,
            position_at: 0,
            left: image.dictionaries[form].terms,
            index: 0,
            previous: Vec::new(),
        }
    }

    fn next(&mut self) -> Result<Option<BaseTerm<'a>>> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        let first_of_block = self.index.is_multiple_of(BLOCK);
        self.index += 1;
        let prefix = usize::try_from(self.reader.varint()?)?;
        let suffix = self.reader.bytes()?;
        ensure!(
            prefix <= self.previous.len() && (!first_of_block || prefix == 0),
            "checkpoint term prefix"
        );
        self.previous.truncate(prefix);
        self.previous.extend_from_slice(suffix);
        let count = u32::try_from(self.reader.varint()?)?;
        let len = usize::try_from(self.reader.varint()?)?;
        let position_len = usize::try_from(self.reader.varint()?)?;
        let skip_len = usize::try_from(self.reader.varint()?)?;
        let bytes = self
            .postings
            .get(self.at..self.at + len)
            .context("checkpoint posting range")?;
        let positions = self
            .positions
            .get(self.position_at..self.position_at + position_len)
            .context("checkpoint position range")?;
        let skips = self
            .skips
            .get(self.skip_at..self.skip_at + skip_len)
            .context("checkpoint skip range")?;
        self.at += len;
        self.position_at += position_len;
        self.skip_at += skip_len;
        Ok(Some(BaseTerm {
            term: self.previous.clone(),
            count,
            bytes,
            skips,
            positions,
        }))
    }
}

/// One form's output: dictionary, block table, and the shared posting run.
struct FormWriter {
    dict: BlockWriter,
    blocks: BlockWriter,
    terms: u64,
    previous: Vec<u8>,
    entry: Vec<u8>,
}

impl FormWriter {
    fn new(directory: &Path) -> Result<Self> {
        Ok(Self {
            dict: BlockWriter::create_in(directory)?,
            blocks: BlockWriter::create_in(directory)?,
            terms: 0,
            previous: Vec::new(),
            entry: Vec::new(),
        })
    }

    /// Appends one term whose postings, skip table and positions were just written to the
    /// shared runs after `at`. `None` when an offset no longer fits the format.
    fn push(&mut self, term: &[u8], count: u64, at: Runs, runs: &Runs) -> Result<Option<()>> {
        let prefix = if self.terms.is_multiple_of(BLOCK as u64) {
            for value in [self.dict.len(), at.postings, at.skips, at.positions] {
                self.blocks.write(&value.to_le_bytes())?;
            }
            0
        } else {
            self.previous
                .iter()
                .zip(term)
                .take_while(|(a, b)| a == b)
                .count()
        };
        self.entry.clear();
        put_varint(&mut self.entry, prefix as u64);
        put_bytes(&mut self.entry, &term[prefix..]);
        put_varint(&mut self.entry, count);
        put_varint(&mut self.entry, runs.postings - at.postings);
        put_varint(&mut self.entry, runs.positions - at.positions);
        put_varint(&mut self.entry, runs.skips - at.skips);
        self.dict.write(&self.entry)?;
        self.previous.clear();
        self.previous.extend_from_slice(term);
        self.terms += 1;
        Ok(Some(()))
    }
}

/// Lengths of the three shared runs (postings, skip tables, positions) at one moment.
#[derive(Clone, Copy)]
struct Runs {
    postings: u64,
    skips: u64,
    positions: u64,
}

/// The shared runs one form's merge appends to.
struct Streams<'w> {
    postings: &'w mut BlockWriter,
    skips: &'w mut BlockWriter,
    positions: &'w mut BlockWriter,
    /// Whether the merged checkpoint carries word positions.
    positional: bool,
    /// Cleared when a merged list holds a posting of frequency zero.
    positive: bool,
}

impl Streams<'_> {
    fn lens(&self) -> Runs {
        Runs {
            postings: self.postings.len(),
            skips: self.skips.len(),
            positions: self.positions.len(),
        }
    }
}

/// Merges one form's terms. `None` when an offset exceeds the format.
fn merge_form(
    base: Option<&ServingImage>,
    store: &DocumentStore,
    form: usize,
    split: usize,
    out: &mut FormWriter,
    streams: &mut Streams<'_>,
) -> Result<Option<()>> {
    let (terms, offsets, lists) = store.postings[form].sorted_terms();
    let mut live = (0..lists.len())
        .filter(|&i| !lists[i].is_empty())
        .peekable();
    let mut checkpoint = base.map(|image| BaseTerms::new(image, form));
    let mut pending = match checkpoint.as_mut() {
        Some(source) => source.next()?,
        None => None,
    };
    let positional = streams.positional;
    let mut scratch = Vec::new();
    let mut skip_scratch = Vec::new();
    let mut read = 0usize;
    // The impact bounds of a list come from the word count of each slot it holds: the checkpoint's
    // for its own slots, the store's for those written since.
    let slot_table = base.map_or(&[][..], |image| image.section(Section::Slots));
    let words = |slot: usize| {
        if slot < split {
            u32_at(slot_table, slot * SLOT_BYTES)
        } else {
            DocumentStore::slot_word_count(store, slot - split)
        }
    };
    let mut packer = CheckpointPacker::with_info(true, &words, true);
    let mut placed = PositionPacker::new();
    loop {
        let live_term = live
            .peek()
            .map(|&i| &terms[offsets[i] as usize..offsets[i + 1] as usize]);
        let order = match (&pending, live_term) {
            (None, None) => break,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(b), Some(l)) => b.term.as_slice().cmp(l),
        };
        let at = streams.lens();
        let (term, count) = if order == std::cmp::Ordering::Less {
            let b = pending.take().expect("ordered by a checkpoint term");
            streams.postings.write(b.bytes)?;
            streams.skips.write(b.skips)?;
            streams.positions.write(b.positions)?;
            read += b.bytes.len() + b.skips.len() + b.positions.len();
            (b.term, u64::from(b.count))
        } else {
            let from_base = order == std::cmp::Ordering::Equal;
            let base_term = from_base.then(|| pending.take().expect("equal terms"));
            let i = live.next().expect("a live term");
            scratch.clear();
            skip_scratch.clear();
            placed.reset();
            if let Some(b) = &base_term {
                let mut cursor = Cursor::open(ListRef {
                    bytes: b.bytes,
                    count: b.count,
                    skips: b.skips,
                    positions: b.positions,
                });
                while cursor.doc() != NO_DOC {
                    packer.push(cursor.doc() as usize, cursor.frequency(), &mut scratch);
                    if positional {
                        placed.push(cursor.position_bytes());
                    }
                    cursor.next();
                }
                read += b.bytes.len() + b.skips.len() + b.positions.len();
            }
            streams.positive &= lists[i].positive_len() == lists[i].len();
            let mut failure = None;
            lists[i].for_each_with_ordinal(|slot, frequency, ordinal| {
                packer.push(slot + split, frequency, &mut scratch);
                if positional && failure.is_none() {
                    match store
                        .position_source(slot)
                        .filter(|_| ordinal > 0)
                        .map(|source| source.bytes(ordinal as usize - 1))
                    {
                        Some(bytes) => placed.push(bytes),
                        None => failure = Some("positional posting lacks its positions"),
                    }
                }
            });
            ensure!(failure.is_none(), "{}", failure.unwrap_or_default());
            packer.finish(&mut scratch);
            let infos: Vec<BlockInfo> = packer.take_infos();
            if !placed.finish() || u32::try_from(scratch.len()).is_err() {
                return Ok(None);
            }
            let count =
                lists[i].len() as u64 + base_term.as_ref().map_or(0, |b| u64::from(b.count));
            if !append_skips(&infos, &placed.offsets, &scratch, count, &mut skip_scratch) {
                return Ok(None);
            }
            streams.postings.write(&scratch)?;
            streams.skips.write(&skip_scratch)?;
            streams.positions.write(&placed.out)?;
            let term = match base_term {
                Some(b) => b.term,
                None => terms[offsets[i] as usize..offsets[i + 1] as usize].to_vec(),
            };
            (term, count)
        };
        if out.push(&term, count, at, &streams.lens())?.is_none() {
            return Ok(None);
        }
        if order != std::cmp::Ordering::Greater {
            pending = match checkpoint.as_mut() {
                Some(source) => source.next()?,
                None => None,
            };
            if read >= RELEASE_EVERY {
                read = 0;
                if let Some(image) = base {
                    image.release_pages();
                }
            }
        }
    }
    Ok(Some(()))
}

/// Streams the checkpoint of `base` (when there is one) plus `store` to `destination`, durably
/// and atomically; the number of bytes written. `None` when the combined store does not fit the
/// checkpoint layout or cannot be merged (a version 1 checkpoint, one with compressed text, or
/// one with dead documents).
pub(crate) fn merge(
    base: Option<&ServingImage>,
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
    destination: &Path,
    order: Option<&SlotOrder>,
) -> Result<Option<u64>> {
    let Some(positional) = positions_of(base, store) else {
        return Ok(None);
    };
    if !super::representable(store)
        || base.is_some_and(|image| !image.mergeable() || image.dimensions != dimensions)
    {
        return Ok(None);
    }
    let directory = destination.parent().unwrap_or_else(|| Path::new("."));
    let split = base.map_or(0, |image| image.slots);
    let mut words = [0u64; HEADER_WORDS];
    let mut run: Vec<Option<BlockWriter>> = (0..V4_SECTIONS).map(|_| None).collect();
    let mut postings = BlockWriter::create_in(directory)?;
    let mut skips = BlockWriter::create_in(directory)?;
    let mut positions = BlockWriter::create_in(directory)?;
    let mut streams = Streams {
        postings: &mut postings,
        skips: &mut skips,
        positions: &mut positions,
        positional,
        positive: base.is_none_or(|image| image.positive),
    };
    // With an order, each new slot's word count: the checkpoint's for its slots, the store's
    // for those written since.
    let new_words: Vec<u32> = order.map_or_else(Vec::new, |order| {
        let table = base.map_or(&[][..], |image| image.section(Section::Slots));
        let mut words = vec![0u32; order.slots];
        for (slot, &new) in order.map.iter().enumerate() {
            if new != u32::MAX {
                words[new as usize] = if slot < split {
                    u32_at(table, slot * SLOT_BYTES)
                } else {
                    DocumentStore::slot_word_count(store, slot - split)
                };
            }
        }
        words
    });
    let new_word = |slot: usize| new_words[slot];
    for form in 0..2 {
        let mut out = FormWriter::new(directory)?;
        let merged = match order {
            Some(order) => merge_form_ordered(
                base,
                store,
                form,
                split,
                order,
                &new_word,
                &mut out,
                &mut streams,
            )?,
            None => merge_form(base, store, form, split, &mut out, &mut streams)?,
        };
        if merged.is_none() {
            return Ok(None);
        }
        words[H_TERMS + form] = out.terms;
        let (d, b) = if form == 0 {
            (Section::Dict0, Section::Blocks0)
        } else {
            (Section::Dict1, Section::Blocks1)
        };
        run[d as usize] = Some(out.dict);
        run[b as usize] = Some(out.blocks);
    }
    if positional {
        words[H_FLAGS] |= FLAG_POSITIONAL;
    }
    if streams.positive {
        words[H_FLAGS] |= FLAG_POSITIVE;
    }
    run[Section::Postings as usize] = Some(postings);
    run[Section::Skips as usize] = Some(skips);
    run[Section::Positions as usize] = Some(positions);
    #[cfg(test)]
    crate::core::storage::compaction_crash_point("fold-postings");

    // Documents: both id-ordered sets interleave; the checkpoint's records are copied as bytes.
    let segment = store.text_segment();
    let mut id_offsets = BlockWriter::create_in(directory)?;
    let mut ids = BlockWriter::create_in(directory)?;
    let mut doc_offsets = BlockWriter::create_in(directory)?;
    let mut docs = BlockWriter::create_in(directory)?;
    let base_documents = base.map_or(0, |image| image.docs);
    let mut base_ordinal = vec![0u32; base_documents];
    let mut owners = vec![(FREE, 0u32); store.slot_count];
    let mut live = store.documents.iter().peekable();
    let mut next_base = 0usize;
    let mut ordinal = 0u32;
    let mut record = Vec::new();
    loop {
        let from_base = match (next_base < base_documents, live.peek()) {
            (false, None) => break,
            (true, None) => true,
            (false, Some(_)) => false,
            (true, Some((id, _))) => {
                let image = base.expect("base documents exist");
                let order = image.id(next_base).cmp(id.as_str());
                ensure!(
                    order != std::cmp::Ordering::Equal,
                    "document in both the checkpoint and the store written since"
                );
                order == std::cmp::Ordering::Less
            }
        };
        let (Ok(id_at), Ok(doc_at)) = (u32::try_from(ids.len()), u32::try_from(docs.len())) else {
            return Ok(None);
        };
        id_offsets.write(&id_at.to_le_bytes())?;
        doc_offsets.write(&doc_at.to_le_bytes())?;
        if from_base {
            let image = base.expect("base documents exist");
            ids.write(image.id(next_base).as_bytes())?;
            let (start, end) = image.span(Section::DocOffsets, next_base);
            docs.write(&image.section(Section::Docs)[start..end])?;
            base_ordinal[next_base] = ordinal;
            next_base += 1;
            if next_base.is_multiple_of(RELEASE_EVERY / 64) {
                image.release_pages();
            }
        } else {
            let (id, document) = live.next().expect("peeked");
            let document_slots = store.slots.get(id).context("document without slots")?;
            ensure!(
                document_slots.len() == document.chunks.len(),
                "document slots and chunks disagree"
            );
            for (chunk, &slot) in document_slots.iter().enumerate() {
                owners[slot] = (ordinal, u32::try_from(chunk).context("chunk index")?);
            }
            ids.write(id.as_bytes())?;
            record.clear();
            encode_document(&mut record, document, segment.as_deref(), None)?;
            docs.write(&record)?;
        }
        ordinal = ordinal
            .checked_add(1)
            .context("checkpoint document count")?;
    }
    let (Ok(id_end), Ok(doc_end)) = (u32::try_from(ids.len()), u32::try_from(docs.len())) else {
        return Ok(None);
    };
    id_offsets.write(&id_end.to_le_bytes())?;
    doc_offsets.write(&doc_end.to_le_bytes())?;

    // Slots: the checkpoint's, owners renumbered, then the live store's after them; with an
    // order, each at its new slot.
    let mut slots = BlockWriter::create_in(directory)?;
    if let Some(order) = order {
        let table = base.map_or(&[][..], |image| image.section(Section::Slots));
        let mut entries = vec![[0u32; 3]; order.slots];
        for (slot, &new) in order.map.iter().enumerate() {
            if new == u32::MAX {
                continue;
            }
            entries[new as usize] = if slot < split {
                let at = slot * SLOT_BYTES;
                [
                    u32_at(table, at),
                    base_ordinal[u32_at(table, at + 4) as usize],
                    u32_at(table, at + 8),
                ]
            } else {
                let (owner, chunk) = owners[slot - split];
                ensure!(owner != FREE, "ordered slot without a document");
                [
                    DocumentStore::slot_word_count(store, slot - split),
                    owner,
                    chunk,
                ]
            };
        }
        for entry in entries {
            for value in entry {
                slots.write(&value.to_le_bytes())?;
            }
        }
    } else if let Some(image) = base {
        let table = image.section(Section::Slots);
        for slot in 0..image.slots {
            let at = slot * SLOT_BYTES;
            let owner = u32_at(table, at + 4);
            slots.write(&table[at..at + 4])?;
            let owner = if owner == FREE {
                FREE
            } else {
                base_ordinal[owner as usize]
            };
            slots.write(&owner.to_le_bytes())?;
            slots.write(&table[at + 8..at + 12])?;
        }
    }
    for (slot, &(owner, chunk)) in owners.iter().enumerate().filter(|_| order.is_none()) {
        slots.write(&DocumentStore::slot_word_count(store, slot).to_le_bytes())?;
        slots.write(&owner.to_le_bytes())?;
        slots.write(&chunk.to_le_bytes())?;
    }
    run[Section::Slots as usize] = Some(slots);
    run[Section::IdOffsets as usize] = Some(id_offsets);
    run[Section::Ids as usize] = Some(ids);
    run[Section::DocOffsets as usize] = Some(doc_offsets);
    run[Section::Docs as usize] = Some(docs);

    let spools: Vec<Spool> = run
        .into_iter()
        .map(|writer| match writer {
            Some(writer) => writer.finish_spool(),
            // The sections of compressed text: a fold writes inline text.
            None => BlockWriter::create_in(directory)?.finish_spool(),
        })
        .collect::<Result<_>>()?;
    #[cfg(test)]
    crate::core::storage::compaction_crash_point("fold-spooled");

    words[H_GENERATION] = u64::from(generation);
    words[H_DIMENSIONS] = dimensions as u64;
    // Slots written since the checkpoint follow its slots, so its reordered prefix stays one.
    (words[H_SLOTS], words[H_ORDERED]) = match order {
        Some(order) => (order.slots as u64, order.slots as u64),
        None => (
            (split + store.slot_count) as u64,
            base.map_or(store.ordered_slots.min(store.slot_count), |image| {
                image.ordered
            }) as u64,
        ),
    };
    words[H_CHUNKS] = (base.map_or(0, |image| image.chunks) + store.chunk_count) as u64;
    words[H_WORDS] = (base.map_or(0, |image| image.words) + store.word_count) as u64;
    words[H_DOCS] = (base_documents + store.documents.len()) as u64;
    let mut at = HEADER_LEN as u64;
    for (i, spool) in spools.iter().enumerate() {
        words[H_SECTIONS + 2 * i] = at;
        words[H_SECTIONS + 2 * i + 1] = spool.len();
        at += spool.len();
    }
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    for value in words {
        header.extend_from_slice(&value.to_le_bytes());
    }
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&header[super::CRC_START..]);
    for spool in &spools {
        spool.for_each_block(|block| {
            hasher.update(block);
            Ok(())
        })?;
    }
    header[8..16].copy_from_slice(&u64::from(hasher.finalize()).to_le_bytes());
    let mut file = BlockWriter::create_in(directory)?;
    file.write(&header)?;
    for spool in &spools {
        spool.for_each_block(|block| file.write(block))?;
    }
    let written = file.finish_synced(destination)?;
    #[cfg(test)]
    crate::core::storage::compaction_crash_point("fold-written");
    Ok(Some(written))
}

/// Whether the merged checkpoint holds word positions: `Some(true)` when every chunk of the
/// checkpoint and of the store has them, `Some(false)` when neither does, and `None` when
/// they differ (the merge would have to drop or invent positions).
fn positions_of(base: Option<&ServingImage>, store: &DocumentStore) -> Option<bool> {
    let from_base = base
        .filter(|image| image.chunks > 0)
        .map(ServingImage::has_positions);
    let from_store = (store.chunk_count > 0).then_some(store.nonpositional_chunks == 0);
    match (from_base, from_store) {
        (Some(a), Some(b)) if a != b => None,
        (a, b) => Some(a.or(b).unwrap_or(false)),
    }
}

/// Whether [`merge`] can write the checkpoint of `store` over `base`.
pub(crate) fn can_merge(base: Option<&ServingImage>, store: &DocumentStore) -> bool {
    positions_of(base, store).is_some()
        && super::representable(store)
        && base.is_none_or(ServingImage::mergeable)
}

/// Most (document, term) pairs the reorder's forward index holds (4 bytes each).
pub(crate) const REORDER_MAX_EDGES: usize = 1 << 28;

/// A slot order for [`merge`]: `map[s]` is the new slot of combined slot `s` (the checkpoint's
/// slots, then the live store's after them, as the merge numbers them), `u32::MAX` for a free
/// slot; `slots` is the count of occupied slots, numbered `0..slots`.
pub(crate) struct SlotOrder {
    map: Vec<u32>,
    slots: usize,
}

/// One term of the merged dictionaries: the checkpoint's list, the live store's, or both.
struct MergedTerm<'a> {
    term: Vec<u8>,
    base: Option<BaseTerm<'a>>,
    live: Option<&'a crate::core::documents::postings::PostingList>,
}

impl MergedTerm<'_> {
    fn count(&self) -> usize {
        self.base.as_ref().map_or(0, |b| b.count as usize) + self.live.map_or(0, |l| l.len())
    }
}

/// Calls `f` for every term of form `form` of `base` plus `store`, in term order.
fn for_each_merged_term<'a>(
    base: Option<&'a ServingImage>,
    form: usize,
    terms: &'a (
        Vec<u8>,
        Vec<u32>,
        Vec<&'a crate::core::documents::postings::PostingList>,
    ),
    mut f: impl FnMut(MergedTerm<'a>) -> Result<()>,
) -> Result<()> {
    let (bytes, offsets, lists) = terms;
    let mut live = (0..lists.len())
        .filter(|&i| !lists[i].is_empty())
        .peekable();
    let mut checkpoint = base.map(|image| BaseTerms::new(image, form));
    let mut pending = match checkpoint.as_mut() {
        Some(source) => source.next()?,
        None => None,
    };
    loop {
        let live_term = live
            .peek()
            .map(|&i| &bytes[offsets[i] as usize..offsets[i + 1] as usize]);
        let order = match (&pending, live_term) {
            (None, None) => break,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(b), Some(l)) => b.term.as_slice().cmp(l),
        };
        let from_base = (order != std::cmp::Ordering::Greater)
            .then(|| pending.take())
            .flatten();
        let from_live = (order != std::cmp::Ordering::Less)
            .then(|| live.next())
            .flatten();
        let term = match (&from_base, from_live) {
            (Some(b), _) => b.term.clone(),
            (None, Some(i)) => bytes[offsets[i] as usize..offsets[i + 1] as usize].to_vec(),
            (None, None) => unreachable!("a term from either side"),
        };
        f(MergedTerm {
            term,
            base: from_base,
            live: from_live.map(|i| lists[i]),
        })?;
        if order != std::cmp::Ordering::Greater {
            pending = match checkpoint.as_mut() {
                Some(source) => source.next()?,
                None => None,
            };
        }
    }
    Ok(())
}

/// Bytes [`bisection_order`] and the permuted [`merge`] hold beyond what a plain merge holds,
/// from the counts alone: the forward index (`pairs` 4-byte entries and two offset arrays), the
/// slot tables (owner, map, document slots, new word counts, the new slot table), the bisection's
/// per-document arrays, and the longest list's postings sorted by new slot.
fn order_estimate(documents: usize, slots: usize, pairs: usize, longest: usize) -> usize {
    pairs * 4 + documents * (2 * 8 + 4 + 4 + 16) + slots * (4 + 4 + 4 + 4 + 12) + longest * 24
}

/// The slot order recursive graph bisection gives the merge of `base` and `store` (see
/// `reorder`), built from the checkpoint's lists where they lie and the live store's lists,
/// without rebuilding the store: the forward index holds the terms of at least
/// `documents / 1024` postings (at most 4096), and of those, when they exceed
/// [`REORDER_MAX_EDGES`] pairs, the shortest that fit. Returns the estimate of the bytes it and
/// the permuted merge hold ([`order_estimate`]) and the order, or no order when the estimate is
/// over `budget` (nothing larger than the dictionaries is built before that is known) or the
/// merge has fewer than `min_documents` documents. A merged document's chunks keep ascending
/// slots.
pub(crate) fn bisection_order(
    base: Option<&ServingImage>,
    store: &DocumentStore,
    min_documents: usize,
    budget: Option<usize>,
) -> Result<(usize, Option<SlotOrder>)> {
    let split = base.map_or(0, |image| image.slots);
    let total = split + store.slot_count;
    let base_documents = base.map_or(0, |image| image.docs);
    let documents = base_documents + store.documents.len();
    if documents < min_documents {
        return Ok((0, None));
    }
    let min_len = (documents / 1024).clamp(2, 4096);
    let forms = [
        store.postings[0].sorted_terms(),
        store.postings[1].sorted_terms(),
    ];
    let mut chosen: Vec<MergedTerm<'_>> = Vec::new();
    let mut longest = 0usize;
    for (form, terms) in forms.iter().enumerate() {
        for_each_merged_term(base, form, terms, |term| {
            let count = term.count();
            longest = longest.max(count);
            if count >= min_len {
                chosen.push(term);
            }
            Ok(())
        })?;
    }
    let mut lengths: Vec<usize> = chosen.iter().map(MergedTerm::count).collect();
    lengths.sort_unstable();
    let mut max_len = usize::MAX;
    let mut pairs = 0usize;
    for &length in &lengths {
        if pairs + length > REORDER_MAX_EDGES {
            // Lists exactly as long as the one that overflows are left out with it.
            max_len = length - 1;
            break;
        }
        pairs += length;
    }
    chosen.retain(|term| term.count() <= max_len);
    let estimate = order_estimate(documents, total, pairs, longest);
    if budget.is_some_and(|budget| estimate > budget) {
        return Ok((estimate, None));
    }

    // Merged document ordinals, as the merge numbers the documents (id order), and each
    // combined slot's document.
    let mut owner = vec![u32::MAX; total];
    let mut base_ordinal = vec![0u32; base_documents];
    let mut live = store.documents.keys().peekable();
    let (mut next_base, mut ordinal) = (0usize, 0u32);
    loop {
        let from_base = match (next_base < base_documents, live.peek()) {
            (false, None) => break,
            (true, None) => true,
            (false, Some(_)) => false,
            (true, Some(id)) => base.expect("base documents exist").id(next_base) < id.as_str(),
        };
        if from_base {
            base_ordinal[next_base] = ordinal;
            next_base += 1;
        } else {
            let id = live.next().expect("peeked");
            for &slot in store.slots.get(id).context("document without slots")? {
                owner[split + slot] = ordinal;
            }
        }
        ordinal += 1;
    }
    if let Some(image) = base {
        let table = image.section(Section::Slots);
        for (slot, owner) in owner.iter_mut().enumerate().take(split) {
            let document = u32_at(table, slot * SLOT_BYTES + 4);
            if document != FREE {
                *owner = base_ordinal[document as usize];
            }
        }
    }
    drop(base_ordinal);

    // The forward index: per document, the chosen terms it holds, sorted, each once.
    let mut offsets = vec![0usize; documents + 1];
    let visit = |term: &MergedTerm<'_>, f: &mut dyn FnMut(u32)| {
        if let Some(b) = &term.base {
            let mut cursor = Cursor::open(ListRef {
                bytes: b.bytes,
                count: b.count,
                skips: b.skips,
                positions: b.positions,
            });
            while cursor.doc() != NO_DOC {
                f(owner[cursor.doc() as usize]);
                cursor.next();
            }
        }
        if let Some(list) = term.live {
            list.for_each(|slot, _| f(owner[split + slot]));
        }
    };
    let mut read = 0usize;
    let mut release = |term: &MergedTerm<'_>| {
        read += term.base.as_ref().map_or(0, |b| b.bytes.len());
        if read >= RELEASE_EVERY {
            read = 0;
            if let Some(image) = base {
                image.release_pages();
            }
        }
    };
    for term in &chosen {
        visit(term, &mut |document| offsets[document as usize + 1] += 1);
        release(term);
    }
    for at in 1..offsets.len() {
        offsets[at] += offsets[at - 1];
    }
    let mut fill = offsets.clone();
    let mut terms = vec![0u32; offsets[documents]];
    for (id, term) in chosen.iter().enumerate() {
        visit(term, &mut |document| {
            terms[fill[document as usize]] = id as u32;
            fill[document as usize] += 1;
        });
        release(term);
    }
    drop(fill);
    let term_count = chosen.len();
    drop(chosen);
    let mut write = 0;
    for document in 0..documents {
        let (start, end) = (offsets[document], offsets[document + 1]);
        offsets[document] = write;
        terms[start..end].sort_unstable();
        let mut previous = None;
        for at in start..end {
            let term = terms[at];
            if previous != Some(term) {
                terms[write] = term;
                write += 1;
                previous = Some(term);
            }
        }
    }
    offsets[documents] = write;
    terms.truncate(write);
    let order =
        crate::core::documents::reorder::order(&crate::core::documents::reorder::ForwardIndex {
            offsets,
            terms,
            term_count,
        });

    // Each document's slots, ascending (its chunks in order), then the map in bisection order.
    let mut starts = vec![0u32; documents + 1];
    for &document in &owner {
        if document != u32::MAX {
            starts[document as usize + 1] += 1;
        }
    }
    for at in 1..starts.len() {
        starts[at] += starts[at - 1];
    }
    let mut cursor = starts.clone();
    let mut slots_of = vec![0u32; starts[documents] as usize];
    for (slot, &document) in owner.iter().enumerate() {
        if document != u32::MAX {
            slots_of[cursor[document as usize] as usize] = slot as u32;
            cursor[document as usize] += 1;
        }
    }
    drop(cursor);
    drop(owner);
    let mut map = vec![u32::MAX; total];
    let mut next = 0u32;
    for &document in &order {
        let document = document as usize;
        for &slot in &slots_of[starts[document] as usize..starts[document + 1] as usize] {
            map[slot as usize] = next;
            next += 1;
        }
    }
    Ok((
        estimate,
        Some(SlotOrder {
            map,
            slots: next as usize,
        }),
    ))
}

/// [`merge_form`] with every posting moved to its slot in `order` (`words` by new slot): each
/// list is unpacked, its postings sorted by new slot with their positions, and packed again.
fn merge_form_ordered(
    base: Option<&ServingImage>,
    store: &DocumentStore,
    form: usize,
    split: usize,
    order: &SlotOrder,
    words: &dyn Fn(usize) -> u32,
    out: &mut FormWriter,
    streams: &mut Streams<'_>,
) -> Result<Option<()>> {
    let terms = store.postings[form].sorted_terms();
    let positional = streams.positional;
    let mut packer = CheckpointPacker::with_info(true, words, true);
    let mut placed = PositionPacker::new();
    let (mut scratch, mut skip_scratch) = (Vec::new(), Vec::new());
    let mut moved: Vec<(u32, u32, &[u8])> = Vec::new();
    let mut read = 0usize;
    let mut fits = true;
    for_each_merged_term(base, form, &terms, |term| {
        if !fits {
            return Ok(());
        }
        moved.clear();
        if let Some(b) = &term.base {
            let mut cursor = Cursor::open(ListRef {
                bytes: b.bytes,
                count: b.count,
                skips: b.skips,
                positions: b.positions,
            });
            while cursor.doc() != NO_DOC {
                let slot = order.map[cursor.doc() as usize];
                let frequency = cursor.frequency();
                let bytes = if positional {
                    cursor.position_bytes()
                } else {
                    &[][..]
                };
                moved.push((slot, frequency, bytes));
                cursor.next();
            }
            read += b.bytes.len() + b.skips.len() + b.positions.len();
        }
        if let Some(list) = term.live {
            streams.positive &= list.positive_len() == list.len();
            let mut failure = false;
            list.for_each_with_ordinal(|slot, frequency, ordinal| {
                let bytes = if positional {
                    match store
                        .position_source(slot)
                        .filter(|_| ordinal > 0)
                        .map(|source| source.bytes(ordinal as usize - 1))
                    {
                        Some(bytes) => bytes,
                        None => {
                            failure = true;
                            &[][..]
                        }
                    }
                } else {
                    &[][..]
                };
                moved.push((order.map[split + slot], frequency, bytes));
            });
            ensure!(!failure, "positional posting lacks its positions");
        }
        moved.sort_unstable_by_key(|posting| posting.0);
        scratch.clear();
        skip_scratch.clear();
        placed.reset();
        for &(slot, frequency, bytes) in &moved {
            packer.push(slot as usize, frequency, &mut scratch);
            if positional {
                placed.push(bytes);
            }
        }
        packer.finish(&mut scratch);
        let infos: Vec<BlockInfo> = packer.take_infos();
        let count = moved.len() as u64;
        if !placed.finish()
            || u32::try_from(scratch.len()).is_err()
            || !append_skips(&infos, &placed.offsets, &scratch, count, &mut skip_scratch)
        {
            fits = false;
            return Ok(());
        }
        let at = streams.lens();
        streams.postings.write(&scratch)?;
        streams.skips.write(&skip_scratch)?;
        streams.positions.write(&placed.out)?;
        if out.push(&term.term, count, at, &streams.lens())?.is_none() {
            fits = false;
        }
        if read >= RELEASE_EVERY {
            read = 0;
            if let Some(image) = base {
                image.release_pages();
            }
        }
        Ok(())
    })?;
    Ok(fits.then_some(()))
}
