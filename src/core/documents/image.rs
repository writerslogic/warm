// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! The document checkpoint (`documents.ckpt`, see `docs/file-format/CHECKPOINT.md`): the whole
//! document store of a journal generation as the bytes a query reads. A generation holding one
//! opens by checking and mapping it instead of replaying document frames, ranks unfiltered
//! lexical queries straight from the mapping with the live store's code ([`LexicalSource`]),
//! and rebuilds the live store from it ([`ServingImage::hydrate`]) when anything else is asked.
//!
//! The posting lists hold the live lists' postings as varints and every count, word count and slot
//! (holes included) is the live store's, so ranking from a checkpoint is ranking from the store
//! that wrote it. Layout, little endian: an 8-byte magic, `HEADER_WORDS` `u64` header words (a
//! CRC32 of everything after the CRC word, the CRC32 of the generation directory's name, the
//! vector dimensions, the store's counts, then an `(offset, length)` pair per [`Section`]),
//! then the sections:
//!
//! - per posting form, a dictionary of the sorted terms in blocks of [`BLOCK`] front-coded
//!   entries (`prefix`, `suffix length`, suffix bytes, posting count, posting bytes; varints),
//!   and a block table of `u32` pairs: the block's dictionary offset and the posting offset of
//!   its first term;
//! - the posting bytes of every term, form 0 then form 1, in dictionary order;
//! - per slot, `u32` word count, document ordinal (`u32::MAX` for a free slot) and chunk index;
//! - document ids in id order (ordinal order): `docs + 1` `u32` offsets, then the UTF-8 bytes;
//! - per document, `docs + 1` `u32` offsets and a record: version, source URI, metadata JSON,
//!   and per chunk its flags (own id, stemmed terms), id, byte span and text (none or inline).
//!
//! A checkpoint holds lexical-only chunks (no vector, embedding or pending encoding), so the
//! chunk fields it leaves out are the same for every chunk it holds.

use std::sync::Arc;

use anyhow::{bail, ensure, Context, Result};

use super::binary::{put_bytes, put_varint, Reader};
use super::index::{DocumentStore, StoreIdentity};
use super::lists::{Cursor, ListRef};
use super::packed_terms::StoredTerms;
use super::postings::{PostingList, PostingLists, Postings, RestoredList};
use super::search::LexicalSource;
use super::text::TextSegment;
use super::{StoredChunk, StoredDocument, VectorState};
use crate::core::entangled::EntangledHVec;

/// The magic of the version written: version 3's postings plus per-list skip tables and word
/// positions beside the postings.
pub(crate) const MAGIC: &[u8; 8] = b"HMSCKP\x00\x04";
/// Version 3, still read: block-encoded postings with per-list and per-block impacts.
const MAGIC_V3: &[u8; 8] = b"HMSCKP\x00\x03";
/// Version 2, still read: block-encoded postings without impacts.
const MAGIC_V2: &[u8; 8] = b"HMSCKP\x00\x02";
/// Version 1, still read: flat varint postings.
const MAGIC_V1: &[u8; 8] = b"HMSCKP\x00\x01";

/// Whether `magic` opens a plaintext checkpoint of a version this build reads.
pub(crate) fn known_magic(magic: &[u8]) -> bool {
    magic == MAGIC || magic == MAGIC_V3 || magic == MAGIC_V2 || magic == MAGIC_V1
}
const HEADER_WORDS: usize = 40;
pub(crate) const HEADER_LEN: usize = 8 + 8 * HEADER_WORDS;
/// Dictionary entries per block: one binary search over blocks, then at most this many
/// entries decoded per term looked up.
const BLOCK: usize = 32;

const H_CRC: usize = 0;
/// Version 4 flags: bit 0, every chunk holds word positions.
const H_FLAGS: usize = 3;
const FLAG_POSITIONAL: u64 = 1;
/// Version 4 flags: bit 1, no posting has frequency zero.
const FLAG_POSITIVE: u64 = 2;
const H_GENERATION: usize = 1;
const H_DIMENSIONS: usize = 2;
const H_SLOTS: usize = 8;
const H_CHUNKS: usize = 9;
const H_WORDS: usize = 10;
const H_DOCS: usize = 11;
const H_TERMS: usize = 12; // and 13
const H_SECTIONS: usize = 14;

#[derive(Clone, Copy)]
enum Section {
    Dict0,
    Blocks0,
    Dict1,
    Blocks1,
    Postings,
    Slots,
    IdOffsets,
    Ids,
    DocOffsets,
    Docs,
    Skips,
    Positions,
}
const SECTIONS: usize = 12;
/// Sections of versions 1 to 3; version 4 adds the skip tables and the positions.
const SECTIONS_V3: usize = 10;
const _: () = assert!(H_SECTIONS + 2 * SECTIONS <= HEADER_WORDS);

pub(super) const SLOT_BYTES: usize = 12;
const FREE: u32 = u32::MAX;

const TEXT_NONE: u8 = 0;
const TEXT_INLINE: u8 = 2;
/// Chunk flags: the chunk id is stored (it is not the default id for its index); the chunk's
/// terms are stemmed (its postings are in form 1).
const OWN_ID: u8 = 1;
const STEMMED: u8 = 2;
/// Terms from a store analyzer other than the legacy one (see `StoredChunk::analyzed`).
const ANALYZED: u8 = 4;
/// Version 4: the chunk's positions are a whole article's (`positions::DOCUMENT_VERSION`).
const WHOLE: u8 = 8;

fn word(bytes: &[u8], index: usize) -> u64 {
    let at = 8 + 8 * index;
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"))
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

/// The generation-name CRC32 and the vector dimensions a checkpoint header records (`header`
/// holds at least the magic and the first three header words).
pub(crate) fn binding(header: &[u8]) -> (u32, u64) {
    (
        word(header, H_GENERATION) as u32,
        word(header, H_DIMENSIONS),
    )
}

/// The CRC a checkpoint header records: over everything after the CRC word.
pub(crate) fn recorded_crc(header: &[u8]) -> u32 {
    word(header, H_CRC) as u32
}

/// Offset the checkpoint CRC starts at.
pub(crate) const CRC_START: usize = 16;

/// Whether every chunk of `store` is lexical-only and either every chunk or none holds word
/// positions, the only chunks a checkpoint holds.
pub(crate) fn representable(store: &DocumentStore) -> bool {
    (store.nonpositional_chunks == store.chunk_count || store.nonpositional_chunks == 0)
        && store.documents.values().all(|document| {
            !document.vector_record
                && document.chunks.iter().all(|chunk| {
                    chunk.vector_state == VectorState::Lexical
                        && chunk.embedding.is_none()
                        && chunk.vector.indices.is_empty()
                })
        })
}

/// Encodes `store` as the checkpoint of the generation whose name has CRC32 `generation`;
/// `None` when a chunk is not lexical-only or an offset would not fit the `u32` fields.
pub(crate) fn encode(
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
) -> Result<Option<Vec<u8>>> {
    encode_as(store, generation, dimensions, 4)
}

/// [`encode`] in version 1, for tests of reading it.
#[cfg(test)]
pub(crate) fn encode_v1(
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
) -> Result<Option<Vec<u8>>> {
    encode_as(store, generation, dimensions, 1)
}

/// [`encode`] in version 2, for tests of reading it.
#[cfg(test)]
pub(crate) fn encode_v2(
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
) -> Result<Option<Vec<u8>>> {
    encode_as(store, generation, dimensions, 2)
}

/// [`encode`] in version 3, for tests of reading it.
#[cfg(test)]
pub(crate) fn encode_v3(
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
) -> Result<Option<Vec<u8>>> {
    encode_as(store, generation, dimensions, 3)
}

/// Appends the positions of one block of postings, `(slot, ordinal)` each, to `out`; `false`
/// when the block would start beyond what a `u32` offset from `base` holds.
fn pack_position_block(
    store: &DocumentStore,
    block: &mut Vec<(usize, u32)>,
    out: &mut Vec<u8>,
    base: usize,
    offsets: &mut Vec<u32>,
) -> Result<bool> {
    let Ok(at) = u32::try_from(out.len() - base) else {
        return Ok(false);
    };
    offsets.push(at);
    let mut lengths = Vec::with_capacity(block.len());
    let mut sources = Vec::with_capacity(block.len());
    for &(slot, ordinal) in block.iter() {
        ensure!(ordinal > 0, "positional posting lacks its term ordinal");
        let positions = store
            .position_source(slot)
            .context("positional posting in a chunk without positions")?;
        let bytes = positions.bytes(ordinal as usize - 1);
        lengths.push(bytes.len() as u64);
        sources.push(bytes);
    }
    let longest = lengths.iter().copied().max().unwrap_or(1).max(1);
    let width = (64 - longest.leading_zeros()) as u8;
    out.push(width);
    super::postings::pack(out, lengths.iter().copied(), width);
    for bytes in sources {
        out.extend_from_slice(bytes);
    }
    block.clear();
    Ok(true)
}

/// Appends the positions of `list`'s postings to `out`, block by block as `lists::Cursor` reads
/// them, and records where each block starts (from the first byte this call appends) in
/// `offsets`. `false` when an offset would not fit a `u32`.
fn pack_positions(
    store: &DocumentStore,
    list: &PostingList,
    out: &mut Vec<u8>,
    offsets: &mut Vec<u32>,
) -> Result<bool> {
    let base = out.len();
    offsets.clear();
    let mut block: Vec<(usize, u32)> = Vec::with_capacity(super::postings::CHECKPOINT_BLOCK);
    let mut outcome: Result<bool> = Ok(true);
    list.for_each_with_ordinal(|slot, _, ordinal| {
        if !matches!(outcome, Ok(true)) {
            return;
        }
        block.push((slot, ordinal));
        if block.len() == super::postings::CHECKPOINT_BLOCK {
            outcome = pack_position_block(store, &mut block, out, base, offsets);
        }
    });
    if matches!(outcome, Ok(true)) && !block.is_empty() {
        outcome = pack_position_block(store, &mut block, out, base, offsets);
    }
    Ok(outcome? && u32::try_from(out.len() - base).is_ok())
}

/// Appends a list's frontier directory and frontier bytes (see `lists`) after its skip arrays;
/// false when the frontier bytes outgrow the `u32` offsets.
fn pack_frontiers(infos: &[super::postings::BlockInfo], skips: &mut Vec<u8>) -> bool {
    use super::lists::RUN_BLOCKS;
    let runs = infos.len().div_ceil(RUN_BLOCKS);
    let mut data: Vec<u8> = Vec::new();
    let mut directory: Vec<u32> = Vec::with_capacity(infos.len() + runs + 1);
    let put = |data: &mut Vec<u8>, points: &[(u32, u32)]| {
        put_varint(data, points.len() as u64);
        for &(frequency, words) in points {
            put_varint(data, u64::from(frequency));
            put_varint(data, u64::from(words));
        }
    };
    for info in infos {
        let Ok(at) = u32::try_from(data.len()) else {
            return false;
        };
        directory.push(at);
        put(&mut data, info.frontier.as_deref().unwrap_or(&[]));
    }
    for run in infos.chunks(RUN_BLOCKS) {
        let Ok(at) = u32::try_from(data.len()) else {
            return false;
        };
        directory.push(at);
        let points: Vec<(u32, u32)> = run
            .iter()
            .flat_map(|info| match info.frontier.as_deref() {
                Some(points) => points.to_vec(),
                None => vec![(info.max_frequency, info.min_words)],
            })
            .collect();
        put(&mut data, &super::postings::pareto(points));
    }
    let Ok(end) = u32::try_from(data.len()) else {
        return false;
    };
    directory.push(end);
    for at in directory {
        skips.extend_from_slice(&at.to_le_bytes());
    }
    skips.extend_from_slice(&data);
    true
}

/// [`encode`] with a skip table and word positions per list (version 4), impact-carrying block
/// postings (version 3), plain block postings (version 2) or flat ones (version 1).
fn encode_as(
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
    version: u8,
) -> Result<Option<Vec<u8>>> {
    let (blocked, impacts, extended) = (version >= 2, version >= 3, version >= 4);
    if !representable(store) {
        return Ok(None);
    }
    let positional = store.chunk_count > 0 && store.nonpositional_chunks == 0;
    if positional && !extended {
        return Ok(None);
    }
    let mut words = [0u64; HEADER_WORDS];
    if positional {
        words[H_FLAGS] |= FLAG_POSITIONAL;
    }
    let mut sections: Vec<Vec<u8>> = vec![Vec::new(); SECTIONS];
    let mut postings = Vec::new();
    let (mut skips, mut positions) = (Vec::new(), Vec::new());
    let mut positive = true;
    let mut infos: Vec<super::postings::BlockInfo> = Vec::new();
    let mut offsets: Vec<u32> = Vec::new();
    for form in 0..2 {
        let (terms, term_offsets, lists) = store.postings[form].sorted_terms();
        let (mut dict, mut blocks) = (Vec::new(), Vec::new());
        let mut count = 0u64;
        for (i, list) in lists.iter().enumerate() {
            if list.is_empty() {
                continue;
            }
            positive &= list.positive_len() == list.len();
            let term = &terms[term_offsets[i] as usize..term_offsets[i + 1] as usize];
            let prefix = if (count as usize).is_multiple_of(BLOCK) {
                if extended {
                    for value in [dict.len(), postings.len(), skips.len(), positions.len()] {
                        blocks.extend_from_slice(&(value as u64).to_le_bytes());
                    }
                } else {
                    let (Ok(at), Ok(posting)) =
                        (u32::try_from(dict.len()), u32::try_from(postings.len()))
                    else {
                        return Ok(None);
                    };
                    blocks.extend_from_slice(&at.to_le_bytes());
                    blocks.extend_from_slice(&posting.to_le_bytes());
                }
                0
            } else {
                let previous = previous_term(&terms, &term_offsets, &lists, i);
                previous
                    .iter()
                    .zip(term)
                    .take_while(|(a, b)| a == b)
                    .count()
            };
            put_varint(&mut dict, prefix as u64);
            put_bytes(&mut dict, &term[prefix..]);
            let (start, positions_start, skips_start) =
                (postings.len(), positions.len(), skips.len());
            if blocked {
                let slot_words = |slot: usize| DocumentStore::slot_word_count(store, slot);
                infos.clear();
                super::postings::pack_checkpoint(
                    list,
                    &slot_words,
                    impacts,
                    &mut postings,
                    extended.then_some(&mut infos),
                );
                if extended {
                    if u32::try_from(postings.len() - start).is_err() {
                        return Ok(None);
                    }
                    if positional && !pack_positions(store, list, &mut positions, &mut offsets)? {
                        return Ok(None);
                    }
                    if infos.len() >= 2 {
                        for field in 0..5 {
                            for (block, info) in infos.iter().enumerate() {
                                let value = match field {
                                    0 => info.last,
                                    1 => info.offset,
                                    2 => offsets.get(block).copied().unwrap_or(0),
                                    3 => info.max_frequency,
                                    _ => info.min_words,
                                };
                                skips.extend_from_slice(&value.to_le_bytes());
                            }
                        }
                        if !pack_frontiers(&infos, &mut skips) {
                            return Ok(None);
                        }
                    }
                }
            } else {
                #[cfg(test)]
                super::postings::pack_checkpoint_flat(list, &mut postings);
            }
            put_varint(&mut dict, list.len() as u64);
            put_varint(&mut dict, (postings.len() - start) as u64);
            if extended {
                put_varint(&mut dict, (positions.len() - positions_start) as u64);
                put_varint(&mut dict, (skips.len() - skips_start) as u64);
            }
            count += 1;
        }
        words[H_TERMS + form] = count;
        let (d, b) = if form == 0 {
            (Section::Dict0, Section::Blocks0)
        } else {
            (Section::Dict1, Section::Blocks1)
        };
        sections[d as usize] = dict;
        sections[b as usize] = blocks;
    }
    if !extended && u32::try_from(postings.len()).is_err() {
        return Ok(None);
    }
    sections[Section::Postings as usize] = postings;
    if positive && extended {
        words[H_FLAGS] |= FLAG_POSITIVE;
    }
    sections[Section::Skips as usize] = skips;
    sections[Section::Positions as usize] = positions;

    let segment = store.text_segment();
    let mut slots = Vec::with_capacity(store.slot_count * SLOT_BYTES);
    let mut owners = vec![(FREE, 0u32); store.slot_count];
    let (mut ids, mut id_offsets) = (Vec::new(), Vec::new());
    let (mut docs, mut doc_offsets) = (Vec::new(), Vec::new());
    for (ordinal, (id, document)) in store.documents.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).context("checkpoint document count")?;
        let document_slots = store.slots.get(id).context("document without slots")?;
        ensure!(
            document_slots.len() == document.chunks.len(),
            "document slots and chunks disagree"
        );
        for (chunk, &slot) in document_slots.iter().enumerate() {
            owners[slot] = (ordinal, u32::try_from(chunk).context("chunk index")?);
        }
        let (Ok(id_at), Ok(doc_at)) = (u32::try_from(ids.len()), u32::try_from(docs.len())) else {
            return Ok(None);
        };
        id_offsets.extend_from_slice(&id_at.to_le_bytes());
        ids.extend_from_slice(id.as_bytes());
        doc_offsets.extend_from_slice(&doc_at.to_le_bytes());
        encode_document(&mut docs, document, segment.as_deref())?;
    }
    let (Ok(id_end), Ok(doc_end)) = (u32::try_from(ids.len()), u32::try_from(docs.len())) else {
        return Ok(None);
    };
    id_offsets.extend_from_slice(&id_end.to_le_bytes());
    doc_offsets.extend_from_slice(&doc_end.to_le_bytes());
    for (slot, &(ordinal, chunk)) in owners.iter().enumerate() {
        slots.extend_from_slice(&DocumentStore::slot_word_count(store, slot).to_le_bytes());
        slots.extend_from_slice(&ordinal.to_le_bytes());
        slots.extend_from_slice(&chunk.to_le_bytes());
    }
    sections[Section::Slots as usize] = slots;
    sections[Section::IdOffsets as usize] = id_offsets;
    sections[Section::Ids as usize] = ids;
    sections[Section::DocOffsets as usize] = doc_offsets;
    sections[Section::Docs as usize] = docs;

    words[H_GENERATION] = u64::from(generation);
    words[H_DIMENSIONS] = dimensions as u64;
    words[H_SLOTS] = store.slot_count as u64;
    words[H_CHUNKS] = store.chunk_count as u64;
    words[H_WORDS] = store.word_count as u64;
    words[H_DOCS] = store.documents.len() as u64;
    let section_count = if extended { SECTIONS } else { SECTIONS_V3 };
    let mut at = HEADER_LEN as u64;
    for (i, section) in sections.iter().take(section_count).enumerate() {
        words[H_SECTIONS + 2 * i] = at;
        words[H_SECTIONS + 2 * i + 1] = section.len() as u64;
        at += section.len() as u64;
    }
    let mut out = Vec::with_capacity(at as usize);
    out.extend_from_slice(match version {
        4 => MAGIC,
        3 => MAGIC_V3,
        2 => MAGIC_V2,
        _ => MAGIC_V1,
    });
    for value in words {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for section in sections.iter().take(section_count) {
        out.extend_from_slice(section);
    }
    let crc = crc32fast::hash(&out[CRC_START..]);
    out[8..16].copy_from_slice(&u64::from(crc).to_le_bytes());
    Ok(Some(out))
}

/// The term before `lists[i]` among those with postings (front coding skips empty lists).
fn previous_term<'a>(
    terms: &'a [u8],
    offsets: &[u32],
    lists: &[&PostingList],
    i: usize,
) -> &'a [u8] {
    let j = (0..i)
        .rev()
        .find(|&j| !lists[j].is_empty())
        .expect("a block's later entry follows a written one");
    &terms[offsets[j] as usize..offsets[j + 1] as usize]
}

fn encode_document(
    out: &mut Vec<u8>,
    document: &StoredDocument,
    segment: Option<&TextSegment>,
) -> Result<()> {
    put_bytes(out, document.version.as_bytes());
    match &document.source_uri {
        Some(uri) => {
            out.push(1);
            put_bytes(out, uri.as_bytes());
        }
        None => out.push(0),
    }
    put_bytes(out, &serde_json::to_vec(&document.metadata)?);
    put_varint(out, document.chunks.len() as u64);
    for (index, chunk) in document.chunks.iter().enumerate() {
        let own_id = !super::is_chunk_id(&chunk.id, &document.id, index);
        out.push(
            if own_id { OWN_ID } else { 0 }
                | if chunk.stemmed { STEMMED } else { 0 }
                | if chunk.analyzed { ANALYZED } else { 0 }
                | if chunk.terms.is_whole_document() {
                    WHOLE
                } else {
                    0
                },
        );
        if own_id {
            put_bytes(out, chunk.id.as_bytes());
        }
        put_varint(out, u64::from(chunk.start_byte));
        put_varint(out, u64::from(chunk.end_byte));
        match super::index::try_text_of(chunk, segment)? {
            Some(text) => {
                out.push(TEXT_INLINE);
                put_bytes(out, text.as_bytes());
            }
            None => out.push(TEXT_NONE),
        }
    }
    Ok(())
}

/// Bytes a checkpoint is read from: its file mapped, or (encrypted stores) decrypted in memory.
pub(crate) enum Backing {
    Mapped(memmap2::Mmap),
    #[cfg_attr(not(feature = "security"), allow(dead_code))]
    Owned(Vec<u8>),
}

impl std::ops::Deref for Backing {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Mapped(map) => map,
            Self::Owned(bytes) => bytes,
        }
    }
}

/// Where a block of dictionary entries starts in the dictionary, posting, skip and positions
/// sections.
#[derive(Clone, Copy)]
struct BlockBase {
    dict: usize,
    posting: usize,
    skip: usize,
    pos: usize,
}

/// A dictionary entry's counts, after its term.
struct Entry {
    count: u64,
    len: usize,
    pos_len: usize,
    skip_len: usize,
}

/// One posting form's dictionary.
#[derive(Clone, Copy, Default)]
struct Dictionary {
    dict: (usize, usize),
    blocks: (usize, usize),
    terms: usize,
}

/// A validated checkpoint. The CRC was checked by whoever produced `bytes`; construction
/// checks every section, dictionary entry, slot and offset it will later read without checks.
pub(crate) struct ServingImage {
    bytes: Backing,
    identity: StoreIdentity,
    slots: usize,
    chunks: usize,
    words: usize,
    docs: usize,
    dimensions: usize,
    dictionaries: [Dictionary; 2],
    sections: [(usize, usize); SECTIONS],
    /// Versions 2 to 4: postings in the block encoding (see `postings::Postings`).
    blocked: bool,
    /// Versions 3 and 4: the block encoding with impacts.
    impacts: bool,
    /// Version 4: skip tables beside the postings, positions after them, and the larger
    /// dictionary entries and block table that address both.
    extended: bool,
    /// Version 4: every chunk holds word positions.
    positional: bool,
    /// Version 4: no posting has frequency zero, so a posting is a match.
    positive: bool,
    /// Direct frequency tables of the longest lists, built by queries (see `impact`).
    dense: super::impact::DenseTables,
    /// Per slot, the class of its word count (see `impact`), built by the first pruned query.
    word_classes: std::sync::OnceLock<Vec<u8>>,
    /// Per slot, its word count capped at `u16::MAX`, built by the first pruned query.
    word_counts: std::sync::OnceLock<Vec<u16>>,
}

/// One decoded chunk of a checkpoint record.
struct RecordChunk<'a> {
    id: Option<&'a str>,
    stemmed: bool,
    analyzed: bool,
    whole: bool,
    start_byte: u32,
    end_byte: u32,
    text: Option<&'a str>,
}

fn parse_metadata(bytes: &[u8]) -> Result<serde_json::Value> {
    let mut metadata: serde_json::Value =
        serde_json::from_slice(bytes).context("checkpoint metadata")?;
    ensure!(
        metadata.is_object(),
        "checkpoint metadata must be an object"
    );
    super::compact_metadata(&mut metadata);
    Ok(metadata)
}

/// Metadata values a query's hits share, keyed by their stored bytes, so a hit whose metadata
/// repeats one already parsed clones it instead of parsing it again. It lives for one capture
/// and gives up after [`Self::GIVE_UP`] distinct blobs with no repeat, so unique metadata pays
/// for a handful of comparisons at most.
#[derive(Default)]
pub(crate) struct MetadataCache {
    entries: Vec<(Box<[u8]>, serde_json::Value)>,
    repeated: bool,
    next: usize,
    /// The entry the last lookup found: consecutive hits mostly share their metadata.
    last: usize,
}

impl MetadataCache {
    const CAPACITY: usize = 16;
    const GIVE_UP: usize = 8;
    /// The longest stored blob worth keeping.
    const LONGEST: usize = 256;

    fn metadata(&mut self, bytes: &[u8]) -> Result<serde_json::Value> {
        if bytes.len() > Self::LONGEST || (!self.repeated && self.entries.len() >= Self::GIVE_UP) {
            return parse_metadata(bytes);
        }
        let found = match self.entries.get(self.last) {
            Some((stored, _)) if &**stored == bytes => Some(self.last),
            _ => self
                .entries
                .iter()
                .position(|(stored, _)| &**stored == bytes),
        };
        if let Some(at) = found {
            self.repeated = true;
            self.last = at;
            return Ok(self.entries[at].1.clone());
        }
        let value = parse_metadata(bytes)?;
        let entry = (bytes.into(), value.clone());
        if self.entries.len() < Self::CAPACITY {
            self.entries.push(entry);
        } else {
            self.entries[self.next] = entry;
            self.next = (self.next + 1) % Self::CAPACITY;
        }
        Ok(value)
    }
}

/// One decoded checkpoint record.
struct Record<'a> {
    version: &'a str,
    source_uri: Option<&'a str>,
    metadata: serde_json::Value,
    chunks: Vec<RecordChunk<'a>>,
}

impl ServingImage {
    pub(crate) fn new(bytes: Backing) -> Result<Self> {
        ensure!(
            bytes.len() >= HEADER_LEN && known_magic(&bytes[..8]),
            "not a document checkpoint of this version"
        );
        let extended = &bytes[..8] == MAGIC;
        let impacts = extended || &bytes[..8] == MAGIC_V3;
        let blocked = impacts || &bytes[..8] == MAGIC_V2;
        let positional = extended && word(&bytes, H_FLAGS) & FLAG_POSITIONAL != 0;
        let positive = extended && word(&bytes, H_FLAGS) & FLAG_POSITIVE != 0;
        ensure!(
            word(&bytes, H_FLAGS)
                & !(if extended {
                    FLAG_POSITIONAL | FLAG_POSITIVE
                } else {
                    0
                })
                == 0,
            "checkpoint flags"
        );
        let count = |index: usize| usize::try_from(word(&bytes, index)).ok();
        let mut sections = [(0usize, 0usize); SECTIONS];
        let mut end = HEADER_LEN;
        let present = if extended { SECTIONS } else { SECTIONS_V3 };
        for (i, section) in sections.iter_mut().enumerate().take(present) {
            let (at, len) = (count(H_SECTIONS + 2 * i), count(H_SECTIONS + 2 * i + 1));
            let (Some(at), Some(len)) = (at, len) else {
                bail!("checkpoint section out of range");
            };
            ensure!(
                at == end && at.checked_add(len).is_some_and(|e| e <= bytes.len()),
                "checkpoint section out of range"
            );
            *section = (at, len);
            end = at + len;
        }
        for section in &mut sections[present..] {
            *section = (end, 0);
        }
        ensure!(end == bytes.len(), "checkpoint has trailing bytes");
        let (Some(slots), Some(chunks), Some(words), Some(docs), Some(dimensions)) = (
            count(H_SLOTS),
            count(H_CHUNKS),
            count(H_WORDS),
            count(H_DOCS),
            count(H_DIMENSIONS),
        ) else {
            bail!("checkpoint counts out of range");
        };
        let mut image = Self {
            bytes,
            identity: StoreIdentity::default(),
            slots,
            chunks,
            words,
            docs,
            dimensions,
            dictionaries: [Dictionary::default(); 2],
            sections,
            blocked,
            impacts,
            extended,
            positional,
            positive,
            dense: Default::default(),
            word_classes: std::sync::OnceLock::new(),
            word_counts: std::sync::OnceLock::new(),
        };
        image.check_documents()?;
        image.check_slots()?;
        let mut totals = (0usize, 0usize);
        for form in 0..2 {
            image.dictionaries[form] = image.check_dictionary(form, &mut totals)?;
        }
        ensure!(
            totals
                == (
                    image.section(Section::Skips).len(),
                    image.section(Section::Positions).len()
                ),
            "checkpoint skip tables or positions disagree with the dictionary"
        );
        Ok(image)
    }

    fn section(&self, section: Section) -> &[u8] {
        let (at, len) = self.sections[section as usize];
        &self.bytes[at..at + len]
    }

    fn check_documents(&self) -> Result<()> {
        for (offsets, data) in [
            (Section::IdOffsets, Section::Ids),
            (Section::DocOffsets, Section::Docs),
        ] {
            let (offsets, data) = (self.section(offsets), self.section(data));
            ensure!(
                self.docs
                    .checked_add(1)
                    .and_then(|entries| entries.checked_mul(4))
                    == Some(offsets.len()),
                "checkpoint document table size"
            );
            let mut previous = 0u32;
            for i in 0..=self.docs {
                let at = u32_at(offsets, 4 * i);
                ensure!(
                    at >= previous && at as usize <= data.len() && (i > 0 || at == 0),
                    "checkpoint document offsets"
                );
                previous = at;
            }
            ensure!(
                previous as usize == data.len(),
                "checkpoint document offsets"
            );
        }
        let ids = std::str::from_utf8(self.section(Section::Ids)).context("checkpoint ids")?;
        let mut previous: Option<&str> = None;
        for i in 0..self.docs {
            let (start, end) = self.span(Section::IdOffsets, i);
            ensure!(
                ids.is_char_boundary(start) && ids.is_char_boundary(end),
                "checkpoint id boundary"
            );
            let id = &ids[start..end];
            ensure!(
                previous.is_none_or(|p| p < id),
                "checkpoint ids out of order"
            );
            ensure!(
                !id.is_empty() && id.len() <= u16::MAX as usize && !id.contains('\0'),
                "checkpoint document id"
            );
            previous = Some(id);
        }
        Ok(())
    }

    fn check_slots(&self) -> Result<()> {
        let table = self.section(Section::Slots);
        ensure!(
            self.slots.checked_mul(SLOT_BYTES) == Some(table.len()),
            "checkpoint slot table size"
        );
        let mut used = 0usize;
        for slot in 0..self.slots {
            let ordinal = u32_at(table, slot * SLOT_BYTES + 4);
            ensure!(
                ordinal == FREE || (ordinal as usize) < self.docs,
                "checkpoint slot owner"
            );
            used += usize::from(ordinal != FREE);
        }
        ensure!(
            used == self.chunks && self.chunks <= self.slots,
            "checkpoint chunk count"
        );
        Ok(())
    }

    /// Bytes per entry of a dictionary block table.
    fn block_stride(&self) -> usize {
        if self.extended {
            32
        } else {
            8
        }
    }

    /// Where the entries of dictionary block `block` begin in each section.
    fn block_base(&self, blocks: &[u8], block: usize) -> BlockBase {
        if self.extended {
            let at = |field: usize| {
                let start = block * 32 + field * 8;
                u64::from_le_bytes(blocks[start..start + 8].try_into().expect("eight bytes"))
                    as usize
            };
            BlockBase {
                dict: at(0),
                posting: at(1),
                skip: at(2),
                pos: at(3),
            }
        } else {
            BlockBase {
                dict: u32_at(blocks, block * 8) as usize,
                posting: u32_at(blocks, block * 8 + 4) as usize,
                skip: 0,
                pos: 0,
            }
        }
    }

    /// The count, byte length and positions byte length of the entry `reader` is at, after its
    /// term.
    fn read_entry(&self, reader: &mut Reader<'_>) -> Result<Entry> {
        let count = reader.varint()?;
        let len = usize::try_from(reader.varint()?)?;
        let (pos_len, skip_len) = if self.extended {
            (
                usize::try_from(reader.varint()?)?,
                usize::try_from(reader.varint()?)?,
            )
        } else {
            (0, 0)
        };
        Ok(Entry {
            count,
            len,
            pos_len,
            skip_len,
        })
    }

    /// Checks form `form`'s dictionary: blocks in order, entries well formed with terms in
    /// strictly ascending byte order, posting, skip and positions ranges inside their sections.
    /// `totals` accumulates the skip and positions bytes the forms' entries address.
    fn check_dictionary(&self, form: usize, totals: &mut (usize, usize)) -> Result<Dictionary> {
        let (dict_section, block_section) = if form == 0 {
            (Section::Dict0, Section::Blocks0)
        } else {
            (Section::Dict1, Section::Blocks1)
        };
        let terms = usize::try_from(word(&self.bytes, H_TERMS + form))?;
        let (dict, blocks) = (self.section(dict_section), self.section(block_section));
        let block_count = terms.div_ceil(BLOCK);
        ensure!(
            blocks.len() == block_count * self.block_stride(),
            "checkpoint block table size"
        );
        let postings = self.section(Section::Postings).len();
        let mut previous = Vec::new();
        let mut reader = Reader::new(dict);
        let mut at = None::<BlockBase>;
        for i in 0..terms {
            let position = reader.position();
            if i.is_multiple_of(BLOCK) {
                let base = self.block_base(blocks, i / BLOCK);
                ensure!(base.dict == position, "checkpoint block offset");
                match at {
                    Some(at) => ensure!(
                        at.posting == base.posting && at.skip == base.skip && at.pos == base.pos,
                        "checkpoint block postings"
                    ),
                    None => ensure!(
                        !self.extended || (base.skip, base.pos) == *totals,
                        "checkpoint block postings"
                    ),
                }
                at = Some(base);
            }
            let prefix = usize::try_from(reader.varint()?)?;
            let suffix = reader.bytes()?;
            ensure!(
                prefix <= previous.len() && (!i.is_multiple_of(BLOCK) || prefix == 0),
                "checkpoint term prefix"
            );
            let mut term = previous[..prefix].to_vec();
            term.extend_from_slice(suffix);
            ensure!(
                std::str::from_utf8(&term).is_ok() && (i == 0 || previous < term),
                "checkpoint terms out of order"
            );
            let entry = self.read_entry(&mut reader)?;
            ensure!(
                entry.count > 0
                    && super::postings::checkpoint_min_len(entry.count, self.blocked, self.impacts)
                        <= entry.len as u64
                    && entry.count <= u64::from(u32::MAX),
                "checkpoint posting count"
            );
            ensure!(
                self.positional || entry.pos_len == 0,
                "checkpoint positions without positional chunks"
            );
            ensure!(
                !self.positional || entry.pos_len as u64 >= entry.count,
                "checkpoint positions length"
            );
            let base = at.as_mut().expect("set at the first entry");
            ensure!(
                base.posting
                    .checked_add(entry.len)
                    .is_some_and(|end| end <= postings),
                "checkpoint posting range"
            );
            base.posting += entry.len;
            base.pos += entry.pos_len;
            if self.extended {
                let minimum = super::lists::skip_min_len(entry.count);
                ensure!(
                    entry.skip_len >= minimum && (minimum > 0 || entry.skip_len == 0),
                    "checkpoint skip table length"
                );
                base.skip += entry.skip_len;
            }
            previous = term;
        }
        ensure!(
            reader.is_empty(),
            "checkpoint dictionary has trailing bytes"
        );
        if let Some(base) = at {
            totals.0 = base.skip;
            totals.1 = base.pos;
        }
        Ok(Dictionary {
            dict: self.sections[dict_section as usize],
            blocks: self.sections[block_section as usize],
            terms,
        })
    }

    fn span(&self, offsets: Section, i: usize) -> (usize, usize) {
        let table = self.section(offsets);
        (
            u32_at(table, 4 * i) as usize,
            u32_at(table, 4 * i + 4) as usize,
        )
    }

    pub(super) fn id(&self, ordinal: usize) -> &str {
        let (start, end) = self.span(Section::IdOffsets, ordinal);
        let ids = self.section(Section::Ids);
        std::str::from_utf8(&ids[start..end]).expect("ids checked at open")
    }

    pub(super) fn slot_total(&self) -> usize {
        self.slots
    }

    pub(super) fn document_total(&self) -> usize {
        self.docs
    }

    pub(super) fn lower_bound(&self, id: &str) -> usize {
        let (mut lo, mut hi) = (0usize, self.docs);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.id(mid) < id {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// The ordinal of document `id` (ids are stored in ascending order).
    pub(super) fn ordinal(&self, id: &str) -> Option<usize> {
        let at = self.lower_bound(id);
        (at < self.docs && self.id(at) == id).then_some(at)
    }

    pub(super) fn document_info(&self, ordinal: usize) -> Result<super::DocumentInfo> {
        Ok(self.info(ordinal, self.record(ordinal)?))
    }

    fn info(&self, ordinal: usize, record: Record<'_>) -> super::DocumentInfo {
        super::DocumentInfo {
            id: self.id(ordinal).to_owned(),
            source_uri: record.source_uri.map(str::to_owned),
            version: record.version.to_owned(),
            metadata: record.metadata,
            chunk_count: record.chunks.len(),
        }
    }

    pub(super) fn document_chunk(
        &self,
        ordinal: usize,
        index: usize,
        revision: u64,
    ) -> Result<Option<super::DocumentChunkRecord>> {
        let record = self.record(ordinal)?;
        let Some(chunk) = record.chunks.get(index) else {
            return Ok(None);
        };
        ensure!(
            chunk
                .text
                .is_none_or(|text| text.len() <= super::MAX_DOCUMENT_BYTES),
            "stored chunk exceeds read limit"
        );
        let id = chunk
            .id
            .map(str::to_owned)
            .unwrap_or_else(|| super::chunk_id(self.id(ordinal), index));
        let (start_byte, end_byte, text) = (
            chunk.start_byte,
            chunk.end_byte,
            chunk.text.map(str::to_owned),
        );
        Ok(Some(super::DocumentChunkRecord {
            revision: revision.to_string(),
            document: self.info(ordinal, record),
            index,
            id,
            start_byte,
            end_byte,
            text,
        }))
    }

    /// The ordinal of the document owning `slot`, `None` for a free slot.
    pub(super) fn slot_owner(&self, slot: usize) -> Option<usize> {
        let (_, ordinal, _) = self.slot(slot);
        (ordinal != FREE).then_some(ordinal as usize)
    }

    fn slot(&self, slot: usize) -> (u32, u32, u32) {
        let table = self.section(Section::Slots);
        let at = slot * SLOT_BYTES;
        (
            u32_at(table, at),
            u32_at(table, at + 4),
            u32_at(table, at + 8),
        )
    }

    /// Decodes document `ordinal`'s record.
    fn record(&self, ordinal: usize) -> Result<Record<'_>> {
        let mut chunks = Vec::new();
        let (version, source_uri, metadata) =
            self.decode_record(ordinal, parse_metadata, |count, chunk| {
                if chunks.capacity() == 0 {
                    chunks.reserve_exact(count);
                }
                chunks.push(chunk);
            })?;
        Ok(Record {
            version,
            source_uri,
            metadata,
            chunks,
        })
    }

    /// Walks document `ordinal`'s record, validating every field. `metadata` builds the value
    /// from the record's metadata bytes; `chunk` receives each chunk with the record's count.
    fn decode_record<'a>(
        &'a self,
        ordinal: usize,
        metadata: impl FnOnce(&'a [u8]) -> Result<serde_json::Value>,
        mut chunk: impl FnMut(usize, RecordChunk<'a>),
    ) -> Result<(&'a str, Option<&'a str>, serde_json::Value)> {
        let (start, end) = self.span(Section::DocOffsets, ordinal);
        let mut reader = Reader::new(&self.section(Section::Docs)[start..end]);
        let version = reader.str()?;
        ensure!(
            (1..=1024).contains(&version.len()),
            "checkpoint version length"
        );
        let source_uri = match reader.byte()? {
            0 => None,
            1 => Some(reader.str()?),
            other => bail!("checkpoint source flag {other}"),
        };
        ensure!(
            source_uri.is_none_or(|uri| uri.len() <= 8192),
            "checkpoint source URI length"
        );
        let metadata_bytes = reader.bytes()?;
        ensure!(
            metadata_bytes.len() <= 64 * 1024,
            "checkpoint metadata exceeds 64 KiB"
        );
        let metadata = metadata(metadata_bytes)?;
        let count = reader.count(4)?;
        ensure!(
            (1..=4096).contains(&count),
            "checkpoint document chunk count"
        );
        for _ in 0..count {
            let flags = reader.byte()?;
            ensure!(
                flags & !(OWN_ID | STEMMED | ANALYZED | WHOLE) == 0,
                "checkpoint chunk flags {flags}"
            );
            let id = if flags & OWN_ID != 0 {
                Some(reader.str()?)
            } else {
                None
            };
            ensure!(
                id.is_none_or(|id| !id.is_empty()
                    && id.len() <= u16::MAX as usize
                    && !id.contains('\0')),
                "checkpoint chunk id"
            );
            let start_byte = u32::try_from(reader.varint()?)?;
            let end_byte = u32::try_from(reader.varint()?)?;
            ensure!(
                start_byte < end_byte && end_byte as usize <= super::MAX_DOCUMENT_BYTES,
                "checkpoint chunk byte span"
            );
            let text = match reader.byte()? {
                TEXT_NONE => None,
                TEXT_INLINE => Some(reader.str()?),
                other => bail!("checkpoint text kind {other}"),
            };
            ensure!(
                text.is_none_or(|text| text.len() == (end_byte - start_byte) as usize),
                "checkpoint chunk text length"
            );
            chunk(
                count,
                RecordChunk {
                    id,
                    stemmed: flags & STEMMED != 0,
                    analyzed: flags & ANALYZED != 0,
                    whole: flags & WHOLE != 0,
                    start_byte,
                    end_byte,
                    text,
                },
            );
        }
        ensure!(reader.is_empty(), "checkpoint record has trailing bytes");
        Ok((version, source_uri, metadata))
    }

    /// A stored chunk as the live store holds a lexical-only one; text inline.
    fn stored_chunk(
        &self,
        document: &str,
        index: usize,
        chunk: &RecordChunk<'_>,
        text: Option<String>,
        terms: StoredTerms,
        word_count: usize,
    ) -> StoredChunk {
        StoredChunk {
            id: chunk
                .id
                .map_or_else(|| super::chunk_id(document, index), str::to_owned),
            text,
            text_at: None,
            start_byte: chunk.start_byte,
            end_byte: chunk.end_byte,
            vector: EntangledHVec::from_indices(Vec::new(), self.dimensions),
            embedding: None,
            terms,
            word_count,
            stemmed: chunk.stemmed,
            analyzed: chunk.analyzed,
            vector_state: VectorState::Lexical,
        }
    }

    /// What a result reports of the chunk in `slot`, unscored: the chunk's id, span and inline
    /// text, and its document's id, source, version and metadata.
    pub(crate) fn hit(
        &self,
        slot: usize,
        cache: &mut MetadataCache,
    ) -> Result<super::DocumentResult> {
        let (_, ordinal, index) = self.slot(slot);
        ensure!(ordinal != FREE, "checkpoint slot is free");
        let (ordinal, index) = (ordinal as usize, index as usize);
        let id = self.id(ordinal);
        let (mut found, mut seen) = (None, 0usize);
        let (version, source_uri, metadata) = self.decode_record(
            ordinal,
            |bytes| cache.metadata(bytes),
            |_, chunk| {
                if seen == index {
                    found = Some(chunk);
                }
                seen += 1;
            },
        )?;
        let chunk = found.context("checkpoint chunk index")?;
        Ok(super::DocumentResult {
            id: chunk
                .id
                .map_or_else(|| super::chunk_id(id, index), str::to_owned),
            document_id: id.to_owned(),
            source_uri: source_uri.map(str::to_owned),
            version: version.to_owned(),
            metadata,
            text: chunk.text.map(str::to_owned),
            start_byte: chunk.start_byte,
            end_byte: chunk.end_byte,
            score: 0.0,
            lexical_score: 0.0,
            semantic_score: None,
        })
    }

    /// The live store this checkpoint holds, chunk text moved to `text` when given: the same
    /// documents, slots (free ones included), posting bytes and counts, with each chunk's terms
    /// read back from the postings.
    #[cfg(test)]
    pub(crate) fn hydrate(&self, text: Option<Arc<TextSegment>>) -> Result<DocumentStore> {
        self.hydrate_excluding(text, &[])
    }

    /// [`Self::hydrate`] with the postings of the documents `dead` left out: those documents are
    /// still installed, with no terms, for the caller to remove with
    /// `DocumentStore::remove_unindexed`, which then leaves the store exactly as hydrating
    /// everything and removing them would, without decoding and re-encoding every posting list
    /// a dead document is in.
    pub(crate) fn hydrate_excluding(
        &self,
        text: Option<Arc<TextSegment>>,
        dead: &[String],
    ) -> Result<DocumentStore> {
        let dead: std::collections::HashSet<&str> = dead.iter().map(String::as_str).collect();
        let dead_ordinal: Vec<bool> = if dead.is_empty() {
            Vec::new()
        } else {
            (0..self.docs)
                .map(|ordinal| dead.contains(self.id(ordinal)))
                .collect()
        };
        // Owner of every occupied slot, and each document's slots in chunk order.
        let mut document_slots: Vec<Vec<usize>> = vec![Vec::new(); self.docs];
        let mut form_of = vec![u8::MAX; self.slots];
        for slot in 0..self.slots {
            let (_, ordinal, index) = self.slot(slot);
            if ordinal == FREE {
                continue;
            }
            let slots = &mut document_slots[ordinal as usize];
            ensure!(
                index as usize == slots.len(),
                "checkpoint chunk slots out of order"
            );
            slots.push(slot);
        }
        let mut dead_slot = vec![false; if dead.is_empty() { 0 } else { self.slots }];
        for (slots, _) in document_slots
            .iter()
            .zip(&dead_ordinal)
            .filter(|(_, &dead)| dead)
        {
            for &slot in slots {
                dead_slot[slot] = true;
            }
        }
        let records = (0..self.docs)
            .map(|ordinal| self.record(ordinal))
            .collect::<Result<Vec<_>>>()?;
        for (record, slots) in records.iter().zip(&document_slots) {
            ensure!(
                record.chunks.len() == slots.len(),
                "checkpoint record and slots disagree"
            );
            for (chunk, &slot) in record.chunks.iter().zip(slots) {
                form_of[slot] = u8::from(chunk.stemmed);
            }
        }
        // Each slot's `(term, frequency)` pairs, in term order per form (a chunk's terms are
        // all in its own form), read from one pass over the postings; with positions, each
        // slot's position offsets and bytes in the same order.
        let mut forward: Vec<Vec<(u32, u32)>> = vec![Vec::new(); self.slots];
        let mut slot_positions: Vec<(Vec<u32>, Vec<u8>)> = if self.positional {
            vec![(vec![0], Vec::new()); self.slots]
        } else {
            Vec::new()
        };
        let mut lists: [Vec<RestoredList>; 2] = [Vec::new(), Vec::new()];
        for (form, out) in lists.iter_mut().enumerate() {
            let dictionary = self.dictionaries[form];
            let dict = &self.bytes[dictionary.dict.0..dictionary.dict.0 + dictionary.dict.1];
            let postings = self.section(Section::Postings);
            let (skips, positions) = (
                self.section(Section::Skips),
                self.section(Section::Positions),
            );
            let mut reader = Reader::new(dict);
            let mut term = Vec::new();
            let mut base = if dictionary.terms == 0 {
                BlockBase {
                    dict: 0,
                    posting: 0,
                    skip: 0,
                    pos: 0,
                }
            } else {
                let blocks =
                    &self.bytes[dictionary.blocks.0..dictionary.blocks.0 + dictionary.blocks.1];
                self.block_base(blocks, 0)
            };
            out.reserve_exact(dictionary.terms);
            for _ in 0..dictionary.terms {
                let prefix = usize::try_from(reader.varint()?)?;
                term.truncate(prefix);
                term.extend_from_slice(reader.bytes()?);
                let entry = self.read_entry(&mut reader)?;
                let count = u32::try_from(entry.count)?;
                let len = entry.len;
                let bytes = &postings[base.posting..base.posting + len];
                let skip_len = entry.skip_len;
                let list = ListRef {
                    bytes,
                    count,
                    skips: &skips[base.skip..base.skip + skip_len],
                    positions: &positions[base.pos..base.pos + entry.pos_len],
                };
                base.posting += len;
                base.skip += skip_len;
                base.pos += entry.pos_len;
                let mut last = 0usize;
                let mut seen = 0u32;
                let mut failure = None;
                let mut live = Vec::with_capacity(count as usize);
                let mut ordinals = Vec::new();
                // The term's position among the lists kept: a list of dead documents only is
                // left out, and no live slot refers to it.
                let position = out.len() as u32;
                let mut visit = |slot: usize, frequency: u32, positions: Option<&[u8]>| {
                    let fits = slot < self.slots
                        && form_of[slot] == form as u8
                        && (seen == 0 || slot > last);
                    if !fits {
                        failure = Some(slot);
                        return false;
                    }
                    if dead_slot.get(slot) != Some(&true) {
                        forward[slot].push((position, frequency));
                        if let Some(bytes) = positions {
                            let (offsets, data) = &mut slot_positions[slot];
                            data.extend_from_slice(bytes);
                            offsets.push(data.len() as u32);
                            ordinals.push(forward[slot].len() as u32);
                        }
                        live.push((slot, frequency));
                    }
                    last = slot;
                    seen += 1;
                    true
                };
                if self.positional {
                    // The block decoder checks the list's postings before the unchecked
                    // cursor reads its skip table and positions.
                    Postings {
                        bytes,
                        count,
                        blocked: self.blocked,
                        impacts: self.impacts,
                    }
                    .try_for_each(
                        &|slot| {
                            if slot < self.slots {
                                self.slot(slot).0
                            } else {
                                0
                            }
                        },
                        |_, _| true,
                    )?;
                    let mut cursor = Cursor::open(list);
                    while !cursor.is_end() {
                        let (slot, frequency) = (cursor.doc() as usize, cursor.frequency());
                        let bytes = cursor.position_bytes();
                        if !visit(slot, frequency, Some(bytes)) {
                            break;
                        }
                        cursor.next();
                    }
                } else {
                    Postings {
                        bytes,
                        count,
                        blocked: self.blocked,
                        impacts: self.impacts,
                    }
                    .try_for_each(
                        &|slot| {
                            if slot < self.slots {
                                self.slot(slot).0
                            } else {
                                0
                            }
                        },
                        |slot, frequency| visit(slot, frequency, None),
                    )?;
                }
                ensure!(
                    failure.is_none() && seen == count,
                    "checkpoint posting list for a slot it cannot hold"
                );
                let term = std::str::from_utf8(&term).context("checkpoint term")?;
                if !live.is_empty() {
                    out.push((term.to_owned(), live, ordinals));
                }
            }
        }
        let terms: [Vec<&str>; 2] = [
            lists[0].iter().map(|(t, ..)| t.as_str()).collect(),
            lists[1].iter().map(|(t, ..)| t.as_str()).collect(),
        ];
        let mut documents = Vec::with_capacity(self.docs);
        for (ordinal, (record, slots)) in records.iter().zip(&document_slots).enumerate() {
            let id = self.id(ordinal);
            let mut chunks = Vec::with_capacity(slots.len());
            for (index, (chunk, &slot)) in record.chunks.iter().zip(slots).enumerate() {
                let pairs = std::mem::take(&mut forward[slot]);
                let names = &terms[usize::from(chunk.stemmed)];
                let bytes = pairs.iter().map(|&(t, _)| names[t as usize].len()).sum();
                let mut stored = StoredTerms::from_sorted_refs(
                    pairs.len(),
                    bytes,
                    pairs.iter().map(|&(t, f)| (names[t as usize], f)),
                );
                if self.positional {
                    let (offsets, data) = std::mem::take(&mut slot_positions[slot]);
                    let positions = super::positions::Positions {
                        version: if chunk.whole {
                            super::positions::DOCUMENT_VERSION
                        } else {
                            super::positions::VERSION
                        },
                        offsets: offsets.into(),
                        data: data.into_boxed_slice(),
                    };
                    positions
                        .validate(&stored)
                        .context("checkpoint positions")?;
                    stored.positions = Some(Arc::new(positions));
                }
                let mut stored_chunk = self.stored_chunk(
                    id,
                    index,
                    chunk,
                    chunk.text.map(str::to_owned),
                    stored,
                    self.slot(slot).0 as usize,
                );
                if let Some(segment) = &text {
                    if let Some(inline) = stored_chunk.text.take() {
                        stored_chunk.text_at = Some(segment.append(&inline)?);
                    }
                }
                chunks.push(stored_chunk);
            }
            documents.push((
                Arc::new(StoredDocument {
                    vector_record: false,
                    id: id.to_owned(),
                    source_uri: record.source_uri.map(str::to_owned),
                    version: record.version.to_owned(),
                    metadata: record.metadata.clone(),
                    chunks,
                }),
                slots.clone(),
            ));
        }
        drop(terms);
        let words: Vec<u32> = (0..self.slots).map(|slot| self.slot(slot).0).collect();
        let [zero, one] = lists;
        let postings = [
            PostingLists::from_sorted(zero, &words),
            PostingLists::from_sorted(one, &words),
        ];
        let store = DocumentStore::restore(documents, self.slots, postings, text);
        ensure!(
            (store.chunk_count, store.word_count) == (self.chunks, self.words),
            "checkpoint counts disagree with its slots"
        );
        Ok(store)
    }
}

impl ServingImage {
    /// The posting list of `term` in posting form `form`, with its skip table and positions.
    pub(super) fn list(&self, form: usize, term: &str) -> Option<ListRef<'_>> {
        let dictionary = &self.dictionaries[form];
        if dictionary.terms == 0 {
            return None;
        }
        let dict = &self.bytes[dictionary.dict.0..dictionary.dict.0 + dictionary.dict.1];
        let blocks = &self.bytes[dictionary.blocks.0..dictionary.blocks.0 + dictionary.blocks.1];
        let first_term = |block: usize| -> &[u8] {
            let mut reader = Reader::new(&dict[self.block_base(blocks, block).dict..]);
            reader.varint().expect("dictionary checked at open");
            reader.bytes().expect("dictionary checked at open")
        };
        let target = term.as_bytes();
        // The last block whose first term is at or below the target.
        let (mut lo, mut hi) = (0usize, blocks.len() / self.block_stride());
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if first_term(mid) <= target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let block = lo;
        let entries = (dictionary.terms - block * BLOCK).min(BLOCK);
        let mut base = self.block_base(blocks, block);
        let mut reader = Reader::new(&dict[base.dict..]);
        let postings = self.section(Section::Postings);
        let (skips, positions) = (
            self.section(Section::Skips),
            self.section(Section::Positions),
        );
        let mut current: Vec<u8> = Vec::with_capacity(target.len() + 8);
        for _ in 0..entries {
            let prefix = reader.varint().expect("dictionary checked at open") as usize;
            let suffix = reader.bytes().expect("dictionary checked at open");
            current.truncate(prefix);
            current.extend_from_slice(suffix);
            let entry = self
                .read_entry(&mut reader)
                .expect("dictionary checked at open");
            let skip_len = entry.skip_len;
            match current.as_slice().cmp(target) {
                std::cmp::Ordering::Equal => {
                    return Some(ListRef {
                        bytes: &postings[base.posting..base.posting + entry.len],
                        count: entry.count as u32,
                        skips: &skips[base.skip..base.skip + skip_len],
                        positions: &positions[base.pos..base.pos + entry.pos_len],
                    })
                }
                std::cmp::Ordering::Greater => return None,
                std::cmp::Ordering::Less => {
                    base.posting += entry.len;
                    base.skip += skip_len;
                    base.pos += entry.pos_len;
                }
            }
        }
        None
    }

    /// The terms of posting form `form`.
    pub(super) fn term_count(&self, form: usize) -> usize {
        self.dictionaries[form].terms
    }

    /// Whether every chunk holds word positions (so phrases can be matched from the mapping).
    pub(super) fn has_positions(&self) -> bool {
        self.positional
    }

    /// Whether this checkpoint's lists carry skip tables (version 4) and every posting is a
    /// match (none has frequency zero), what the executors over the mapping rely on.
    pub(super) fn has_skips(&self) -> bool {
        self.extended && self.positive
    }

    /// Whether this checkpoint's postings carry the impacts block-max pruning reads.
    pub(super) fn has_impacts(&self) -> bool {
        self.impacts
    }

    /// The slot table: per slot its word count, document ordinal and chunk index, `u32`s.
    pub(super) fn slot_table(&self) -> &[u8] {
        self.section(Section::Slots)
    }

    /// Per slot the entry of `class_of` for its word count.
    pub(super) fn word_classes(&self, class_of: &[u8]) -> &[u8] {
        self.word_classes.get_or_init(|| {
            let last = class_of.len() - 1;
            (0..self.slots)
                .map(|slot| class_of[(self.slot(slot).0 as usize).min(last)])
                .collect()
        })
    }

    /// Per slot its word count, `u16::MAX` for counts that do not fit (read the slot table then):
    /// a table a sixth the size of the slot table, so it stays in cache where the slot table
    /// does not.
    pub(super) fn word_counts(&self) -> &[u16] {
        self.word_counts.get_or_init(|| {
            (0..self.slots)
                .map(|slot| self.slot(slot).0.min(u32::from(u16::MAX)) as u16)
                .collect()
        })
    }
    pub(super) fn dense(&self) -> &super::impact::DenseTables {
        &self.dense
    }
}

impl LexicalSource for ServingImage {
    type List<'a> = Postings<'a>;

    fn impact_image(&self) -> Option<&ServingImage> {
        self.impacts.then_some(self)
    }

    fn slot_count(&self) -> usize {
        self.slots
    }

    fn chunk_count(&self) -> usize {
        self.chunks
    }

    fn word_count(&self) -> usize {
        self.words
    }

    fn state(&self) -> (StoreIdentity, u64) {
        (self.identity, 0)
    }

    fn postings(&self, form: usize, term: &str) -> Option<Postings<'_>> {
        self.list(form, term).map(|list| Postings {
            bytes: list.bytes,
            count: list.count,
            blocked: self.blocked,
            impacts: self.impacts,
        })
    }

    fn slot_word_count(&self, slot: usize) -> u32 {
        self.slot(slot).0
    }

    fn slot_order(&self, slot: usize) -> (&str, usize) {
        let (_, ordinal, index) = self.slot(slot);
        if ordinal == FREE {
            return ("", index as usize);
        }
        (self.id(ordinal as usize), index as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checkpoint of header words `words` (index, value) and empty-but-sized sections.
    fn checkpoint(words: &[(usize, u64)], lens: [usize; SECTIONS]) -> Backing {
        let mut header = [0u64; HEADER_WORDS];
        for &(index, value) in words {
            header[index] = value;
        }
        let mut at = HEADER_LEN as u64;
        for (i, &len) in lens.iter().enumerate() {
            header[H_SECTIONS + 2 * i] = at;
            header[H_SECTIONS + 2 * i + 1] = len as u64;
            at += len as u64;
        }
        let mut bytes = MAGIC.to_vec();
        header
            .iter()
            .for_each(|w| bytes.extend_from_slice(&w.to_le_bytes()));
        bytes.resize(at as usize, 0);
        Backing::Owned(bytes)
    }

    #[test]
    fn table_sizes_that_overflow_are_errors() {
        let mut lens = [0; SECTIONS];
        lens[Section::IdOffsets as usize] = 4;
        lens[Section::DocOffsets as usize] = 4;
        assert!(ServingImage::new(checkpoint(&[], lens)).is_ok());
        // (docs + 1) * 4 and slots * 12 wrap to the 4- and 0-byte tables.
        assert!(ServingImage::new(checkpoint(&[(H_DOCS, 1 << 62)], lens)).is_err());
        assert!(ServingImage::new(checkpoint(&[(H_SLOTS, 1 << 62)], lens)).is_err());
    }
    fn image_with_record(record: &[u8]) -> ServingImage {
        let mut lens = [0; SECTIONS];
        lens[Section::IdOffsets as usize] = 8;
        lens[Section::Ids as usize] = 3;
        lens[Section::DocOffsets as usize] = 8;
        lens[Section::Docs as usize] = record.len();
        let mut bytes = checkpoint(&[(H_DOCS, 1)], lens).to_vec();
        for (section, data) in [
            (
                Section::IdOffsets,
                [0u32.to_le_bytes(), 3u32.to_le_bytes()].concat(),
            ),
            (Section::Ids, b"doc".to_vec()),
            (
                Section::DocOffsets,
                [0u32.to_le_bytes(), (record.len() as u32).to_le_bytes()].concat(),
            ),
            (Section::Docs, record.to_vec()),
        ] {
            let at = word(&bytes, H_SECTIONS + 2 * section as usize) as usize;
            bytes[at..at + data.len()].copy_from_slice(&data);
        }
        ServingImage::new(Backing::Owned(bytes)).unwrap()
    }

    fn record_prefix(version: usize, uri: usize, metadata: usize, chunks: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_bytes(&mut bytes, "v".repeat(version).as_bytes());
        bytes.push(1);
        put_bytes(&mut bytes, "u".repeat(uri).as_bytes());
        let metadata = serde_json::json!({"x": "x".repeat(metadata - 8)}).to_string();
        put_bytes(&mut bytes, metadata.as_bytes());
        put_varint(&mut bytes, chunks as u64);
        bytes
    }

    #[test]
    fn record_lengths_obey_ingest_bounds_before_allocating_fields() {
        for (field, limit) in [
            ("version", 1024),
            ("uri", 8192),
            ("metadata", 64 * 1024),
            ("chunks", 4096),
        ] {
            for value in [limit - 1, limit, limit + 1] {
                let (mut version, mut uri, mut metadata, mut chunks) = (1, 1, 8, 1);
                match field {
                    "version" => version = value,
                    "uri" => uri = value,
                    "metadata" => metadata = value,
                    _ => chunks = value,
                }
                let mut bytes = record_prefix(version, uri, metadata, chunks);
                for _ in 0..chunks {
                    bytes.extend_from_slice(&[0, 0, 1, TEXT_NONE]);
                }
                let image = image_with_record(&bytes);
                assert_eq!(image.record(0).is_ok(), value <= limit, "{field} {value}");
            }
        }
        let bytes = record_prefix(1, 1, 8, 0);
        assert!(image_with_record(&bytes).record(0).is_err());
    }

    #[test]
    fn chunk_ids_spans_and_text_obey_stored_record_bounds() {
        for length in [65534, 65535, 65536] {
            let mut bytes = record_prefix(1, 1, 8, 1);
            bytes.push(OWN_ID);
            put_bytes(&mut bytes, "i".repeat(length).as_bytes());
            bytes.extend_from_slice(&[0, 1, TEXT_NONE]);
            assert_eq!(image_with_record(&bytes).record(0).is_ok(), length <= 65535);
        }
        for end in [
            super::super::MAX_DOCUMENT_BYTES - 1,
            super::super::MAX_DOCUMENT_BYTES,
            super::super::MAX_DOCUMENT_BYTES + 1,
        ] {
            let mut bytes = record_prefix(1, 1, 8, 1);
            bytes.extend_from_slice(&[0, 0]);
            put_varint(&mut bytes, end as u64);
            bytes.push(TEXT_NONE);
            assert_eq!(
                image_with_record(&bytes).record(0).is_ok(),
                end <= super::super::MAX_DOCUMENT_BYTES
            );
        }
        for (end, text) in [(1, "x"), (2, "x"), (1, "xx")] {
            let mut bytes = record_prefix(1, 1, 8, 1);
            bytes.extend_from_slice(&[0, 0, end, TEXT_INLINE]);
            put_bytes(&mut bytes, text.as_bytes());
            assert_eq!(
                image_with_record(&bytes).record(0).is_ok(),
                text.len() == end as usize
            );
        }
    }

    #[test]
    fn metadata_cache_returns_what_parsing_returns() {
        // Repeats, more distinct blobs than the cache holds or gives up after, a blob past the
        // longest kept, and blobs that are not objects: every call equals a fresh parse.
        let long = format!("{{\"k\":\"{}\"}}", "x".repeat(MetadataCache::LONGEST));
        let mut blobs: Vec<String> = (0..3 * MetadataCache::CAPACITY)
            .map(|i| format!("{{\"bucket\":{}}}", i % 5))
            .collect();
        blobs.extend((0..40).map(|i| format!("{{\"id\":{i},\"tags\":[\"a\",\"b\"]}}")));
        blobs.push(long);
        blobs.extend((0..4).map(|i| format!("{{\"bucket\":{i}}}")));
        let mut cache = MetadataCache::default();
        for blob in &blobs {
            let cached = cache.metadata(blob.as_bytes()).unwrap();
            assert_eq!(cached, parse_metadata(blob.as_bytes()).unwrap(), "{blob}");
        }
        for bad in ["[]", "3", "{", ""] {
            assert!(cache.metadata(bad.as_bytes()).is_err(), "{bad:?}");
        }
        let mut unique = MetadataCache::default();
        for i in 0..100 {
            unique
                .metadata(format!("{{\"id\":{i}}}").as_bytes())
                .unwrap();
        }
        assert!(unique.entries.len() <= MetadataCache::GIVE_UP);
    }

    /// `record` and `slots` are the image's one document and its slot table as
    /// `(word_count, chunk index)` pairs, all owned by that document.
    fn image_with_record_and_slots(record: &[u8], slots: &[(u32, u32)]) -> ServingImage {
        let mut lens = [0; SECTIONS];
        lens[Section::IdOffsets as usize] = 8;
        lens[Section::Ids as usize] = 3;
        lens[Section::DocOffsets as usize] = 8;
        lens[Section::Docs as usize] = record.len();
        lens[Section::Slots as usize] = slots.len() * SLOT_BYTES;
        let words = [
            (H_DOCS, 1),
            (H_SLOTS, slots.len() as u64),
            (H_CHUNKS, slots.len() as u64),
        ];
        let mut bytes = checkpoint(&words, lens).to_vec();
        let table: Vec<u8> = slots
            .iter()
            .flat_map(|&(words, chunk)| {
                [words.to_le_bytes(), 0u32.to_le_bytes(), chunk.to_le_bytes()].concat()
            })
            .collect();
        for (section, data) in [
            (
                Section::IdOffsets,
                [0u32.to_le_bytes(), 3u32.to_le_bytes()].concat(),
            ),
            (Section::Ids, b"doc".to_vec()),
            (
                Section::DocOffsets,
                [0u32.to_le_bytes(), (record.len() as u32).to_le_bytes()].concat(),
            ),
            (Section::Docs, record.to_vec()),
            (Section::Slots, table),
        ] {
            let at = word(&bytes, H_SECTIONS + 2 * section as usize) as usize;
            bytes[at..at + data.len()].copy_from_slice(&data);
        }
        ServingImage::new(Backing::Owned(bytes)).unwrap()
    }

    #[test]
    fn hit_reports_the_same_chunk_the_full_record_decodes() {
        // Three chunks: a bare one, one with its own id, inline text and both analyzer
        // flags, and a bare third; a fourth slot names a chunk the record does not have.
        let mut bytes = record_prefix(2, 3, 12, 3);
        bytes.extend_from_slice(&[0, 0, 1, TEXT_NONE]);
        bytes.push(OWN_ID | STEMMED | ANALYZED);
        put_bytes(&mut bytes, b"own-id");
        bytes.extend_from_slice(&[1, 3, TEXT_INLINE]);
        put_bytes(&mut bytes, b"xy");
        bytes.extend_from_slice(&[0, 3, 4, TEXT_NONE]);
        let image = image_with_record_and_slots(&bytes, &[(5, 0), (7, 1), (9, 2), (1, 7)]);
        let record = image.record(0).unwrap();
        assert_eq!(record.chunks.len(), 3);
        let mut cache = MetadataCache::default();
        for (slot, chunk) in record.chunks.iter().enumerate() {
            let hit = image.hit(slot, &mut cache).unwrap();
            let want_id = chunk
                .id
                .map_or_else(|| super::super::chunk_id("doc", slot), str::to_owned);
            assert_eq!(hit.id, want_id, "slot {slot}");
            assert_eq!(hit.document_id, "doc");
            assert_eq!(hit.version, record.version);
            assert_eq!(hit.source_uri.as_deref(), record.source_uri);
            assert_eq!(hit.metadata, record.metadata);
            assert_eq!(hit.text.as_deref(), chunk.text, "slot {slot}");
            assert_eq!(
                (hit.start_byte, hit.end_byte),
                (chunk.start_byte, chunk.end_byte)
            );
            assert_eq!(image.slot_word_count(slot), [5, 7, 9][slot]);
        }
        assert_eq!(record.chunks[1].id, Some("own-id"));
        assert!(record.chunks[1].stemmed && record.chunks[1].analyzed);
        assert_eq!(record.chunks[1].text, Some("xy"));
        let error = image.hit(3, &mut cache).unwrap_err().to_string();
        assert!(error.contains("chunk index"), "{error}");
    }
}
