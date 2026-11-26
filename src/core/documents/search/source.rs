// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// What [`LexicalScratch::rank`] reads: the live store, or a mapped checkpoint of one.
pub(in super::super) trait LexicalSource {
    type List<'a>: PostingSet
    where
        Self: 'a;
    fn slot_count(&self) -> usize;
    fn chunk_count(&self) -> usize;
    fn word_count(&self) -> usize;
    /// The instance and generation cached norms are valid for.
    fn state(&self) -> (super::super::index::StoreIdentity, u64);
    fn postings(&self, form: usize, term: &str) -> Option<Self::List<'_>>;
    fn slot_word_count(&self, slot: usize) -> u32;
    /// `(document id, chunk index)` of the chunk in `slot`, the tie order of equal scores.
    fn slot_order(&self, slot: usize) -> (&str, usize);
    /// The order of [`Self::slot_order`] as one integer, when this source can give it.
    fn slot_tie_key(&self, _slot: usize) -> Option<u64> {
        None
    }
    /// The checkpoint whose postings carry impacts, when this source is one: its queries may
    /// be pruned by block-max bounds (see [`super::super::impact`]).
    fn impact_image(&self) -> Option<&super::super::image::ServingImage> {
        None
    }
    /// The shortest posting list and the most postings a thread keeps decoded and weighted
    /// for this source (see [`HotList`]).
    fn hot_policy(&self) -> HotPolicy {
        HotPolicy {
            min_len: HOT_MIN_LEN.max(self.slot_count() / HOT_MIN_SLOTS),
            postings: HOT_POSTINGS,
        }
    }
}

/// How much of a source's posting lists a thread keeps decoded and weighted.
#[derive(Clone, Copy)]
pub(in super::super) struct HotPolicy {
    pub(super) min_len: usize,
    pub(super) postings: usize,
}

/// A term's postings as [`LexicalScratch::rank`] reads them.
pub(in super::super) trait PostingSet {
    fn len(&self) -> usize;
    fn for_each(&self, f: impl FnMut(usize, u32));
}

impl PostingSet for Postings<'_> {
    fn len(&self) -> usize {
        Postings::len(self)
    }

    fn for_each(&self, f: impl FnMut(usize, u32)) {
        Postings::for_each(self, f);
    }
}

impl PostingSet for &PostingList {
    fn len(&self) -> usize {
        PostingList::len(self)
    }

    fn for_each(&self, f: impl FnMut(usize, u32)) {
        PostingList::for_each(self, f);
    }
}

impl LexicalSource for DocumentStore {
    type List<'a> = &'a PostingList;

    fn slot_count(&self) -> usize {
        self.slot_count
    }

    fn chunk_count(&self) -> usize {
        self.chunk_count
    }

    fn word_count(&self) -> usize {
        self.word_count
    }

    fn state(&self) -> (super::super::index::StoreIdentity, u64) {
        (self.identity, self.generation)
    }

    fn postings(&self, form: usize, term: &str) -> Option<&PostingList> {
        self.postings[form].get(term)
    }

    fn slot_word_count(&self, slot: usize) -> u32 {
        DocumentStore::slot_word_count(self, slot)
    }

    fn slot_order(&self, slot: usize) -> (&str, usize) {
        DocumentStore::slot_order(self, slot)
    }

    /// A live store's lists are short and its queries long: keep every list of 16 postings or more.
    fn hot_policy(&self) -> HotPolicy {
        HotPolicy {
            min_len: LIVE_HOT_MIN_LEN,
            postings: LIVE_HOT_POSTINGS,
        }
    }
}

pub(super) struct FilteredPostings<'a> {
    pub(super) list: &'a PostingList,
    pub(super) slots: &'a [bool],
    pub(super) count: usize,
}

impl PostingSet for FilteredPostings<'_> {
    fn len(&self) -> usize {
        self.count
    }

    fn for_each(&self, mut f: impl FnMut(usize, u32)) {
        self.list.for_each(|slot, frequency| {
            if self.slots[slot] {
                f(slot, frequency);
            }
        });
    }
}

pub(super) struct FilteredLexical<'a> {
    pub(super) store: &'a DocumentStore,
    pub(super) slots: &'a [bool],
    pub(super) dfs: &'a [BTreeMap<String, usize>; 2],
    pub(super) count: usize,
    pub(super) words: usize,
    pub(super) identity: super::super::index::StoreIdentity,
}

impl LexicalSource for FilteredLexical<'_> {
    type List<'a>
        = FilteredPostings<'a>
    where
        Self: 'a;

    fn slot_count(&self) -> usize {
        self.slots.len()
    }
    fn chunk_count(&self) -> usize {
        self.count
    }
    fn word_count(&self) -> usize {
        self.words
    }
    fn state(&self) -> (super::super::index::StoreIdentity, u64) {
        (self.identity, self.store.generation)
    }
    fn slot_word_count(&self, slot: usize) -> u32 {
        self.store.slot_word_count(slot)
    }
    fn slot_order(&self, slot: usize) -> (&str, usize) {
        self.store.slot_order(slot)
    }

    fn postings(&self, form: usize, term: &str) -> Option<Self::List<'_>> {
        Some(FilteredPostings {
            list: self.store.postings[form].get(term)?,
            slots: self.slots,
            count: self.dfs[form][term],
        })
    }
}
