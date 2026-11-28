// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Compact lexical postings: a two-level term dictionary and independently bitpacked blocks.
//!
//! The dictionary keeps a sorted base of concatenated term bytes with one `u32` offset and
//! one `u32` list id per term, and a small overlay map for terms added since the base was
//! built; the overlay folds into the base when it outgrows a quarter of it. A posting list is
//! a sequence of blocks of at most 128 slots, with independent bit widths for gaps and
//! frequencies, positional term ordinals, block endpoints, maximum frequency and a
//! conservative minimum word count.
//! Postings are rebuilt from journal term tables, so these bytes are never persisted.
//!
//! Bulk indexing ([`index_chunks`]) takes a batch of chunks in ascending slot order and
//! builds every term's new postings once:
//!
//! 1. Contiguous chunk ranges are scanned in parallel into per-range hash maps keyed by the
//!    chunk's own term bytes (no term is copied or allocated), one map per term-hash bucket.
//! 2. The maps of one bucket merge, in parallel across buckets, into one slot-ordered run per
//!    term: ranges are ordered, so a later range's postings go after an earlier range's.
//! 3. Each run is looked up once in the dictionary (parallel, read-only), then appended to
//!    its list in one bitpacking pass; nothing stored is re-encoded unless the run starts
//!    at or below the list's final slot (a slot reused after deletes), which merges that one
//!    list once. Ids are resolved and new lists allocated serially; the appends, which touch
//!    disjoint lists, run in parallel for large batches, and the two forms install together.
//! 4. Terms new to the dictionary are merged into a fresh sorted base in one pass together
//!    with the overlay when there are many, or added to the overlay when there are few.
//!
//! Single-chunk inserts and the per-term replacement path take the same append-or-merge step
//! through [`PostingLists::insert_with_ordinal`], so one document at a time and one batch reach the same
//! lists.

use std::collections::{hash_map::Entry, BTreeMap, HashMap};

use foldhash::fast::FixedState;
use rayon::prelude::*;

use super::binary::put_varint;
use super::StoredChunk;

const NONE: u32 = u32::MAX;

/// `(slot, frequency)`.
pub(super) type Posting = (usize, u32);

/// One restored list: its term, its postings in slot order and, for positional postings, each
/// posting's chunk-term ordinal plus one (empty for a list of non-positional postings).
pub(super) type RestoredList = (String, Vec<Posting>, Vec<u32>);

/// Decodes one varint at `*at`, advancing it, from checkpoint postings the open checked.
#[inline(always)]
pub(super) fn get_varint(bytes: &[u8], at: &mut usize) -> u64 {
    let first = bytes[*at];
    *at += 1;
    if first < 0x80 {
        return u64::from(first);
    }
    let mut value = u64::from(first & 0x7f);
    let mut shift = 7;
    loop {
        let b = bytes[*at];
        *at += 1;
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return value;
        }
        shift += 7;
        assert!(shift < 64, "posting varint overflow");
    }
}

mod bits;
mod checkpoint;
mod cursor;
#[cfg(test)]
mod cursor_tests;
mod dictionary;
mod list;
mod set;
#[cfg(test)]
mod tests;
pub(super) use self::bits::*;
pub(super) use self::checkpoint::*;
pub(super) use self::cursor::*;
pub(super) use self::dictionary::*;
pub(super) use self::list::*;
pub(super) use self::set::*;
