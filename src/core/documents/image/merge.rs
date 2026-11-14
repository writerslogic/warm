// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Writes the checkpoint of a checkpoint plus the live store written since, or of a live store
//! alone, as a stream: term dictionaries merge in term order, the checkpoint's documents and
//! posting lists of terms the live store lacks are copied as bytes, and every section goes to a
//! block-addressed run ([`BlockWriter`]) instead of a buffer. Nothing larger than the live
//! store, one posting list and one document record is held at once, however large the
//! checkpoint is. The result is byte-for-byte the checkpoint [`super::encode`] writes for the
//! store holding both sets of documents (`tests::merged_checkpoint_equals_the_whole_encoding`).
//! The merge needs a version 2 checkpoint with no dead documents, so it applies to a store
//! written once and extended by inserts; anything else keeps its in-memory path.

use std::path::Path;

use anyhow::{ensure, Context, Result};

use super::{
    encode_document, put_bytes, put_varint, DocumentStore, Reader, ServingImage, BLOCK, FREE,
    H_CHUNKS, H_DIMENSIONS, H_DOCS, H_GENERATION, H_SECTIONS, H_SLOTS, H_TERMS, H_WORDS,
    HEADER_LEN, HEADER_WORDS, MAGIC, SECTIONS, SLOT_BYTES,
};
use super::{Section, u32_at};
use crate::core::documents::postings::{CheckpointPacker, Postings};
use crate::core::durable_file::{BlockWriter, Spool};

/// Pages of the mapped checkpoint are released after this many bytes of it were read.
const RELEASE_EVERY: usize = 4 << 20;

/// One term of the checkpoint's dictionary with its posting bytes.
struct BaseTerm<'a> {
    term: Vec<u8>,
    count: u32,
    bytes: &'a [u8],
}

/// Sequential reader of one form's dictionary and posting bytes in the checkpoint.
struct BaseTerms<'a> {
    reader: Reader<'a>,
    postings: &'a [u8],
    at: usize,
    left: usize,
    index: usize,
    previous: Vec<u8>,
}

impl<'a> BaseTerms<'a> {
    fn new(image: &'a ServingImage, form: usize) -> Self {
        let (dict, blocks) = if form == 0 {
            (Section::Dict0, Section::Blocks0)
        } else {
            (Section::Dict1, Section::Blocks1)
        };
        let blocks = image.section(blocks);
        let at = if blocks.is_empty() {
            0
        } else {
            u32_at(blocks, 4) as usize
        };
        Self {
            reader: Reader::new(image.section(dict)),
            postings: image.section(Section::Postings),
            at,
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
        let bytes = self
            .postings
            .get(self.at..self.at + len)
            .context("checkpoint posting range")?;
        self.at += len;
        Ok(Some(BaseTerm {
            term: self.previous.clone(),
            count,
            bytes,
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

    /// Appends one term whose postings were just written to `postings` at `start`. `None` when
    /// an offset no longer fits the format's `u32`.
    fn push(
        &mut self,
        term: &[u8],
        count: u64,
        start: u64,
        postings: &BlockWriter,
    ) -> Result<Option<()>> {
        let prefix = if self.terms.is_multiple_of(BLOCK as u64) {
            let (Ok(at), Ok(posting)) = (u32::try_from(self.dict.len()), u32::try_from(start))
            else {
                return Ok(None);
            };
            self.blocks.write(&at.to_le_bytes())?;
            self.blocks.write(&posting.to_le_bytes())?;
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
        put_varint(&mut self.entry, postings.len() - start);
        self.dict.write(&self.entry)?;
        self.previous.clear();
        self.previous.extend_from_slice(term);
        self.terms += 1;
        Ok(Some(()))
    }
}

/// Merges one form's terms. `None` when an offset exceeds the format.
fn merge_form(
    base: Option<&ServingImage>,
    store: &DocumentStore,
    form: usize,
    split: usize,
    out: &mut FormWriter,
    postings: &mut BlockWriter,
) -> Result<Option<()>> {
    let (terms, offsets, lists) = store.postings[form].sorted_terms();
    let mut live = (0..lists.len()).filter(|&i| !lists[i].is_empty()).peekable();
    let mut checkpoint = base.map(|image| BaseTerms::new(image, form));
    let mut pending = match checkpoint.as_mut() {
        Some(source) => source.next()?,
        None => None,
    };
    let mut scratch = Vec::new();
    let mut read = 0usize;
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
        let start = postings.len();
        let (term, count) = match order {
            std::cmp::Ordering::Less => {
                let b = pending.take().expect("ordered by a checkpoint term");
                postings.write(b.bytes)?;
                read += b.bytes.len();
                (b.term, u64::from(b.count))
            }
            std::cmp::Ordering::Greater => {
                let i = live.next().expect("ordered by a live term");
                scratch.clear();
                let mut packer = CheckpointPacker::new();
                lists[i].for_each(|slot, frequency| {
                    packer.push(slot + split, frequency, &mut scratch);
                });
                packer.finish(&mut scratch);
                postings.write(&scratch)?;
                (
                    terms[offsets[i] as usize..offsets[i + 1] as usize].to_vec(),
                    lists[i].len() as u64,
                )
            }
            std::cmp::Ordering::Equal => {
                let b = pending.take().expect("equal terms");
                let i = live.next().expect("equal terms");
                scratch.clear();
                let mut packer = CheckpointPacker::new();
                Postings {
                    bytes: b.bytes,
                    count: b.count,
                    blocked: true,
                }
                .for_each(|slot, frequency| packer.push(slot, frequency, &mut scratch));
                lists[i].for_each(|slot, frequency| {
                    packer.push(slot + split, frequency, &mut scratch);
                });
                packer.finish(&mut scratch);
                postings.write(&scratch)?;
                read += b.bytes.len();
                (b.term, u64::from(b.count) + lists[i].len() as u64)
            }
        };
        if out.push(&term, count, start, postings)?.is_none() {
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
/// checkpoint layout or cannot be merged (a version 1 checkpoint, or one with dead documents).
pub(crate) fn merge(
    base: Option<&ServingImage>,
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
    destination: &Path,
) -> Result<Option<u64>> {
    if !super::representable(store) {
        return Ok(None);
    }
    if base.is_some_and(|image| !image.blocked || image.dimensions != dimensions) {
        return Ok(None);
    }
    let directory = destination.parent().unwrap_or_else(|| Path::new("."));
    let split = base.map_or(0, |image| image.slots);
    let mut words = [0u64; HEADER_WORDS];
    let mut run: Vec<Option<BlockWriter>> = (0..SECTIONS).map(|_| None).collect();
    let mut postings = BlockWriter::create_in(directory)?;
    for form in 0..2 {
        let mut out = FormWriter::new(directory)?;
        if merge_form(base, store, form, split, &mut out, &mut postings)?.is_none() {
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
    if u32::try_from(postings.len()).is_err() {
        return Ok(None);
    }
    run[Section::Postings as usize] = Some(postings);
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
        let (Ok(id_at), Ok(doc_at)) = (u32::try_from(ids.len()), u32::try_from(docs.len()))
        else {
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
            encode_document(&mut record, document, segment.as_deref())?;
            docs.write(&record)?;
        }
        ordinal = ordinal.checked_add(1).context("checkpoint document count")?;
    }
    let (Ok(id_end), Ok(doc_end)) = (u32::try_from(ids.len()), u32::try_from(docs.len())) else {
        return Ok(None);
    };
    id_offsets.write(&id_end.to_le_bytes())?;
    doc_offsets.write(&doc_end.to_le_bytes())?;

    // Slots: the checkpoint's, owners renumbered, then the live store's after them.
    let mut slots = BlockWriter::create_in(directory)?;
    if let Some(image) = base {
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
    for (slot, &(owner, chunk)) in owners.iter().enumerate() {
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
        .map(|writer| writer.expect("every section was written").finish_spool())
        .collect::<Result<_>>()?;
    #[cfg(test)]
    crate::core::storage::compaction_crash_point("fold-spooled");

    words[H_GENERATION] = u64::from(generation);
    words[H_DIMENSIONS] = dimensions as u64;
    words[H_SLOTS] = (split + store.slot_count) as u64;
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
