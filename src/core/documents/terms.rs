// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(super) fn stemmer() -> &'static rust_stemmers::Stemmer {
    static STEMMER: OnceLock<rust_stemmers::Stemmer> = OnceLock::new();
    STEMMER.get_or_init(|| rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::English))
}

/// `text` in Unicode NFC, borrowed when it already is, so precomposed and decomposed
/// spellings split into the same words. Applied to the whole text before splitting: per word
/// would be too late, since a decomposed accent ends a word.
pub(crate) fn nfc(text: &str) -> std::borrow::Cow<'_, str> {
    use unicode_normalization::{is_nfc_quick, IsNormalized, UnicodeNormalization};
    if text.is_ascii() || is_nfc_quick(text.chars()) == IsNormalized::Yes {
        std::borrow::Cow::Borrowed(text)
    } else {
        std::borrow::Cow::Owned(text.nfc().collect())
    }
}

/// Writes `word` lowercased into `out`. ASCII is lowercased byte by byte. Other words take
/// `str::to_lowercase` (it applies the final-sigma rule), lose the U+0307 it leaves after
/// `i` for `İ`, and fold dotless `ı` to `i`: without a locale, uppercase Turkish `I` cannot be
/// told from `İ`, so `IŞIK` and `ışık` must meet at one form.
pub(crate) fn lowercase_word(word: &str, out: &mut String) {
    out.clear();
    if word.is_ascii() {
        out.push_str(word);
        out.make_ascii_lowercase();
        return;
    }
    let mut previous = '\0';
    out.extend(word.to_lowercase().chars().filter_map(|c| {
        let c = if c == 'ı' { 'i' } else { c };
        let keep = !(c == '\u{307}' && previous == 'i');
        previous = c;
        keep.then_some(c)
    }));
}

/// Lowercased tokens ([`nfc`], [`words`], [`lowercase_word`]) and their counts; with `stem`, each
/// token is reduced by the English Snowball (Porter 2) stemmer, which is what the published
/// BM25 baselines do.
/// The distinct words of a query in one form.
#[cfg(test)]
pub(crate) fn query_words(text: &str, stem: bool) -> std::collections::BTreeSet<String> {
    query_words_with(Analyzer::Legacy, text, stem)
}

/// [`query_words`] through `analyzer`.
#[cfg(test)]
pub(crate) fn query_words_with(
    analyzer: Analyzer,
    text: &str,
    stem: bool,
) -> std::collections::BTreeSet<String> {
    terms_with(analyzer, text, stem).into_keys().collect()
}

/// The distinct terms of `text` in both forms (unstemmed, stemmed) through `analyzer`: the
/// keys of [`terms_with`] for each form, from one tokenization of the text.
pub(crate) fn query_words_both(
    analyzer: Analyzer,
    text: &str,
) -> [std::collections::BTreeSet<String>; 2] {
    let mut sets: [std::collections::BTreeSet<String>; 2] = Default::default();
    let mut lower = String::new();
    let text = analyzer.prepare(text);
    for word in analyzer.words(&text) {
        for (form, set) in sets.iter_mut().enumerate() {
            let term = analyzer.term(word, form == 1, &mut lower);
            if !term.is_empty() && !set.contains(&*term) {
                set.insert(term.into_owned());
            }
        }
    }
    sets
}

/// [`terms_with`] through [`Analyzer::Legacy`].
#[cfg(any(test, feature = "experimental"))]
pub(crate) fn terms(text: &str, stem: bool) -> BTreeMap<String, u32> {
    terms_with(Analyzer::Legacy, text, stem)
}

/// The terms of `text` in one form through `analyzer`, with their counts.
pub(crate) fn terms_with(analyzer: Analyzer, text: &str, stem: bool) -> BTreeMap<String, u32> {
    // Counted in a hash map keyed by borrowed text (one allocation per distinct term, not per
    // occurrence), then sorted once into the ordered map the postings expect.
    // Seeded per process: the keys are caller text.
    let mut counts: std::collections::HashMap<String, u32, foldhash::fast::RandomState> =
        std::collections::HashMap::with_capacity_and_hasher(text.len() / 16, Default::default());
    let mut lower = String::new();
    let mut add = |key: &str| match counts.get_mut(key) {
        Some(count) => *count += 1,
        None => {
            counts.insert(key.to_owned(), 1);
        }
    };
    let text = analyzer.prepare(text);
    for word in analyzer.words(&text) {
        let term = analyzer.term(word, stem, &mut lower);
        if !term.is_empty() {
            add(&term);
        }
    }
    // Keys are unique, so an unstable sort is exact; the map's own stable sort then sees one
    // ascending run and finishes in a single pass.
    let mut sorted: Vec<(String, u32)> = counts.into_iter().collect();
    sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    sorted.into_iter().collect()
}

/// Words a batch interner maps before it starts over; bounds memory on corpora with
/// unbounded vocabularies while keeping the common-word hit rate.
pub(super) const STEM_MEMO_CAPACITY: usize = 1 << 18;

/// The term id of a word whose term normalizes to nothing; it is never counted.
pub(super) const EMPTY_TERM: u32 = u32::MAX;

/// Hash map keyed by caller text, so seeded per process.
pub(super) type SeededMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;

/// The lexical state of ingest on one thread, carried across a store's batches
/// ([`StemMemoPool`]): every distinct word seen maps to the id of its term (lowercased,
/// Snowball-stemmed), so a repeated word costs one hash lookup and no copy, stemming runs
/// once per distinct word, and a chunk's counts accumulate in an array by term id.
#[derive(Default)]
pub(super) struct Interner {
    /// Word exactly as it appears in the text -> term id.
    pub(super) words: WordMap,
    /// Term <-> id, so the words of one stem count as one term.
    pub(super) terms: TermStore,
    /// Each term's first eight bytes big-endian (zero-padded), by id: a term never contains
    /// a NUL byte, so comparing keys orders terms as their bytes do except on equal keys.
    pub(super) keys: Vec<u64>,
    /// Count per term id for the chunk being counted, and the ids counted so far. Zero and
    /// empty between chunks.
    pub(super) counts: Vec<u32>,
    pub(super) counted: Vec<u32>,
    /// The term id of each word of the chunk, in text order, while `record` is set.
    pub(super) sequence: Vec<u32>,
    pub(super) record: bool,
    pub(super) lower: String,
    /// Terms are the lowercased words themselves rather than their stems.
    pub(super) unstemmed: bool,
    pub(super) analyzer: Analyzer,
}

/// Distinct terms in one arena: id -> bytes through end offsets, and term -> id through a
/// table keyed by the term's hash with a chain through ids of equal hash. A term costs its
/// bytes plus about 24 bytes of tables, with no allocation of its own.
#[derive(Default)]
pub(super) struct TermStore {
    pub(super) arena: String,
    pub(super) ends: Vec<u32>,
    pub(super) heads: SeededMap<u64, u32>,
    pub(super) chain: Vec<u32>,
}

impl TermStore {
    pub(super) fn term(&self, id: u32) -> &str {
        let end = self.ends[id as usize] as usize;
        let start = match id {
            0 => 0,
            _ => self.ends[id as usize - 1] as usize,
        };
        &self.arena[start..end]
    }

    pub(super) fn hash(&self, term: &str) -> u64 {
        std::hash::BuildHasher::hash_one(self.heads.hasher(), term)
    }

    pub(super) fn get(&self, term: &str) -> Option<u32> {
        let mut id = *self.heads.get(&self.hash(term))?;
        while id != u32::MAX {
            if self.term(id) == term {
                return Some(id);
            }
            id = self.chain[id as usize];
        }
        None
    }

    /// Adds `term`, which is not present, and returns its id.
    pub(super) fn push(&mut self, term: &str) -> u32 {
        let id = u32::try_from(self.ends.len()).expect("batch vocabulary over u32");
        let end = u32::try_from(self.arena.len() + term.len()).expect("batch terms over 4 GiB");
        let hash = self.hash(term);
        self.arena.push_str(term);
        self.ends.push(end);
        let head = self.heads.insert(hash, id).unwrap_or(u32::MAX);
        self.chain.push(head);
        id
    }
}

/// Word -> term id. A word of at most 16 bytes (nearly every word) is keyed by its bytes
/// zero-padded into a `u128`, unambiguous because a word holds no NUL byte, so a lookup
/// compares integers rather than following a pointer to compare strings.
#[derive(Default)]
pub(super) struct WordMap {
    pub(super) short: SeededMap<[u64; 2], u32>,
    pub(super) long: SeededMap<Box<str>, u32>,
}

impl WordMap {
    #[inline]
    pub(super) fn short_key(word: &str) -> Option<[u64; 2]> {
        debug_assert!(!word.contains('\0'));
        let bytes = word.as_bytes();
        (bytes.len() <= 16).then(|| {
            let mut key = [0u8; 16];
            key[..bytes.len()].copy_from_slice(bytes);
            let key = u128::from_le_bytes(key);
            [key as u64, (key >> 64) as u64]
        })
    }

    #[inline]
    pub(super) fn get(&self, word: &str) -> Option<u32> {
        match Self::short_key(word) {
            Some(key) => self.short.get(&key).copied(),
            None => self.long.get(word).copied(),
        }
    }

    pub(super) fn insert(&mut self, word: &str, id: u32) {
        match Self::short_key(word) {
            Some(key) => self.short.insert(key, id),
            None => self.long.insert(word.into(), id),
        };
    }

    pub(super) fn len(&self) -> usize {
        self.short.len() + self.long.len()
    }
}

impl Interner {
    pub(super) fn term_id(&mut self, word: &str, ascii: bool) -> u32 {
        if let Some(id) = self.words.get(word) {
            return id;
        }
        let stem = if ascii && self.analyzer.is_legacy() {
            self.lower.clear();
            self.lower.push_str(word);
            self.lower.make_ascii_lowercase();
            if self.unstemmed {
                std::borrow::Cow::Borrowed(self.lower.as_str())
            } else {
                stemmer().stem(&self.lower)
            }
        } else {
            self.analyzer.term(word, !self.unstemmed, &mut self.lower)
        };
        if stem.is_empty() {
            self.words.insert(word, EMPTY_TERM);
            return EMPTY_TERM;
        }
        let id = match self.terms.get(stem.as_ref()) {
            Some(id) => id,
            None => {
                let mut key = [0u8; 8];
                let prefix = stem.len().min(8);
                key[..prefix].copy_from_slice(&stem.as_bytes()[..prefix]);
                self.keys.push(u64::from_be_bytes(key));
                self.counts.push(0);
                self.terms.push(stem.as_ref())
            }
        };
        self.words.insert(word, id);
        id
    }

    /// Counts the words of `text` (already [`nfc`]) `weight` times each, as `terms(text, true)`
    /// splits them ([`words`]); returns how many were counted, weighted.
    pub(super) fn count(&mut self, text: &str, weight: u32) -> usize {
        if !self.analyzer.splits_alphanumeric() {
            let mut counted = 0;
            for word in self.analyzer.words(text) {
                counted += self.count_word(word, false, weight);
            }
            return counted;
        }
        let bytes = text.as_bytes();
        let mut counted = 0;
        let mut at = 0;
        let char_at = |at: usize| text[at..].chars().next().expect("char boundary");
        while at < bytes.len() {
            if bytes[at] < 0x80 {
                if !bytes[at].is_ascii_alphanumeric() {
                    at += 1;
                    continue;
                }
            } else {
                let c = char_at(at);
                if !c.is_alphanumeric() {
                    at += c.len_utf8();
                    continue;
                }
            }
            let start = at;
            let mut ascii = true;
            while at < bytes.len() {
                if bytes[at] < 0x80 {
                    if !bytes[at].is_ascii_alphanumeric() {
                        break;
                    }
                    at += 1;
                } else {
                    let c = char_at(at);
                    if !c.is_alphanumeric() {
                        break;
                    }
                    ascii = false;
                    at += c.len_utf8();
                }
            }
            counted += self.count_word(&text[start..at], ascii, weight);
        }
        counted
    }

    /// Counts one word `weight` times; returns how many were counted (none for a word whose
    /// term normalizes away).
    #[inline]
    pub(super) fn count_word(&mut self, word: &str, ascii: bool, weight: u32) -> usize {
        let id = self.term_id(word, ascii);
        if id == EMPTY_TERM {
            return 0;
        }
        if self.record {
            self.sequence.push(id);
        }
        let count = &mut self.counts[id as usize];
        if *count == 0 {
            self.counted.push(id);
        }
        *count += weight;
        weight as usize
    }

    /// The term table of `text` with its first line's terms counted `title_weight` times in
    /// total, and the weighted word count: what `terms(text, true)` plus the title weighting
    /// produce.
    /// With `positions` (`Some(whole_document)`), the table also carries each term's word
    /// positions, the bytes the reference analyzer (`Positions::from_text`) writes.
    pub(super) fn chunk_terms(
        &mut self,
        text: &str,
        title_weight: u32,
        stem: bool,
        positions: Option<bool>,
        analyzer: Analyzer,
    ) -> (StoredTerms, usize) {
        if self.words.len() >= STEM_MEMO_CAPACITY
            || self.unstemmed == stem
            || self.analyzer != analyzer
        {
            *self = Self {
                unstemmed: !stem,
                analyzer,
                ..Self::default()
            };
        }
        let text = analyzer.prepare(text);
        let text = text.as_ref();
        self.sequence.clear();
        self.record = positions.is_some();
        let mut words = self.count(text, 1);
        self.record = false;
        if title_weight > 1 {
            words += self.count(text.lines().next().unwrap_or_default(), title_weight - 1);
        }
        let (terms, keys, counts) = (&self.terms, &self.keys, &mut self.counts);
        self.counted.sort_unstable_by(|&a, &b| {
            let (a, b) = (a as usize, b as usize);
            keys[a].cmp(&keys[b]).then_with(|| {
                terms
                    .term(a as u32)
                    .as_bytes()
                    .cmp(terms.term(b as u32).as_bytes())
            })
        });
        let bytes = self.counted.iter().map(|&id| terms.term(id).len()).sum();
        let mut table = StoredTerms::from_sorted_refs(
            self.counted.len(),
            bytes,
            self.counted
                .iter()
                .map(|&id| (terms.term(id), counts[id as usize])),
        );
        if let Some(whole_document) = positions {
            // Each word's term as its rank among the chunk's sorted terms, through `counts`
            // (cleared below).
            for (rank, &id) in self.counted.iter().enumerate() {
                counts[id as usize] = rank as u32;
            }
            for id in &mut self.sequence {
                *id = counts[*id as usize];
            }
            table.positions = Some(Arc::new(positions::Positions::from_ranks(
                &self.sequence,
                self.counted.len(),
                whole_document,
            )));
            self.sequence.clear();
        }
        for &id in &self.counted {
            counts[id as usize] = 0;
        }
        self.counted.clear();
        (table, words)
    }
}

thread_local! {
    pub(super) static ACTIVE_STEM_MEMO: std::cell::RefCell<Option<Interner>> =
        const { std::cell::RefCell::new(None) };
}

/// The word interner of an ingest batch. It is installed on a thread only while that thread
/// prepares documents for the batch and is kept by the store between batches
/// ([`StemMemoPool`]), so no thread keeps it and queries never use it.
#[derive(Default)]
pub(crate) struct StemMemo(pub(super) Interner);

pub(crate) struct StemMemoGuard<'a>(&'a mut StemMemo);

impl StemMemo {
    pub(crate) fn install(&mut self) -> StemMemoGuard<'_> {
        let table = std::mem::take(&mut self.0);
        ACTIVE_STEM_MEMO.with(|active| *active.borrow_mut() = Some(table));
        StemMemoGuard(self)
    }
}

impl Drop for StemMemoGuard<'_> {
    fn drop(&mut self) {
        if let Some(table) = ACTIVE_STEM_MEMO.with(|active| active.borrow_mut().take()) {
            self.0 .0 = table;
        }
    }
}

/// The batch interners a store keeps between its document batches, so a corpus loaded in
/// consecutive batches stems each distinct word once rather than once per batch and rayon
/// split. While a batch runs, each split takes an idle interner (the one that has seen the
/// most words) and returns it when done, so a worker's later splits reuse what its earlier
/// ones stemmed. When the batch ends ([`StemMemoPool::trim`]) only the largest is kept,
/// bounded by [`STEM_MEMO_CAPACITY`] words. With keeping turned off
/// ([`StemMemoPool::set_keep`]) each split's interner is freed with the split.
#[derive(Default)]
pub(crate) struct StemMemoPool {
    pub(super) idle: crate::core::sync::Mutex<Vec<StemMemo>>,
    pub(super) release: std::sync::atomic::AtomicBool,
}

impl StemMemoPool {
    /// The idle interner that has seen the most words, or a new one; it goes back to the pool
    /// when the result drops.
    pub(crate) fn take(&self) -> PooledStemMemo<'_> {
        let memo = {
            let mut idle = self.idle.lock();
            let largest = (0..idle.len()).max_by_key(|&i| idle[i].0.words.len());
            largest.map(|i| idle.swap_remove(i))
        };
        PooledStemMemo {
            pool: self,
            memo: memo.unwrap_or_default(),
        }
    }

    /// Keeps the idle interners that have seen the most words, together at most
    /// [`STEM_MEMO_CAPACITY`] words (always the largest one); the others are freed.
    pub(crate) fn trim(&self) {
        let mut idle = std::mem::take(&mut *self.idle.lock());
        idle.sort_unstable_by_key(|memo| std::cmp::Reverse(memo.0.words.len()));
        let mut words = 0;
        let keep = idle
            .iter()
            .position(|memo| {
                words += memo.0.words.len();
                words > STEM_MEMO_CAPACITY
            })
            .unwrap_or(idle.len())
            .max(1)
            .min(idle.len());
        let freed = idle.split_off(keep);
        self.idle.lock().append(&mut idle);
        // The freed interners drop here, with the lock released.
        drop(freed);
    }

    /// Whether interners outlive their batch; turning it off frees the idle ones now.
    pub(crate) fn set_keep(&self, keep: bool) {
        self.release
            .store(!keep, std::sync::atomic::Ordering::Relaxed);
        if !keep {
            let freed = std::mem::take(&mut *self.idle.lock());
            drop(freed);
        }
    }
}

pub(crate) struct PooledStemMemo<'a> {
    pub(super) pool: &'a StemMemoPool,
    pub(super) memo: StemMemo,
}

impl PooledStemMemo<'_> {
    pub(crate) fn install(&mut self) -> StemMemoGuard<'_> {
        self.memo.install()
    }
}

impl Drop for PooledStemMemo<'_> {
    fn drop(&mut self) {
        let memo = std::mem::take(&mut self.memo);
        if self.pool.release.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.pool.idle.lock().push(memo);
    }
}
