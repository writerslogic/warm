// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Term-hash buckets the per-range maps are merged by; the parallelism of that merge.
pub(super) const MERGE_BUCKETS: usize = 16;
/// Batches below this many chunks are scanned as one range into one bucket.
pub(super) const PARALLEL_CHUNKS: usize = 64;
/// Postings after which `index_chunks` closes a group of chunks and packs it.
pub(super) const GROUP_POSTINGS: usize = 1 << 18;
/// The group grows with the slots already indexed, so a large index is packed in few groups: the
/// transient is small beside it and each group repeats the per-term work.
pub(super) const GROUP_POSTINGS_PER_SLOT: usize = 256;
/// Smallest chunk range one thread scans.
pub(super) const MIN_RANGE: usize = 32;
/// Run counts below this look their terms up on the calling thread.
pub(super) const PARALLEL_LOOKUPS: usize = 256;

/// One term's new postings for a batch, in strictly ascending slot order.
pub(in super::super) struct TermRun<'a> {
    pub(super) term: &'a str,
    pub(super) postings: Vec<Posting>,
    pub(super) ordinals: Vec<u32>,
}

/// The postings of one term form (stemmed or not).
#[derive(Default)]
pub(in super::super) struct PostingLists {
    pub(super) dictionary: TermDictionary,
    pub(super) lists: Vec<PostingList>,
    pub(super) free: Vec<u32>,
}

impl PostingLists {
    #[cfg(test)]
    pub(in super::super) fn record_memory(&self, census: &mut super::super::census::MemoryCensus) {
        use std::mem::{size_of, size_of_val};
        census
            .counts
            .insert("postings.block_size", size_of::<Block>());
        census
            .counts
            .insert("postings.list_size", size_of::<PostingList>());
        census.bytes(
            "postings.lists.capacity",
            self.lists.capacity() * size_of::<PostingList>(),
        );
        census.bytes(
            "postings.free.capacity",
            self.free.capacity() * size_of::<u32>(),
        );
        census.bytes(
            "postings.dictionary.text.capacity",
            self.dictionary.bytes.capacity(),
        );
        census.bytes(
            "postings.dictionary.offsets.capacity",
            self.dictionary.offsets.capacity() * size_of::<u32>(),
        );
        census.bytes(
            "postings.dictionary.ids.capacity",
            self.dictionary.base_ids.capacity() * size_of::<u32>(),
        );
        census.bytes(
            "postings.dictionary.overlay.payload",
            self.dictionary
                .overlay
                .iter()
                .map(|(term, id)| term.len() + size_of_val(term) + size_of_val(id))
                .sum(),
        );
        for list in &self.lists {
            *census
                .histogram
                .entry((list.len(), list.blocks.len(), list.blocks.capacity()))
                .or_default() += 1;
            census.count("postings.postings", list.len());
            census.count("postings.lists", 1);
            census.count("postings.blocks", list.blocks.len());
            census.bytes("postings.data.length", list.data.len());
            census.bytes("postings.data.capacity", list.data.capacity());
            census.bytes(
                "postings.data.unused",
                list.data.capacity() - list.data.len(),
            );
            census.bytes(
                "postings.blocks.length",
                list.blocks.len() * size_of::<Block>(),
            );
            census.bytes(
                "postings.blocks.capacity",
                list.blocks.capacity() * size_of::<Block>(),
            );
            census.bytes(
                "postings.blocks.unused",
                (list.blocks.capacity() - list.blocks.len()) * size_of::<Block>(),
            );
            for block in &list.blocks {
                census.bytes(
                    "postings.frontiers",
                    block.frontier.as_deref().map_or(0, size_of_val),
                );
            }
        }
    }

    /// Lists for `entries`: `(term, postings)` in strictly ascending term order, every list
    /// non-empty and sorted by slot; `words` holds each slot's word count, which bounds the
    /// blocks as a fresh build bounds them.
    pub(in super::super) fn from_sorted(entries: Vec<RestoredList>, words: &[u32]) -> Self {
        let mut dictionary = TermDictionary::default();
        let mut lists = Vec::with_capacity(entries.len());
        dictionary.offsets.reserve_exact(entries.len() + 1);
        dictionary.offsets.push(0);
        for (id, (term, postings, ordinals)) in entries.into_iter().enumerate() {
            debug_assert!(!postings.is_empty(), "empty posting list");
            dictionary.bytes.extend_from_slice(term.as_bytes());
            dictionary
                .offsets
                .push(u32::try_from(dictionary.bytes.len()).expect("term dictionary over 4 GiB"));
            dictionary
                .base_ids
                .push(u32::try_from(id).expect("term count over u32"));
            let mut list = PostingList::default();
            list.encode_with_ordinals(&postings, &ordinals, |slot| {
                words.get(slot).copied().unwrap_or(0)
            });
            lists.push(list);
        }
        debug_assert!(
            (1..dictionary.base_ids.len())
                .all(|i| dictionary.base_term(i - 1) < dictionary.base_term(i)),
            "unsorted posting terms"
        );
        dictionary.bytes.shrink_to_fit();
        Self {
            dictionary,
            lists,
            free: Vec::new(),
        }
    }

    pub(in super::super) fn get(&self, term: &str) -> Option<&PostingList> {
        self.dictionary.get(term).map(|id| &self.lists[id as usize])
    }

    /// Every term with postings in byte order (`offsets` delimit the terms in `bytes`) and
    /// its list, as a serving image stores them.
    pub(in super::super) fn sorted_terms(&self) -> (Vec<u8>, Vec<u32>, Vec<&PostingList>) {
        let overlay: Vec<(&[u8], u32)> = self
            .dictionary
            .overlay
            .iter()
            .map(|(term, &id)| (term.as_bytes(), id))
            .collect();
        let (bytes, offsets, ids) = self.dictionary.merged_base(&overlay);
        let lists = ids.iter().map(|&id| &self.lists[id as usize]).collect();
        (bytes, offsets, lists)
    }

    /// Number of terms with postings.
    #[cfg(test)]
    pub(in super::super) fn term_count(&self) -> usize {
        self.dictionary.len()
    }

    /// Adds a `(term, slot)` posting absent from the lists; `words` holds each slot's length.
    #[cfg(test)]
    pub(in super::super) fn insert(
        &mut self,
        term: &str,
        slot: usize,
        frequency: u32,
        words: &[u32],
    ) {
        self.insert_with_ordinal(term, slot, frequency, 0, words);
    }

    pub(in super::super) fn insert_with_ordinal(
        &mut self,
        term: &str,
        slot: usize,
        frequency: u32,
        ordinal: u32,
        words: &[u32],
    ) {
        let id = match self.dictionary.get(term) {
            Some(id) => id,
            None => {
                let id = self.allocate_list();
                self.dictionary.insert(term, id);
                id
            }
        };
        self.lists[id as usize].append_run_with_ordinals(
            &[(slot, frequency)],
            &[ordinal],
            |slot| words.get(slot).copied().unwrap_or(0),
        );
    }

    /// Adds one run per distinct term, each absent from that term's list.
    #[cfg(test)]
    pub(in super::super) fn insert_runs(&mut self, runs: &[TermRun<'_>]) {
        self.insert_runs_with_lengths(runs, &[]);
    }

    pub(super) fn insert_runs_with_lengths(&mut self, runs: &[TermRun<'_>], word_counts: &[u32]) {
        self.install_runs(runs, word_counts, runs.len() >= PARALLEL_LOOKUPS);
    }

    /// Appends each run to its term's list. The serial prologue resolves ids and allocates
    /// lists for new terms in run order, so list ids do not depend on `parallel`; the appends
    /// then touch disjoint lists.
    pub(super) fn install_runs(&mut self, runs: &[TermRun<'_>], word_counts: &[u32], parallel: bool) {
        if runs.is_empty() {
            return;
        }
        let dictionary = &self.dictionary;
        let found: Vec<Option<u32>> = if parallel {
            runs.par_iter()
                .map(|run| dictionary.get(run.term))
                .collect()
        } else {
            runs.iter().map(|run| dictionary.get(run.term)).collect()
        };
        let mut new_terms = Vec::new();
        let mut targets: Vec<(u32, u32)> = Vec::with_capacity(runs.len());
        for (index, (run, id)) in runs.iter().zip(found).enumerate() {
            let id = id.unwrap_or_else(|| {
                let id = self.allocate_list();
                new_terms.push((run.term, id));
                id
            });
            targets.push((id, index as u32));
        }
        let words = |slot: usize| word_counts.get(slot).copied().unwrap_or(0);
        if parallel {
            // Runs hold distinct terms and each live term has one list, so ids are distinct.
            targets.sort_unstable_by_key(|&(id, _)| id);
            debug_assert!(
                targets.windows(2).all(|pair| pair[0].0 < pair[1].0),
                "duplicate list id in install targets"
            );
            append_disjoint(&mut self.lists, 0, &targets, runs, &words);
        } else {
            for &(id, index) in &targets {
                let run = &runs[index as usize];
                self.lists[id as usize].append_run_with_ordinals(
                    &run.postings,
                    &run.ordinals,
                    words,
                );
            }
        }
        self.dictionary.insert_many(new_terms);
    }

    /// The id of an empty list for a new term.
    pub(super) fn allocate_list(&mut self) -> u32 {
        match self.free.pop() {
            Some(id) => id,
            None => {
                self.lists.push(PostingList::default());
                u32::try_from(self.lists.len() - 1).expect("too many terms")
            }
        }
    }

    pub(in super::super) fn remove(&mut self, term: &str, slot: usize, words: &[u32]) {
        let id = self
            .dictionary
            .get(term)
            .expect("document term missing posting");
        let list = &mut self.lists[id as usize];
        list.remove(slot, |slot| words.get(slot).copied().unwrap_or(0));
        if list.is_empty() {
            *list = PostingList::default();
            self.dictionary.remove(term);
            self.free.push(id);
        }
    }

    pub(in super::super) fn invalidate_length(&mut self, term: &str, slot: usize) {
        let id = self
            .dictionary
            .get(term)
            .expect("document term missing posting");
        self.lists[id as usize].invalidate_length(slot);
    }

    #[cfg(test)]
    pub(in super::super) fn set_frequency(
        &mut self,
        term: &str,
        slot: usize,
        frequency: u32,
        words: &[u32],
    ) {
        self.set_entry(term, slot, frequency, None, words);
    }

    pub(in super::super) fn set_entry(
        &mut self,
        term: &str,
        slot: usize,
        frequency: u32,
        ordinal: Option<u32>,
        words: &[u32],
    ) {
        let id = self
            .dictionary
            .get(term)
            .expect("document term missing posting");
        self.lists[id as usize].set_entry(slot, frequency, ordinal, |slot| {
            words.get(slot).copied().unwrap_or(0)
        });
    }

    /// Moves every posting from slot `s` to slot `map[s]` (`map` is injective on the slots
    /// that hold postings), keeping its frequency and positional ordinal; `words` holds the
    /// word count of each new slot. One list is decoded at a time per thread, so the extra
    /// memory is bounded by the longest lists, not the index.
    pub(in super::super) fn permute_slots(&mut self, map: &[u32], words: &[u32]) {
        self.lists
            .par_iter_mut()
            .for_each(|list| list.permute_slots(map, words));
    }

    /// The lists of at least `min_len` postings, in list id order.
    pub(in super::super) fn long_lists(
        &self,
        min_len: usize,
    ) -> impl Iterator<Item = &PostingList> {
        self.lists.iter().filter(move |list| list.len() >= min_len)
    }

    /// Every live term has a non-empty, strictly slot-sorted list whose recorded final slot
    /// is its last posting; every free id an empty one.
    #[cfg(test)]
    pub(in super::super) fn assert_valid(&self) {
        let mut live = vec![false; self.lists.len()];
        let ids = self.dictionary.overlay.values().copied().chain(
            self.dictionary
                .base_ids
                .iter()
                .copied()
                .filter(|&id| id != NONE),
        );
        for id in ids {
            assert!(!live[id as usize], "list id shared by two terms");
            live[id as usize] = true;
            let list = &self.lists[id as usize];
            assert!(!list.is_empty(), "live term with an empty list");
            let slots: Vec<usize> = list.iter().map(|(slot, _)| slot).collect();
            assert!(slots.windows(2).all(|pair| pair[0] < pair[1]));
            assert_eq!(slots.last().copied(), Some(list.last as usize));
            // In-place edits keep every block but the last at least a quarter full.
            let (last, earlier) = list.blocks.split_last().expect("nonempty list");
            assert!(last.count > 0);
            assert!(earlier
                .iter()
                .all(|b| usize::from(b.count) >= MIN_BLOCK_LEN));
        }
        for &id in &self.free {
            assert!(!live[id as usize] && self.lists[id as usize].is_empty());
        }
        assert_eq!(
            live.iter().filter(|&&l| l).count() + self.free.len(),
            self.lists.len()
        );
    }
}

/// Per-range accumulator: a term's postings in the order its chunks were scanned.
pub(super) type RunMap<'a> = HashMap<&'a str, TermRun<'a>, FixedState>;

/// Merge bucket of a term: FNV-1a over its bytes, deliberately not the maps' hash, so the
/// keys that share a bucket do not also share the low hash bits their map probes by.
pub(super) fn bucket_of(term: &str, buckets: usize) -> usize {
    let hash = term.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    (hash % buckets as u64) as usize
}

/// [`scan_range`] for a range whose chunks all take their terms from one dictionary: a term is
/// found by its dictionary id in a table, not by hashing its text, and only each distinct term
/// reaches the maps. `false`, with `maps` untouched, when the range does not qualify.
pub(super) fn scan_shared<'a>(
    range: &[(usize, &'a StoredChunk)],
    buckets: usize,
    maps: &mut [Vec<RunMap<'a>>; 2],
) -> bool {
    let Some(&(_, first)) = range.first() else {
        return false;
    };
    let Some((dictionary, _)) = first.terms.shared_ids() else {
        return false;
    };
    let same = range.iter().all(|(_, chunk)| {
        chunk
            .terms
            .shared_ids()
            .is_some_and(|(other, _)| std::sync::Arc::ptr_eq(other, dictionary))
    });
    if !same {
        return false;
    }
    const NONE: u32 = u32::MAX;
    let words = dictionary.len();
    let mut tables: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
    let mut runs: [Vec<TermRun<'a>>; 2] = [Vec::new(), Vec::new()];
    for &(slot, chunk) in range {
        let form = usize::from(chunk.stemmed);
        if tables[form].is_empty() {
            tables[form] = vec![NONE; words];
        }
        let (_, ids) = chunk.terms.shared_ids().expect("checked above");
        for (id, frequency) in ids {
            let at = &mut tables[form][id as usize];
            if *at == NONE {
                *at = runs[form].len() as u32;
                runs[form].push(TermRun {
                    term: dictionary.term(id as usize),
                    postings: Vec::new(),
                    ordinals: Vec::new(),
                });
            }
            runs[form][*at as usize].postings.push((slot, frequency));
        }
    }
    for (form, runs) in runs.into_iter().enumerate() {
        for run in runs {
            let bucket = if buckets == 1 {
                0
            } else {
                bucket_of(run.term, buckets)
            };
            maps[form][bucket].insert(run.term, run);
        }
    }
    true
}
/// Scans one chunk range into per-form, per-bucket maps.
pub(super) fn scan_range<'a>(
    range: &[(usize, &'a StoredChunk)],
    buckets: usize,
) -> [Vec<RunMap<'a>>; 2] {
    let mut maps: [Vec<RunMap<'a>>; 2] =
        std::array::from_fn(|_| (0..buckets).map(|_| RunMap::default()).collect());
    if scan_shared(range, buckets, &mut maps) {
        return maps;
    }
    for &(slot, chunk) in range {
        let form = &mut maps[usize::from(chunk.stemmed)];
        for (index, (term, &frequency)) in (&chunk.terms).into_iter().enumerate() {
            let bucket = if buckets == 1 {
                0
            } else {
                bucket_of(term, buckets)
            };
            let run = form[bucket].entry(term).or_insert_with(|| TermRun {
                term,
                postings: Vec::new(),
                ordinals: Vec::new(),
            });
            let ordinal = if chunk.terms.positions.is_some() {
                index as u32 + 1
            } else {
                0
            };
            if ordinal > 0 && run.ordinals.is_empty() {
                run.ordinals.resize(run.postings.len(), 0);
            }
            if ordinal > 0 || !run.ordinals.is_empty() {
                run.ordinals.push(ordinal);
            }
            run.postings.push((slot, frequency));
        }
    }
    maps
}

/// Merges the maps of one bucket, given in range order, into one run per term, sorted by
/// term so list ids are assigned the same way whatever the thread count.
pub(super) fn merge_bucket<'a>(maps: Vec<RunMap<'a>>) -> Vec<TermRun<'a>> {
    let mut maps = maps.into_iter();
    let Some(mut merged) = maps.next() else {
        return Vec::new();
    };
    for map in maps {
        for (term, run) in map {
            match merged.entry(term) {
                Entry::Occupied(entry) => {
                    let previous = entry.into_mut();
                    debug_assert!(
                        previous
                            .postings
                            .last()
                            .is_some_and(|last| last.0 < run.postings[0].0),
                        "chunk ranges merged out of slot order"
                    );
                    if !previous.ordinals.is_empty() || !run.ordinals.is_empty() {
                        previous.ordinals.resize(previous.postings.len(), 0);
                        previous.ordinals.extend_from_slice(&run.ordinals);
                        previous
                            .ordinals
                            .resize(previous.postings.len() + run.postings.len(), 0);
                    }
                    previous.postings.extend(run.postings);
                }
                Entry::Vacant(entry) => {
                    entry.insert(run);
                }
            }
        }
    }
    let mut runs: Vec<TermRun<'a>> = merged.into_values().collect();
    runs.sort_unstable_by(|a, b| a.term.cmp(b.term));
    runs
}

/// Targets per task below which `append_disjoint` stops splitting.
const APPEND_LEAF: usize = 256;

/// Appends `runs[index]` to `lists[id - base]` for every `(id, index)` of `targets`, which is
/// sorted by strictly ascending id with every id inside `lists`' window starting at `base`.
/// The window is split at a target's id so each half owns the lists of its targets.
pub(super) fn append_disjoint(
    lists: &mut [PostingList],
    base: u32,
    targets: &[(u32, u32)],
    runs: &[TermRun<'_>],
    words: &(impl Fn(usize) -> u32 + Sync),
) {
    if targets.len() <= APPEND_LEAF {
        for &(id, index) in targets {
            let run = &runs[index as usize];
            lists[(id - base) as usize].append_run_with_ordinals(
                &run.postings,
                &run.ordinals,
                words,
            );
        }
        return;
    }
    let (head, tail) = targets.split_at(targets.len() / 2);
    let split = tail[0].0;
    let (left, right) = lists.split_at_mut((split - base) as usize);
    rayon::join(
        || append_disjoint(left, base, head, runs, words),
        || append_disjoint(right, split, tail, runs, words),
    );
}

/// Indexes `chunks`, given as `(slot, chunk)` in strictly ascending slot order with every
/// slot absent from the lists, into the unstemmed (`postings[0]`) and stemmed (`postings[1]`)
/// lists. See the module head for the phases.
pub(in super::super) fn index_chunks(
    postings: &mut [PostingLists; 2],
    chunks: &[(usize, &StoredChunk)],
    word_counts: &[u32],
) {
    debug_assert!(
        chunks.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "chunks out of slot order"
    );
    // Runs hold 16 bytes per posting until they are packed into the lists, so a large batch is
    // indexed in slot-ordered groups: the peak is the lists plus one group, not the whole batch.
    let mut rest = chunks;
    while !rest.is_empty() {
        let mut postings_in_group = 0usize;
        let group_cap = GROUP_POSTINGS.max(rest[0].0.saturating_mul(GROUP_POSTINGS_PER_SLOT));
        let mut end = 0;
        while end < rest.len() && (end == 0 || postings_in_group < group_cap) {
            postings_in_group += rest[end].1.terms.keys().len();
            end += 1;
        }
        let (group, tail) = rest.split_at(end);
        index_group(postings, group, word_counts);
        rest = tail;
    }
}
pub(super) fn index_group<'a>(
    postings: &mut [PostingLists; 2],
    chunks: &[(usize, &'a StoredChunk)],
    word_counts: &[u32],
) {
    let buckets = if chunks.len() < PARALLEL_CHUNKS {
        1
    } else {
        MERGE_BUCKETS
    };
    let range = chunks
        .len()
        .div_ceil(rayon::current_num_threads().max(1))
        .max(MIN_RANGE);
    let partials: Vec<[Vec<RunMap<'a>>; 2]> = chunks
        .par_chunks(range)
        .map(|slice| scan_range(slice, buckets))
        .collect();
    // Every range's map of one (form, bucket) goes to that bucket's merge, in range order.
    let mut by_bucket: Vec<Vec<RunMap<'a>>> = (0..2 * buckets)
        .map(|_| Vec::with_capacity(partials.len()))
        .collect();
    for partial in partials {
        for (form, maps) in partial.into_iter().enumerate() {
            for (bucket, map) in maps.into_iter().enumerate() {
                by_bucket[form * buckets + bucket].push(map);
            }
        }
    }
    let merged: Vec<Vec<TermRun<'a>>> = by_bucket.into_par_iter().map(merge_bucket).collect();
    let mut merged = merged.into_iter();
    let unstemmed: Vec<TermRun<'a>> = merged.by_ref().take(buckets).flatten().collect();
    let stemmed: Vec<TermRun<'a>> = merged.by_ref().take(buckets).flatten().collect();
    let [first, second] = postings;
    rayon::join(
        || first.insert_runs_with_lengths(&unstemmed, word_counts),
        || second.insert_runs_with_lengths(&stemmed, word_counts),
    );
}
