// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Boolean, phrase and counting lexical queries answered from a mapped version 4 checkpoint
//! without building the live store.
//!
//! Required clauses are intersected by leapfrogging their posting cursors from the rarest list,
//! each cursor skipping whole blocks through the list's skip table. A phrase is its words'
//! conjunction followed by a check of their positions. Scores are those of the exhaustive
//! scorer to the bit: the clause weights `Bm25::weight` at the chunk's length, summed in the
//! query's clause order from zero. Top-k pruning drops a document only when a conservative
//! bound of its score is strictly below the current k-th best exact score, so ties are kept and
//! decided by the exhaustive tie order (document id, then chunk index).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::wand::{sum_bound, Bound};
use super::*;
use crate::core::documents::image::ServingImage;
use crate::core::documents::lists::{Cursor, ListRef, PositionIter, NO_DOC};
use crate::core::documents::search::LexicalSource;

/// One clause with its cursors: a single word, or a phrase's words rarest first.
struct Rt<'a> {
    /// The clause's index in the query, the order scores are summed in.
    clause: usize,
    phrase: bool,
    cursors: Vec<Cursor<'a>>,
    /// The phrase word each cursor reads.
    word: Vec<usize>,
    bound: Option<Bound>,
    /// A bound of the clause's weight anywhere in its lists.
    ceiling: f64,
    /// The phrase's frequency at the document the last successful seek reached.
    frequency: u32,
    iters: Vec<std::iter::Peekable<PositionIter<'a>>>,
}

impl<'a> Rt<'a> {
    fn new(clause: usize, phrase: bool, lists: &[ListRef<'a>], idf: f64, shape: &Shape) -> Self {
        let mut order: Vec<usize> = (0..lists.len()).collect();
        order.sort_by_key(|&i| lists[i].count);
        let cursors: Vec<_> = order.iter().map(|&i| Cursor::open(lists[i])).collect();
        let bound = Bound::new(idf, shape.average, shape.bm25);
        let ceiling = bound.map_or(f64::INFINITY, |bound| {
            let frequency = cursors.iter().map(|c| c.list_bounds().0).min().unwrap_or(0);
            let words = cursors.iter().map(|c| c.list_bounds().1).max().unwrap_or(0);
            bound.at(frequency, words)
        });
        Self {
            clause,
            phrase,
            cursors,
            word: order,
            bound,
            ceiling,
            frequency: 0,
            iters: Vec::new(),
        }
    }

    /// The rarest word's posting count.
    fn rarest(&self) -> usize {
        self.cursors[0].len()
    }

    /// The first document at or after `target` the clause matches, [`NO_DOC`] at the end.
    /// With `exact`, a phrase's occurrences are counted for [`Self::frequency`].
    fn seek(&mut self, target: u32, exact: bool) -> u32 {
        if !self.phrase {
            return self.cursors[0].advance_to(target);
        }
        let mut doc = target;
        loop {
            doc = self.align(doc);
            if doc == NO_DOC {
                return NO_DOC;
            }
            let frequency = self.phrase_frequency(exact);
            if frequency > 0 {
                self.frequency = frequency;
                return doc;
            }
            doc += 1;
        }
    }

    /// The first document at or after `target` that has every word of the clause, without
    /// checking a phrase's positions.
    fn candidate(&mut self, target: u32) -> u32 {
        if self.phrase {
            self.align(target)
        } else {
            self.cursors[0].advance_to(target)
        }
    }

    /// Whether the clause matches the document its cursors are all at; a phrase's frequency is
    /// set for [`Self::frequency`] (counted exactly only with `exact`).
    fn verify(&mut self, exact: bool) -> bool {
        if !self.phrase {
            return true;
        }
        let frequency = self.phrase_frequency(exact);
        self.frequency = frequency;
        frequency > 0
    }

    /// The first document at or after `doc` every word has.
    fn align(&mut self, mut doc: u32) -> u32 {
        'outer: loop {
            for cursor in &mut self.cursors {
                let at = cursor.advance_to(doc);
                if at == NO_DOC {
                    return NO_DOC;
                }
                if at != doc {
                    doc = at;
                    continue 'outer;
                }
            }
            return doc;
        }
    }

    /// The clause's frequency at the document the last seek reached.
    fn frequency(&mut self) -> u32 {
        if self.phrase {
            self.frequency
        } else {
            self.cursors[0].frequency()
        }
    }

    /// Occurrences of the phrase at the document every word cursor is at (one when not
    /// `exact` and there is any).
    fn phrase_frequency(&mut self, exact: bool) -> u32 {
        let words = self.cursors.len();
        self.iters.clear();
        self.iters
            .resize_with(words, || PositionIter::new(&[]).peekable());
        let (mut lead, mut least) = (0usize, u32::MAX);
        for (at, cursor) in self.cursors.iter_mut().enumerate() {
            let word = self.word[at];
            self.iters[word] = PositionIter::new(cursor.position_bytes()).peekable();
            let frequency = cursor.frequency();
            if frequency < least || (frequency == least && word < lead) {
                lead = word;
                least = frequency;
            }
        }
        let lead_bytes = {
            let at = self
                .word
                .iter()
                .position(|&w| w == lead)
                .expect("a lead word");
            self.cursors[at].position_bytes()
        };
        let mut count = 0;
        for position in PositionIter::new(lead_bytes) {
            let Some(start) = position.checked_sub(lead as u32) else {
                continue;
            };
            let matched = self.iters.iter_mut().enumerate().all(|(offset, iter)| {
                if offset == lead {
                    return true;
                }
                let Some(target) = start.checked_add(offset as u32) else {
                    return false;
                };
                while iter.peek().is_some_and(|&p| p < target) {
                    iter.next();
                }
                iter.peek() == Some(&target)
            });
            count += u32::from(matched);
            if matched && !exact {
                break;
            }
        }
        count
    }

    /// A bound of the clause's weight at the document its cursors are at, from their blocks.
    fn block_bound(&self) -> f64 {
        let Some(bound) = self.bound else {
            return f64::INFINITY;
        };
        let (mut frequency, mut words) = (u32::MAX, 0u32);
        for cursor in &self.cursors {
            let (f, w) = cursor.block_bounds();
            frequency = frequency.min(f);
            words = words.max(w);
        }
        bound.at(frequency, words)
    }

    /// A bound of an optional clause's weight at every slot from `doc` through the slot it
    /// also returns: zero before the clause's next posting, else from the block that holds `doc`.
    fn optional_bound(&mut self, doc: u32) -> (f64, u32) {
        let (Some(bound), false) = (self.bound, self.phrase) else {
            return (self.ceiling, NO_DOC);
        };
        let cursor = &mut self.cursors[0];
        if !cursor.advance_block(doc) {
            return (0.0, NO_DOC);
        }
        if cursor.block_first() > doc {
            return (0.0, cursor.block_first() - 1);
        }
        let (frequency, words) = cursor.block_bounds();
        (bound.at(frequency, words), cursor.block_last())
    }

    /// The last slot of the blocks the clause's cursors are in, the lowest of them.
    fn block_last(&self) -> u32 {
        self.cursors
            .iter()
            .map(Cursor::block_last)
            .min()
            .unwrap_or(NO_DOC)
    }
}

/// What every clause's scoring shares.
struct Shape {
    bm25: Bm25,
    average: f64,
}

/// A hit as the top-k heap orders it: greater is worse. Ids are read only to break ties.
struct Worse<'a> {
    score: f64,
    slot: u32,
    image: &'a ServingImage,
}

impl PartialEq for Worse<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Worse<'_> {}
impl PartialOrd for Worse<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Worse<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other.score.total_cmp(&self.score).then_with(|| {
            if self.slot == other.slot {
                return Ordering::Equal;
            }
            let mine = self.image.slot_order(self.slot as usize);
            let theirs = self.image.slot_order(other.slot as usize);
            mine.cmp(&theirs)
        })
    }
}

struct Top<'a> {
    image: &'a ServingImage,
    k: usize,
    heap: BinaryHeap<Worse<'a>>,
}

impl<'a> Top<'a> {
    fn new(image: &'a ServingImage, k: usize) -> Self {
        Self {
            image,
            k,
            heap: BinaryHeap::with_capacity(k + 1),
        }
    }

    /// The k-th best score so far, once there are k.
    fn floor(&self) -> Option<f64> {
        (self.k > 0 && self.heap.len() == self.k).then(|| self.heap.peek().expect("full").score)
    }

    fn offer(&mut self, score: f64, slot: u32) {
        if self.k == 0 {
            return;
        }
        let hit = Worse {
            score,
            slot,
            image: self.image,
        };
        if self.heap.len() < self.k {
            self.heap.push(hit);
        } else if hit < *self.heap.peek().expect("full") {
            self.heap.pop();
            self.heap.push(hit);
        }
    }

    fn finish(self) -> Vec<LexicalHit> {
        // Ascending in `Worse` is best first.
        let image = self.image;
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|hit| {
                let (id, chunk) = image.slot_order(hit.slot as usize);
                LexicalHit {
                    document_id: id.to_owned(),
                    chunk_index: chunk,
                    score: hit.score,
                }
            })
            .collect()
    }
}

/// The score of the chunk in `slot` as the exhaustive scorer sums it.
fn exact_score(
    query: &LexicalQuery,
    frequencies: &[u32],
    idfs: &[f64],
    word_count: u32,
    shape: &Shape,
) -> Result<f64> {
    let norm = shape.bm25.norm(f64::from(word_count), shape.average);
    let mut score = 0.0;
    for ((clause, &frequency), &idf) in query.clauses.iter().zip(frequencies).zip(idfs) {
        if clause.occur != Occur::MustNot && frequency > 0 {
            score += shape.bm25.weight(idf, f64::from(frequency), norm);
        }
    }
    ensure!(
        score.is_finite(),
        "lexical score is nonfinite; reduce BM25 parameters"
    );
    Ok(score)
}

/// Answers `query` from `image`, or `None` when this executor does not serve it (the caller
/// then builds the live store). `ranked` holds the hits of a bare disjunction already ranked
/// by the checkpoint's pruned ranker.
pub(in crate::core::documents) fn execute(
    image: &ServingImage,
    query: &LexicalQuery,
    options: &LexicalOptions,
    bm25: Bm25,
    ranked: Option<Vec<LexicalHit>>,
) -> Result<Option<LexicalResults>> {
    if options.execution == LexicalExecution::Exhaustive
        || !image.has_skips()
        || !image.has_impacts()
        || image.chunk_count() != image.document_total()
    {
        return Ok(None);
    }
    let form = match (image.term_count(0) > 0, image.term_count(1) > 0) {
        (true, true) => return Ok(None),
        (false, true) => 1,
        _ => 0,
    };
    let empty = LexicalResults {
        hits: Vec::new(),
        count: options.count.then_some(0),
    };
    if image.chunk_count() == 0 || !query.has_positive() {
        return Ok(Some(empty));
    }
    for clause in &query.clauses {
        let words = &clause.words[form];
        let shaped = if clause.phrase {
            image.has_positions() && words.len() > 1
        } else {
            words.len() == 1
        };
        if !shaped {
            return Ok(None);
        }
    }
    let n = image.chunk_count() as f64;
    let average = (image.word_count() as f64 / n.max(1.0)).max(1.0);
    let shape = Shape { bm25, average };

    let mut idfs = Vec::with_capacity(query.clauses.len());
    let mut required = Vec::new();
    let mut excluded = Vec::new();
    let mut optional = Vec::new();
    for (index, clause) in query.clauses.iter().enumerate() {
        let lists: Vec<Option<ListRef<'_>>> = clause.words[form]
            .iter()
            .map(|word| image.list(form, word))
            .collect();
        let idf: f64 = lists
            .iter()
            .map(|list| Bm25::idf(n, list.map_or(0, |l| l.count as usize) as f64))
            .sum();
        idfs.push(idf);
        let Some(lists) = lists.into_iter().collect::<Option<Vec<_>>>() else {
            match clause.occur {
                Occur::Must => return Ok(Some(empty)),
                _ => continue,
            }
        };
        let rt = Rt::new(index, clause.phrase, &lists, idf, &shape);
        match clause.occur {
            Occur::Must => required.push(rt),
            Occur::MustNot => excluded.push(rt),
            Occur::Should => optional.push(rt),
        }
    }
    if required.is_empty() && optional.is_empty() {
        return Ok(Some(empty));
    }
    let all_terms = optional.iter().all(|rt| !rt.phrase);
    if required.is_empty() && optional.len() == 1 && ranked.is_none() {
        // A lone optional clause is what the documents must match.
        required = std::mem::take(&mut optional);
    }
    let k = options.k;
    let mut run = Run {
        image,
        query,
        shape: &shape,
        idfs: &idfs,
        k,
        count: options.count,
        frequencies: vec![0; query.clauses.len()],
    };
    let solo =
        required.len() == 1 && !required[0].phrase && excluded.is_empty() && optional.is_empty();
    let solo_count = if solo { required[0].rarest() } else { 0 };
    if solo {
        if k == 0 {
            return Ok(Some(LexicalResults {
                hits: Vec::new(),
                count: options.count.then_some(solo_count),
            }));
        }
        // Every posting counts, so the ranking may skip blocks.
        run.count = false;
    }
    let (hits, count) = if required.is_empty() {
        if excluded.is_empty() && all_terms && (ranked.is_some() || k == 0) {
            // Ranked by the checkpoint's pruned ranker, or only counted.
            let count = options
                .count
                .then(|| union_count(&mut optional, image.slot_count()));
            return Ok(Some(LexicalResults {
                hits: ranked.unwrap_or_default(),
                count,
            }));
        }
        run.union_scan(optional, excluded)?
    } else {
        run.conjunction(required, excluded, optional)?
    };
    Ok(Some(LexicalResults {
        hits,
        count: options
            .count
            .then_some(if solo { solo_count } else { count }),
    }))
}

struct Run<'a, 'q> {
    image: &'a ServingImage,
    query: &'q LexicalQuery,
    shape: &'q Shape,
    idfs: &'q [f64],
    k: usize,
    count: bool,
    frequencies: Vec<u32>,
}

impl<'a> Run<'a, '_> {
    /// Scores the chunk in `slot` from the frequencies set in `self.frequencies`.
    fn score(&self, slot: u32) -> Result<f64> {
        exact_score(
            self.query,
            &self.frequencies,
            self.idfs,
            self.image.slot_word_count(slot as usize),
            self.shape,
        )
    }

    /// Documents every required clause matches, none excluded, scored with the optional ones.
    fn conjunction(
        &mut self,
        mut required: Vec<Rt<'a>>,
        mut excluded: Vec<Rt<'a>>,
        mut optional: Vec<Rt<'a>>,
    ) -> Result<(Vec<LexicalHit>, usize)> {
        required.sort_by_key(Rt::rarest);
        let scoring = self.k > 0;
        let mut top = Top::new(self.image, self.k);
        let mut count = 0usize;
        let clauses = required.len() + optional.len();
        // The most the optional clauses can add anywhere.
        let optional_ceiling: f64 = optional.iter().map(|rt| rt.ceiling).sum();
        let all_bounded = required
            .iter()
            .chain(&optional)
            .all(|rt| rt.bound.is_some());
        let list_ceiling = sum_bound(
            required.iter().map(|rt| rt.ceiling).sum::<f64>() + optional_ceiling,
            clauses,
        );
        let mut target = 0u32;
        'docs: loop {
            // Nothing further can reach the k-th best score.
            if scoring && !self.count && all_bounded {
                if let Some(floor) = top.floor() {
                    if list_ceiling < floor {
                        break;
                    }
                }
            }
            let mut doc = required[0].candidate(target);
            if doc == NO_DOC {
                break;
            }
            loop {
                let mut moved = false;
                for rt in required.iter_mut().skip(1) {
                    let at = rt.candidate(doc);
                    if at == NO_DOC {
                        break 'docs;
                    }
                    if at != doc {
                        doc = at;
                        moved = true;
                        break;
                    }
                }
                if !moved {
                    break;
                }
                let at = required[0].candidate(doc);
                if at == NO_DOC {
                    break 'docs;
                }
                doc = at;
            }
            // Every required clause's words are in `doc`.
            let mut pruned = false;
            if scoring && all_bounded {
                if let Some(floor) = top.floor() {
                    let mut sum: f64 = required.iter().map(Rt::block_bound).sum();
                    let mut valid = required.iter().map(Rt::block_last).min().unwrap_or(doc);
                    for rt in &mut optional {
                        let (weight, until) = rt.optional_bound(doc);
                        sum += weight;
                        valid = valid.min(until);
                    }
                    if sum_bound(sum, clauses) < floor {
                        if !self.count {
                            // No document in these blocks can enter the top k.
                            target = valid.max(doc) + 1;
                            continue 'docs;
                        }
                        pruned = true;
                    }
                }
            }
            if !required.iter_mut().all(|rt| rt.verify(scoring && !pruned)) {
                target = doc + 1;
                continue;
            }
            if pruned {
                // Counted, but not scored.
                if !excluded.iter_mut().any(|rt| rt.seek(doc, false) == doc) {
                    count += 1;
                }
                target = doc + 1;
                continue;
            }
            if excluded.iter_mut().any(|rt| rt.seek(doc, false) == doc) {
                target = doc + 1;
                continue;
            }
            count += 1;
            if scoring {
                for rt in &mut required {
                    self.frequencies[rt.clause] = rt.frequency();
                }
                for rt in &mut optional {
                    let at = rt.seek(doc, true);
                    self.frequencies[rt.clause] = if at == doc { rt.frequency() } else { 0 };
                }
                let score = self.score(doc)?;
                top.offer(score, doc);
            }
            target = doc + 1;
        }
        Ok((top.finish(), count))
    }

    /// Documents any of the optional clauses matches and none of the excluded ones does, in
    /// slot order, without pruning.
    fn union_scan(
        &mut self,
        mut optional: Vec<Rt<'a>>,
        mut excluded: Vec<Rt<'a>>,
    ) -> Result<(Vec<LexicalHit>, usize)> {
        let exact = self.k > 0;
        let mut top = Top::new(self.image, self.k);
        let mut count = 0usize;
        let mut current: Vec<u32> = optional.iter_mut().map(|rt| rt.seek(0, exact)).collect();
        loop {
            let doc = current.iter().copied().min().unwrap_or(NO_DOC);
            if doc == NO_DOC {
                break;
            }
            if !excluded.iter_mut().any(|rt| rt.seek(doc, false) == doc) {
                count += 1;
                if exact {
                    for (rt, &at) in optional.iter_mut().zip(&current) {
                        self.frequencies[rt.clause] = if at == doc { rt.frequency() } else { 0 };
                    }
                    let score = self.score(doc)?;
                    top.offer(score, doc);
                }
            }
            for (rt, at) in optional.iter_mut().zip(current.iter_mut()) {
                if *at == doc {
                    *at = rt.seek(doc + 1, exact);
                }
            }
        }
        Ok((top.finish(), count))
    }
}

/// Documents with any of the term clauses' words: the largest list's postings, plus the
/// documents of the others that it lacks, probed through its skip table; or, when the lists
/// are of similar size, a bitmap over all slots.
fn union_count(optional: &mut [Rt<'_>], slots: usize) -> usize {
    if optional.len() == 1 {
        return optional[0].cursors[0].len();
    }
    optional.sort_by_key(|rt| std::cmp::Reverse(rt.cursors[0].len()));
    let (big, rest) = optional.split_first_mut().expect("two or more lists");
    let rest_total: usize = rest.iter().map(|rt| rt.cursors[0].len()).sum();
    let big_len = big.cursors[0].len();
    if big_len >= 4 * rest_total {
        let mut count = big_len;
        loop {
            let doc = rest
                .iter()
                .map(|rt| rt.cursors[0].doc())
                .min()
                .unwrap_or(NO_DOC);
            if doc == NO_DOC {
                return count;
            }
            if big.cursors[0].advance_to(doc) != doc {
                count += 1;
            }
            for rt in rest.iter_mut() {
                if rt.cursors[0].doc() == doc {
                    rt.cursors[0].next();
                }
            }
        }
    }
    let mut bits = vec![0u64; slots.div_ceil(64)];
    for rt in optional.iter_mut() {
        rt.cursors[0].for_each_block(|block| {
            for &doc in block {
                let doc = doc as usize;
                bits[doc / 64] |= 1 << (doc % 64);
            }
        });
    }
    bits.iter().map(|word| word.count_ones() as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::super::tests::insert;
    use super::*;
    use crate::core::documents::image::{encode, Backing};
    use proptest::prelude::*;

    fn corpus(docs: &[Vec<u8>], positions: bool) -> DocumentStore {
        let mut store = DocumentStore::default();
        for (i, words) in docs.iter().enumerate() {
            let text: Vec<String> = words.iter().map(|w| format!("w{w}")).collect();
            insert(
                &mut store,
                &format!("d{i:05}"),
                &text.join(" "),
                positions,
                true,
            );
        }
        store
    }

    fn image_of(store: &DocumentStore) -> ServingImage {
        let bytes = encode(store, 7, 64).unwrap().expect("checkpointable");
        ServingImage::new(Backing::Owned(bytes)).unwrap()
    }

    /// Whether the executor served the query.
    fn assert_same(
        image: &ServingImage,
        live: &DocumentStore,
        query: &LexicalQuery,
        options: &LexicalOptions,
    ) -> bool {
        let reference = super::super::execute(
            live,
            query,
            &LexicalOptions {
                execution: LexicalExecution::Exhaustive,
                ..options.clone()
            },
        )
        .unwrap();
        let bm25 = prepare(options).unwrap();
        let Some(served) = served::execute(image, query, options, bm25, None).unwrap() else {
            return false;
        };
        assert_eq!(served.count, reference.count, "{query:?} {options:?}");
        assert_eq!(
            served.hits.len(),
            reference.hits.len(),
            "{query:?} {options:?}"
        );
        for (a, b) in served.hits.iter().zip(&reference.hits) {
            assert_eq!(
                (&a.document_id, a.chunk_index, a.score.to_bits()),
                (&b.document_id, b.chunk_index, b.score.to_bits()),
                "{query:?} {options:?}"
            );
        }
        true
    }

    fn query_text(parts: &[(u8, Vec<u8>)]) -> String {
        parts
            .iter()
            .map(|(kind, words)| {
                let words: Vec<String> = words.iter().map(|w| format!("w{w}")).collect();
                match kind % 6 {
                    0 => words[0].clone(),
                    1 => format!("+{}", words[0]),
                    2 => format!("-{}", words[0]),
                    3 => format!("\"{}\"", words.join(" ")),
                    4 => format!("+\"{}\"", words.join(" ")),
                    _ => format!("-\"{}\"", words.join(" ")),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_executor_serves_every_query_shape_it_claims() {
        let docs: Vec<Vec<u8>> = (0..300u32)
            .map(|i| {
                (0..12)
                    .map(|j| ((i * 7 + j * j + j / 3) % 9) as u8)
                    .collect()
            })
            .collect();
        let live = corpus(&docs, true);
        let image = image_of(&live);
        let options = LexicalOptions {
            k: 5,
            count: true,
            ..Default::default()
        };
        for text in [
            "w1 w2",
            "+w1 +w2",
            "+w1 -w2",
            "+w1 w2 w3",
            "\"w1 w2\"",
            "+\"w1 w2\" +w3",
            "w4 -w5",
            "w4 -\"w1 w2\"",
            "\"w1 w2\" w3",
            "\"w1 w2\" \"w3 w4\"",
            "w1",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            let bm25 = prepare(&options).unwrap();
            let served = served::execute(&image, &query, &options, bm25, None)
                .unwrap()
                .unwrap_or_else(|| panic!("{text} is not served"));
            assert!(served.count.unwrap() > 0, "{text}");
            assert!(assert_same(&image, &live, &query, &options), "{text}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        #[test]
        fn served_queries_match_the_exhaustive_scorer_to_the_bit(
            docs in prop::collection::vec(prop::collection::vec(0u8..9, 2..40), 120..420),
            queries in prop::collection::vec(
                prop::collection::vec((0u8..6, prop::collection::vec(0u8..11, 2..4)), 1..4),
                1..6,
            ),
        ) {
            let live = corpus(&docs, true);
            let image = image_of(&live);
            // The mapping rebuilds the store it was written from, positions included.
            let rebuilt = image.hydrate(None).unwrap();
            for parts in &queries {
                let text = query_text(parts);
                let Ok(query) = LexicalQuery::parse(&text) else { continue };
                for k in [0usize, 1, 3, 10] {
                    for count in [false, true] {
                        let options = LexicalOptions { k, count, ..Default::default() };
                        prop_assert!(assert_same(&image, &live, &query, &options));
                        let expected = super::super::execute(&live, &query, &LexicalOptions {
                            execution: LexicalExecution::Exhaustive, ..options.clone() }).unwrap();
                        let again = super::super::execute(&rebuilt, &query, &LexicalOptions {
                            execution: LexicalExecution::Exhaustive, ..options.clone() }).unwrap();
                        prop_assert_eq!(&expected, &again);
                    }
                }
            }
        }
    }
}
