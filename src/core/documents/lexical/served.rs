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
use crate::core::documents::impact::{prefetch, Dense};
use crate::core::documents::lists::{
    dense_word, holds, Cursor, ListRef, PositionIter, RankTable, NO_DOC, RUN_BLOCKS,
};
use crate::core::documents::postings::CHECKPOINT_BLOCK;
use crate::core::documents::search::LexicalSource;

/// One clause with its cursors: a single word, or a phrase's words rarest first.
struct Rt<'a> {
    /// The clause's index in the query, the order scores are summed in.
    clause: usize,
    /// Later clauses of the same single word, which [`merge_twins`] folded into this one: they
    /// share its postings and weight.
    twins: Vec<usize>,
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
            twins: Vec::new(),
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
        if self.bound.is_none() || self.phrase {
            return (self.ceiling, NO_DOC);
        }
        let (weight, until, live) = self.frontier_bound(doc, false);
        (weight, if live { until } else { NO_DOC })
    }

    /// A bound of a term clause's weight in the block its cursor is in, from the block's
    /// frontier; a phrase's is that of [`Self::block_bound`].
    fn tight_block_bound(&self) -> f64 {
        let (Some(bound), false) = (self.bound, self.phrase) else {
            return self.block_bound();
        };
        self.cursors[0].block_weight(bound)
    }

    /// For a term clause with a bound: a bound of its weight at every slot from `doc` through the
    /// slot it also returns, from the frontier of the run (`runs`) or of the block that holds
    /// `doc`; zero up to the slot before its next posting. The cursor ends when it has nothing
    /// at or after `doc`, which the second value `false` reports.
    fn frontier_bound(&mut self, doc: u32, runs: bool) -> (f64, u32, bool) {
        let bound = self.bound.expect("a bounded term");
        let cursor = &mut self.cursors[0];
        if !cursor.advance_block(doc) {
            return (0.0, NO_DOC, false);
        }
        if cursor.block_first() > doc {
            return (0.0, cursor.block_first() - 1, true);
        }
        if runs && cursor.runs() >= 2 {
            let run = cursor.run();
            return (cursor.run_weight(run, bound), cursor.run_last(run), true);
        }
        (cursor.block_weight(bound), cursor.block_last(), true)
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

/// Sorts term clauses rarest first and folds each clause whose word repeats an earlier one's
/// into that one's [`Rt::twins`]: a conjunction reads each list once.
fn merge_twins(terms: &mut Vec<Rt<'_>>) {
    let key = |rt: &Rt<'_>| {
        (
            rt.rarest(),
            rt.cursors[0].list_bytes().as_ptr() as usize,
            rt.clause,
        )
    };
    terms.sort_by_key(key);
    let mut kept: Vec<Rt<'_>> = Vec::with_capacity(terms.len());
    for rt in terms.drain(..) {
        match kept.last_mut() {
            Some(last)
                if last.cursors[0].list_bytes().as_ptr() == rt.cursors[0].list_bytes().as_ptr() =>
            {
                last.twins.push(rt.clause);
            }
            _ => kept.push(rt),
        }
    }
    *terms = kept;
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
            let mine = self.image.slot_key(self.slot as usize);
            let theirs = self.image.slot_key(other.slot as usize);
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
/// then builds the live store). `ranker` ranks a bare disjunction with the checkpoint's windowed
/// ranker, for the shapes the block-max executor does not take; its `None` declines the query.
pub(in crate::core::documents) fn execute(
    image: &ServingImage,
    query: &LexicalQuery,
    options: &LexicalOptions,
    bm25: Bm25,
    ranker: Option<&dyn Fn() -> Result<Option<Vec<LexicalHit>>>>,
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
    // A lone word's count is its posting count; no cursor, bound or block is read.
    if let ([clause], 0, true) = (query.clauses.as_slice(), options.k, options.count) {
        if !clause.phrase && clause.occur != Occur::MustNot {
            let count = image
                .list(form, &clause.words[form][0])
                .map_or(0, |list| list.count as usize);
            return Ok(Some(LexicalResults {
                hits: Vec::new(),
                count: Some(count),
            }));
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
        let mut rt = Rt::new(index, clause.phrase, &lists, idf, &shape);
        for cursor in &mut rt.cursors {
            let table = image.rank_table(cursor, shape.bm25, shape.average);
            cursor.use_ranks(table);
        }
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
    let k = options.k;
    // One long bounded term is ranked by its runs' frontiers; more terms go to the windowed
    // ranker, which measured an order of magnitude faster than document-at-a-time WAND here.
    let pruned_union = required.is_empty()
        && k > 0
        && all_terms
        && optional.len() == 1
        && excluded.is_empty()
        && optional[0].cursors[0].block_count() >= 2
        && optional.iter().all(|rt| rt.bound.is_some());
    if required.is_empty() && optional.len() == 1 && ranker.is_none() && !pruned_union {
        // A lone optional clause is what the documents must match.
        required = std::mem::take(&mut optional);
    }
    let mut run = Run {
        image,
        query,
        shape: &shape,
        idfs: &idfs,
        k,
        count: options.count,
        frequencies: vec![0; query.clauses.len()],
        word_counts: image.word_counts(),
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
    let lone_phrase = excluded.is_empty()
        && required.len() + optional.len() == 1
        && required
            .iter()
            .chain(&optional)
            .all(|rt| rt.phrase && rt.cursors.len() >= 2);
    if lone_phrase {
        let mut rt = required
            .pop()
            .or_else(|| optional.pop())
            .expect("one clause");
        let (hits, count) = run.phrase_top(&mut rt)?;
        return Ok(Some(LexicalResults {
            hits,
            count: options.count.then_some(count),
        }));
    }
    if k == 0
        && required.len() >= 2
        && excluded.is_empty()
        && optional.is_empty()
        && required.iter().all(|rt| !rt.phrase)
    {
        merge_twins(&mut required);
        let count = if required.len() == 1 {
            required[0].rarest()
        } else {
            intersect_count(image, &mut required)
        };
        return Ok(Some(LexicalResults {
            hits: Vec::new(),
            count: options.count.then_some(count),
        }));
    }
    let terms_only = required.len() >= 2
        && k > 0
        && excluded.is_empty()
        && optional.is_empty()
        && required.iter().all(|rt| !rt.phrase && rt.bound.is_some());
    if terms_only {
        merge_twins(&mut required);
        let (hits, count) = run.terms_top(&mut required)?;
        return Ok(Some(LexicalResults {
            hits,
            count: options.count.then_some(count),
        }));
    }
    let (hits, count) = if required.is_empty() {
        if pruned_union {
            let count = options.count.then(|| optional[0].rarest());
            let hits = run.single_top(optional.pop().expect("one term"))?;
            return Ok(Some(LexicalResults { hits, count }));
        }
        if excluded.is_empty() && all_terms && (ranker.is_some() || k == 0) {
            // Ranked by the checkpoint's windowed ranker, or only counted.
            let hits = match ranker {
                Some(rank) => match rank()? {
                    Some(hits) => hits,
                    None => return Ok(None),
                },
                None => Vec::new(),
            };
            let count = options
                .count
                .then(|| union_count(&mut optional, image.slot_count()));
            return Ok(Some(LexicalResults { hits, count }));
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
    /// Per slot its word count, `u16::MAX` where the slot table holds it.
    word_counts: &'a [u16],
}

impl<'a> Run<'a, '_> {
    /// The word count of the chunk in `slot`.
    fn words(&self, slot: u32) -> u32 {
        match self.word_counts[slot as usize] {
            u16::MAX => self.image.slot_word_count(slot as usize),
            short => u32::from(short),
        }
    }

    /// Scores the chunk in `slot` from the frequencies set in `self.frequencies`.
    fn score(&self, slot: u32) -> Result<f64> {
        exact_score(
            self.query,
            &self.frequencies,
            self.idfs,
            self.words(slot),
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
                    let mut extra = 0.0;
                    for rt in &mut optional {
                        let (weight, until) = rt.optional_bound(doc);
                        extra += weight;
                        valid = valid.min(until);
                    }
                    if sum_bound(sum + extra, clauses) >= floor {
                        sum = required.iter().map(Rt::tight_block_bound).sum();
                    }
                    sum += extra;
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

    /// The top `k` of the documents every term clause has, and with `self.count` their number:
    /// the rarest list a block at a time, filtered through the others (see [`keep_common`]).
    /// A block whose bound, with the bounds of the other lists at it, is below the k-th best
    /// score is skipped before the other lists are read, or when counting, filtered but not
    /// scored. `terms` is sorted rarest first and every clause has a bound.
    fn terms_top(&mut self, terms: &mut [Rt<'a>]) -> Result<(Vec<LexicalHit>, usize)> {
        let lists = terms.len();
        let clauses = terms.iter().map(|rt| 1 + rt.twins.len()).sum();
        // Each list's clauses, and how many times its weight enters a score.
        let ids: Vec<Vec<usize>> = terms
            .iter()
            .map(|rt| {
                std::iter::once(rt.clause)
                    .chain(rt.twins.iter().copied())
                    .collect()
            })
            .collect();
        let times: Vec<f64> = ids.iter().map(|ids| ids.len() as f64).collect();
        let ceiling = sum_bound(
            terms.iter().zip(&times).map(|(rt, t)| rt.ceiling * t).sum(),
            clauses,
        );
        let mut top = Top::new(self.image, self.k);
        let mut count = 0usize;
        let mut slots = [0u32; CHECKPOINT_BLOCK];
        let tables = frequency_tables(self.image, &terms[1..]);
        // The other lists in the order candidates are filtered through them: those with a
        // frequency table first, a lookup per candidate, then those whose blocks are decoded.
        let mut sequence: Vec<usize> = (0..lists - 1).collect();
        sequence.sort_by_key(|&l| tables[l].is_none());
        // Row 0: each candidate's index in the lead block; row `p`: its frequency in list
        // `sequence[p - 1]`.
        let mut rows = vec![[0u32; CHECKPOINT_BLOCK]; lists];
        let (lead, rest) = terms.split_first_mut().expect("two or more lists");
        let lead_bound = lead.bound.expect("a bounded term");
        'blocks: loop {
            let floor = top.floor();
            if !self.count && floor.is_some_and(|floor| ceiling < floor) {
                break;
            }
            let cursor = &mut lead.cursors[0];
            if cursor.is_end() {
                break;
            }
            let (first, last) = (cursor.block_first(), cursor.block_last());
            let mut scored = true;
            if let Some(floor) = floor {
                let mut sum = cursor.block_weight(lead_bound) * times[0];
                for (other, t) in rest.iter_mut().zip(&times[1..]) {
                    let c = &mut other.cursors[0];
                    if !c.advance_block(first) {
                        break 'blocks;
                    }
                    let bound = other.bound.expect("a bounded term");
                    let weight = if c.block_last() >= last {
                        c.block_weight(bound)
                    } else {
                        c.span_weight(last, bound, 8).unwrap_or(other.ceiling)
                    };
                    sum += weight * t;
                }
                scored = sum_bound(sum, clauses) >= floor;
            }
            let mut live = true;
            if scored || self.count {
                let cursor = &mut lead.cursors[0];
                let block = cursor.block_slots();
                let mut n = block.len();
                slots[..n].copy_from_slice(block);
                if !scored {
                    // Counted only: the dense lists' bitmaps answer without decoding them.
                    let (kept, more) = keep_counted(rest, &tables, &mut slots, n);
                    count += kept;
                    if !more || !lead.cursors[0].advance_block(last.saturating_add(1)) {
                        break;
                    }
                    continue;
                }
                for (index, value) in rows[0][..n].iter_mut().enumerate() {
                    *value = index as u32;
                }
                for (p, &l) in sequence.iter().enumerate() {
                    let (kept, more) = match &tables[l] {
                        Some(table) => (keep_tabled(table, &mut slots, n, &mut rows, p + 1), true),
                        None => keep_common(
                            &mut rest[l].cursors[0],
                            &mut slots,
                            n,
                            &mut rows,
                            Record::Frequency(p + 1),
                        ),
                    };
                    n = kept;
                    live &= more;
                    if n == 0 {
                        break;
                    }
                }
                count += n;
                if n > 0 {
                    // The candidates are known: start loading their word counts together.
                    for &slot in &slots[..n] {
                        prefetch(self.word_counts, slot as usize);
                    }
                    let frequencies = lead.cursors[0].block_frequencies();
                    for (m, &slot) in slots[..n].iter().enumerate() {
                        let lead_frequency = frequencies[rows[0][m] as usize];
                        if rows[1..].iter().any(|row| row[m] == SATURATED) {
                            // A tabled list holds this candidate fifteen times or more: bound
                            // its weight by the list's largest frequency, and read the list only
                            // when the bound reaches the k-th best score.
                            if let Some(floor) = top.floor() {
                                let words = self.words(slot);
                                let mut sum = lead_bound.at(lead_frequency, words) * times[0];
                                for (p, &l) in sequence.iter().enumerate() {
                                    let other = &rest[l];
                                    let bound = other.bound.expect("a bounded term");
                                    let frequency = match rows[p + 1][m] {
                                        SATURATED => other.cursors[0].list_bounds().0,
                                        found => found,
                                    };
                                    sum += bound.at(frequency, words) * times[l + 1];
                                }
                                if sum_bound(sum, clauses) < floor {
                                    continue;
                                }
                            }
                            for (p, &l) in sequence.iter().enumerate() {
                                if rows[p + 1][m] == SATURATED {
                                    let cursor = &mut rest[l].cursors[0];
                                    let at = cursor.advance_to(slot);
                                    debug_assert_eq!(at, slot, "a tabled slot is in its list");
                                    rows[p + 1][m] = cursor.frequency();
                                }
                            }
                        }
                        for &id in &ids[0] {
                            self.frequencies[id] = lead_frequency;
                        }
                        for (p, &l) in sequence.iter().enumerate() {
                            for &id in &ids[l + 1] {
                                self.frequencies[id] = rows[p + 1][m];
                            }
                        }
                        let score = self.score(slot)?;
                        top.offer(score, slot);
                    }
                }
            }
            if !live || !lead.cursors[0].advance_block(last.saturating_add(1)) {
                break;
            }
        }
        Ok((top.finish(), count))
    }

    /// The documents holding the query's one phrase clause, and with `self.count` their number:
    /// its rarest word's list a block at a time, filtered through its other words' lists (see
    /// [`keep_common`]), then each survivor's positions checked. A block whose bound is below
    /// the k-th best score is skipped unread, or when counting, checked but not scored.
    fn phrase_top(&mut self, rt: &mut Rt<'a>) -> Result<(Vec<LexicalHit>, usize)> {
        let words = rt.cursors.len();
        let scoring = self.k > 0;
        let mut top = Top::new(self.image, self.k);
        let mut count = 0usize;
        let mut slots = [0u32; CHECKPOINT_BLOCK];
        // Row 0: each candidate's index in the lead block; rows `2c - 1` and `2c`: where the
        // positions of cursor `c` start and end.
        let mut rows = vec![[0u32; CHECKPOINT_BLOCK]; 2 * words - 1];
        let mut bytes: Vec<&'a [u8]> = vec![&[]; words];
        let mut iters = Vec::with_capacity(words);
        let mut order = Vec::with_capacity(words);
        // A cursor whose word repeats an earlier cursor's list reads nothing itself: its
        // candidates and positions are that cursor's.
        let twin: Vec<Option<usize>> = (0..words)
            .map(|c| {
                let bytes = rt.cursors[c].list_bytes().as_ptr();
                (0..c).find(|&t| rt.cursors[t].list_bytes().as_ptr() == bytes)
            })
            .collect();
        let (lead, rest) = rt
            .cursors
            .split_first_mut()
            .expect("a phrase of two words or more");
        let positions: Vec<&'a [u8]> = std::iter::once(lead.list_positions())
            .chain(rest.iter().map(|c| c.list_positions()))
            .collect();
        while !lead.is_end() {
            let last = lead.block_last();
            let mut scored = scoring;
            if let (true, Some(floor), Some(bound)) = (scoring, top.floor(), rt.bound) {
                // The phrase occurs in a document no more often than its lead word does.
                let (frequency, least) = lead.block_bounds();
                if sum_bound(bound.at(frequency, least), 1) < floor {
                    if !self.count {
                        if !lead.advance_block(last.saturating_add(1)) {
                            break;
                        }
                        continue;
                    }
                    scored = false;
                }
            }
            let block = lead.block_slots();
            let mut n = block.len();
            slots[..n].copy_from_slice(block);
            for (value, index) in rows[0][..n].iter_mut().zip(0u32..) {
                *value = index;
            }
            let mut live = true;
            for (c, other) in rest.iter_mut().enumerate() {
                if twin[c + 1].is_some() {
                    continue;
                }
                let (kept, more) = keep_common(
                    other,
                    &mut slots,
                    n,
                    &mut rows,
                    Record::Positions(2 * c + 1),
                );
                n = kept;
                live &= more;
                if n == 0 {
                    break;
                }
                if c + 2 < words {
                    // Each word kept must stand at its phrase distance from the lead word
                    // before the longer lists are read for a candidate at all; the last one
                    // is left to the full check.
                    let distance = rt.word[c + 1] as i64 - rt.word[0] as i64;
                    n = keep_at_distance(
                        lead,
                        positions[0],
                        positions[c + 1],
                        distance,
                        &mut slots,
                        n,
                        &mut rows[..2 * c + 3],
                        2 * c + 1,
                    );
                    if n == 0 {
                        break;
                    }
                }
            }
            for (m, &slot) in slots[..n].iter().enumerate() {
                let (start, end) = lead.position_range(rows[0][m] as usize);
                bytes[rt.word[0]] = &positions[0][start as usize..end as usize];
                for (c, &word) in rt.word.iter().enumerate().skip(1) {
                    bytes[word] = match twin[c] {
                        Some(0) => bytes[rt.word[0]],
                        source => {
                            let source = source.unwrap_or(c);
                            let (start, end) = (
                                rows[2 * source - 1][m] as usize,
                                rows[2 * source][m] as usize,
                            );
                            &positions[source][start..end]
                        }
                    };
                }
                let occurrences = phrase_occurrences(&bytes, &mut iters, &mut order, scored);
                if occurrences == 0 {
                    continue;
                }
                count += 1;
                if scored {
                    self.frequencies[rt.clause] = occurrences;
                    let score = self.score(slot)?;
                    top.offer(score, slot);
                }
            }
            if !live || !lead.advance_block(last.saturating_add(1)) {
                break;
            }
        }
        Ok((top.finish(), count))
    }

    /// The top `k` of one long term's postings: its runs strongest first by their frontiers, and
    /// within a run the blocks whose own bound reaches the k-th best score, so the floor rises
    /// before the weak runs are reached and most of them are never read.
    ///
    /// A list with a rank table (see `lists::RankTable`) takes its run order and its run and
    /// block bounds from the table, which decodes no frontier at all.
    fn single_top(&mut self, mut rt: Rt<'a>) -> Result<Vec<LexicalHit>> {
        let bound = rt.bound.expect("a bounded term");
        let clause = rt.clause;
        let cursor = &mut rt.cursors[0];
        let blocks = cursor.block_count();
        let mut top = Top::new(self.image, self.k);
        let below = |top: &Top<'_>, weight: f64| {
            top.floor()
                .is_some_and(|floor| sum_bound(weight, 1) < floor)
        };
        let image = self.image;
        if let Some(table) = image.rank_table(cursor, self.shape.bm25, self.shape.average) {
            let idf = bound.idf;
            for &run in table.order.iter() {
                let run = run as usize;
                if below(&top, RankTable::scale(table.runs[run], idf)) {
                    break;
                }
                for block in run * RUN_BLOCKS..((run + 1) * RUN_BLOCKS).min(blocks) {
                    if below(&top, RankTable::scale(table.blocks[block], idf)) {
                        continue;
                    }
                    cursor.goto_block(block);
                    self.offer_block(cursor, clause, &mut top)?;
                }
            }
            return Ok(top.finish());
        }
        let mut runs: Vec<(f64, usize)> = (0..cursor.runs())
            .map(|run| (cursor.run_weight(run, bound), run))
            .collect();
        runs.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (run_bound, run) in runs {
            if below(&top, run_bound) {
                break;
            }
            for block in run * RUN_BLOCKS..((run + 1) * RUN_BLOCKS).min(blocks) {
                cursor.goto_block(block);
                if below(&top, cursor.block_weight(bound)) {
                    continue;
                }
                self.offer_block(cursor, clause, &mut top)?;
            }
        }
        Ok(top.finish())
    }

    /// Scores every posting of the block `cursor` is at, as clause `clause`'s, into `top`.
    fn offer_block(
        &mut self,
        cursor: &mut Cursor<'a>,
        clause: usize,
        top: &mut Top<'a>,
    ) -> Result<()> {
        let (slots, frequencies) = cursor.block_postings();
        for (&slot, &frequency) in slots.iter().zip(frequencies) {
            self.frequencies[clause] = frequency;
            let score = self.score(slot)?;
            top.offer(score, slot);
        }
        Ok(())
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

/// Occurrences of a phrase in one document whose words' positions (as gap bytes, in phrase
/// order) are `words`; one when there is any and not `exact`.
fn phrase_occurrences<'b>(
    words: &[&'b [u8]],
    iters: &mut Vec<std::iter::Peekable<PositionIter<'b>>>,
    order: &mut Vec<usize>,
    exact: bool,
) -> u32 {
    if let [first, second] = words {
        return pair_occurrences(first, second, exact);
    }
    // Any word may lead; the one with the fewest position bytes reads the least. The others
    // are checked fewest bytes first, so a frequent word's positions are read only where the
    // rarer words already line up (each iterator only moves forward, so skipping one for a
    // start that fails earlier loses nothing).
    order.clear();
    order.extend(0..words.len());
    order.sort_by_key(|&w| (words[w].len(), w));
    let lead = order.remove(0);
    iters.clear();
    iters.extend(
        words
            .iter()
            .map(|bytes| PositionIter::new(bytes).peekable()),
    );
    let mut count = 0;
    for position in PositionIter::new(words[lead]) {
        let Some(start) = position.checked_sub(lead as u32) else {
            continue;
        };
        let matched = order.iter().all(|&offset| {
            let iter = &mut iters[offset];
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

/// Keeps, in place, the candidates among `slots[..n]` where some position of the lead word
/// (its index in the lead block in `rows[0]`, its positions in `lead_positions`) has a position
/// of another word `distance` after it (that word's positions at `rows[row]..rows[row + 1]` of
/// `other_positions`), moving every row along: a necessary condition of the phrase, so the full
/// check later sees fewer candidates and the remaining lists fewer probes.
#[allow(clippy::too_many_arguments)]
fn keep_at_distance(
    lead: &mut Cursor<'_>,
    lead_positions: &[u8],
    other_positions: &[u8],
    distance: i64,
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
    rows: &mut [[u32; CHECKPOINT_BLOCK]],
    row: usize,
) -> usize {
    let mut k = 0;
    for m in 0..n {
        let (start, end) = lead.position_range(rows[0][m] as usize);
        let firsts = &lead_positions[start as usize..end as usize];
        let seconds = &other_positions[rows[row][m] as usize..rows[row + 1][m] as usize];
        if !at_distance(firsts, seconds, distance) {
            continue;
        }
        slots[k] = slots[m];
        for values in rows.iter_mut() {
            values[k] = values[m];
        }
        k += 1;
    }
    k
}

/// Whether some position of `firsts` has one of `seconds` exactly `distance` after it.
fn at_distance(firsts: &[u8], seconds: &[u8], distance: i64) -> bool {
    let (mut firsts, mut seconds) = (PositionIter::new(firsts), PositionIter::new(seconds));
    let (Some(mut at), Some(mut next)) = (firsts.next(), seconds.next()) else {
        return false;
    };
    loop {
        let gap = i64::from(next) - i64::from(at);
        let advanced = match gap.cmp(&distance) {
            Ordering::Equal => return true,
            Ordering::Less => seconds.next().map(|position| next = position),
            Ordering::Greater => firsts.next().map(|position| at = position),
        };
        if advanced.is_none() {
            return false;
        }
    }
}

/// [`phrase_occurrences`] of a phrase of two words: one merge of their positions, with no
/// iterator set or order to build per document.
fn pair_occurrences(first: &[u8], second: &[u8], exact: bool) -> u32 {
    let (mut firsts, mut seconds) = (PositionIter::new(first), PositionIter::new(second));
    let (Some(mut at), Some(mut next)) = (firsts.next(), seconds.next()) else {
        return 0;
    };
    let mut count = 0;
    loop {
        // `next` follows `at` when it is one past it; no addition, so no overflow.
        let advanced = if next <= at {
            seconds.next().map(|position| next = position)
        } else if next - at > 1 {
            firsts.next().map(|position| at = position)
        } else {
            count += 1;
            if !exact {
                break;
            }
            firsts.next().map(|position| at = position)
        };
        if advanced.is_none() {
            break;
        }
    }
    count
}

/// What [`keep_common`] records of each kept candidate in `other`, in the rows from the one
/// given (the rows before it are carried along).
#[derive(Clone, Copy)]
enum Record {
    Nothing,
    /// Its frequency.
    Frequency(usize),
    /// The start and end of its positions in the list's positions bytes, in two rows.
    Positions(usize),
}

/// Keeps, in place, the candidates among `slots[..n]` that `other` also holds and moves each
/// kept candidate's entries in `rows` along with it, recording what `record` asks. The candidates ascend; blocks of `other` no candidate
/// falls in are skipped through its skip table, never decoded. The second value is false when
/// `other` ended before the last candidate, so no later candidate can match either.
fn keep_common(
    other: &mut Cursor<'_>,
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
    rows: &mut [[u32; CHECKPOINT_BLOCK]],
    record: Record,
) -> (usize, bool) {
    let carried = match record {
        Record::Nothing => rows.len(),
        Record::Frequency(row) | Record::Positions(row) => row,
    };
    let mut matched = [0u8; CHECKPOINT_BLOCK];
    let mut source = [0u8; CHECKPOINT_BLOCK];
    let (mut i, mut k) = (0usize, 0usize);
    while i < n {
        let wanted = slots[i];
        if other.block_last() < wanted && !other.advance_block(wanted) {
            return (k, false);
        }
        let first = other.block_first();
        if first > wanted {
            while i < n && slots[i] < first {
                i += 1;
            }
            continue;
        }
        let last = other.block_last();
        let from = k;
        // A frequency is written beside each candidate as it is matched: a separate pass over
        // the matched indices vectorizes into 512-bit gathers under `target-cpu=native`, whose
        // frequency license slows the queries that follow on AVX-512 parts.
        let (block, frequencies, row) = match record {
            Record::Frequency(row) => {
                let (block, frequencies) = other.block_postings();
                (block, frequencies, row)
            }
            _ => (other.block_slots(), &[][..], usize::MAX),
        };
        // The candidates this block can hold: `slots[i..end]`.
        let mut end = i + 1;
        while end < n && slots[end] <= last {
            end += 1;
        }
        if (end - i) * 16 < block.len() {
            // Few candidates: a branch-free binary search each.
            let mut j = 0usize;
            let mut at = i;
            while at < end {
                let value = slots[at];
                let mut size = block.len() - j;
                while size > 1 {
                    let half = size / 2;
                    j += half * usize::from(block[j + half] <= value);
                    size -= half;
                }
                slots[k] = value;
                matched[k] = j as u8;
                source[k] = at as u8;
                if let Some(&frequency) = frequencies.get(j) {
                    rows[row][k] = frequency;
                }
                k += usize::from(block[j] == value);
                at += 1;
            }
        } else {
            // Many: a branch-free merge, each step advancing whichever side is behind.
            let mut j = 0usize;
            let mut at = i;
            while at < end && j < block.len() {
                let (value, posting) = (slots[at], block[j]);
                slots[k] = value;
                matched[k] = j as u8;
                source[k] = at as u8;
                if let Some(&frequency) = frequencies.get(j) {
                    rows[row][k] = frequency;
                }
                k += usize::from(value == posting);
                at += usize::from(value <= posting);
                j += usize::from(posting <= value);
            }
        }
        i = end;
        // The carried rows move after the block, a row at a time: moved per candidate across
        // the rows, the copy vectorizes into 512-bit gathers and scatters (see above).
        for row in rows[..carried].iter_mut() {
            for (m, &at) in source.iter().enumerate().take(k).skip(from) {
                row[m] = row[usize::from(at)];
            }
        }
        if k > from {
            match record {
                Record::Nothing | Record::Frequency(_) => {}
                Record::Positions(row) => {
                    for (m, &j) in matched.iter().enumerate().take(k).skip(from) {
                        let (start, end) = other.position_range(usize::from(j));
                        rows[row][m] = start;
                        rows[row + 1][m] = end;
                    }
                }
            }
        }
    }
    (k, true)
}

/// Documents every term clause has: the rarest list's postings a block at a time, each block's
/// slots tested against the dense lists' bitmaps, then filtered through the other lists (see
/// [`keep_common`]); when every list is dense, the bitmaps ANDed a word at a time. `terms` is
/// sorted rarest first.
fn intersect_count(image: &ServingImage, terms: &mut [Rt<'_>]) -> usize {
    let bitmaps: Vec<&[u8]> = terms
        .iter()
        .filter_map(|rt| rt.cursors[0].dense())
        .collect();
    if bitmaps.len() == terms.len() {
        let words = bitmaps.iter().map(|bits| bits.len() / 8).min().unwrap_or(0);
        return (0..words)
            .map(|index| {
                bitmaps
                    .iter()
                    .fold(u64::MAX, |word, bits| word & dense_word(bits, index))
                    .count_ones() as usize
            })
            .sum();
    }
    let tables = frequency_tables(image, &terms[1..]);
    let (lead, rest) = terms.split_first_mut().expect("two or more lists");
    let lead = &mut lead.cursors[0];
    let mut count = 0usize;
    let mut slots = [0u32; CHECKPOINT_BLOCK];
    loop {
        let block = lead.block_slots();
        if block.is_empty() {
            return count;
        }
        let n = block.len();
        slots[..n].copy_from_slice(block);
        let (n, live) = keep_counted(rest, &tables, &mut slots, n);
        count += n;
        if !live || !lead.advance_block(lead.block_last().saturating_add(1)) {
            return count;
        }
    }
}

/// Each clause's direct frequency table (see [`ServingImage::frequency_table`]), when it has one.
fn frequency_tables(image: &ServingImage, terms: &[Rt<'_>]) -> Vec<Option<std::sync::Arc<Dense>>> {
    terms
        .iter()
        .map(|rt| image.frequency_table(rt.cursors[0].list()))
        .collect()
}

/// Keeps, in place, the candidates among `slots[..n]` that `table`'s list holds, carrying
/// `rows[..row]` along and writing each kept candidate's frequency in `rows[row]`: the table's
/// nibble, or [`SATURATED`] for fifteen and more, which the caller reads from the list only for
/// a candidate that can still reach the top k. Returns how many candidates are kept.
fn keep_tabled(
    table: &Dense,
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
    rows: &mut [[u32; CHECKPOINT_BLOCK]],
    row: usize,
) -> usize {
    // The candidates' nibbles are scattered over the table: start loading them all together.
    for &slot in &slots[..n] {
        table.prefetch(slot as usize);
    }
    let mut source = [0u8; CHECKPOINT_BLOCK];
    let (mut i, mut k) = (0usize, 0usize);
    while i < n {
        let slot = slots[i];
        let found = table.get(slot as usize);
        slots[k] = slot;
        source[k] = i as u8;
        rows[row][k] = u32::from(found);
        k += usize::from(found != 0);
        i += 1;
    }
    for carried in rows[..row].iter_mut() {
        for (m, &at) in source.iter().enumerate().take(k) {
            carried[m] = carried[usize::from(at)];
        }
    }
    for frequency in rows[row][..k].iter_mut() {
        if *frequency == 15 {
            *frequency = SATURATED;
        }
    }
    k
}

/// A frequency [`keep_tabled`] leaves unread: fifteen or more.
const SATURATED: u32 = u32::MAX;

/// Keeps the candidates among `slots[..n]` that every list of `rest` holds: tested against the
/// dense lists' bitmaps first, then the other lists' frequency tables (`tables`, one per list
/// of `rest`), then filtered through the remaining lists (see [`keep_common`]). The
/// second value is false when a list ended, so no later candidate can match.
fn keep_counted(
    rest: &mut [Rt<'_>],
    tables: &[Option<std::sync::Arc<Dense>>],
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
) -> (usize, bool) {
    let mut kept = n;
    for rt in rest.iter() {
        if let Some(bits) = rt.cursors[0].dense() {
            let (mut i, mut k) = (0, 0);
            while i < kept {
                let slot = slots[i];
                slots[k] = slot;
                k += usize::from(holds(bits, slot));
                i += 1;
            }
            kept = k;
        }
    }
    let decoded =
        |rt: &Rt<'_>, table: &Option<_>| rt.cursors[0].dense().is_none() && table.is_none();
    for (rt, table) in rest.iter().zip(tables) {
        if let (Some(table), true) = (table, rt.cursors[0].dense().is_none()) {
            for &slot in &slots[..kept] {
                table.prefetch(slot as usize);
            }
            let (mut i, mut k) = (0, 0);
            while i < kept {
                let slot = slots[i];
                slots[k] = slot;
                k += usize::from(table.get(slot as usize) != 0);
                i += 1;
            }
            kept = k;
        }
    }
    let mut live = true;
    for (other, _) in rest
        .iter_mut()
        .zip(tables)
        .filter(|(rt, table)| decoded(rt, table))
    {
        if kept == 0 {
            break;
        }
        let (k, more) = keep_common(&mut other.cursors[0], slots, kept, &mut [], Record::Nothing);
        kept = k;
        live &= more;
    }
    (kept, live)
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
    // A dense largest list is not probed: its bitmap below costs no decoding.
    if big_len >= 16 * rest_total && big.cursors[0].dense().is_none() {
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
        if let Some(dense) = rt.cursors[0].dense() {
            // A dense list's bitmap is ORed a word at a time; its postings are not decoded.
            let (words, _) = dense.as_chunks::<8>();
            for (at, word) in bits.iter_mut().zip(words) {
                *at |= u64::from_le_bytes(*word);
            }
            continue;
        }
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

    /// Terms sharing their first eight bytes across many dictionary blocks tie on the in-memory
    /// block keys: every present term is still found with its count, every absent one is not.
    #[test]
    fn block_key_ties_find_every_term() {
        let mut live = DocumentStore::default();
        let words: Vec<String> = (0..300)
            .map(|i| format!("sharedpr{i:03}"))
            .chain(["sharedp", "sharedpr", "sharedprz", "aaaa", "zzzzzzzzzz"].map(String::from))
            .collect();
        for (i, word) in words.iter().enumerate() {
            let repeats: Vec<&str> = std::iter::repeat_n(word.as_str(), 1 + i % 3).collect();
            insert(
                &mut live,
                &format!("d{i:05}"),
                &repeats.join(" "),
                false,
                true,
            );
        }
        let image = image_of(&live);
        let options = LexicalOptions {
            k: 0,
            count: true,
            ..Default::default()
        };
        let absent = [
            "a",
            "sharedo",
            "sharedpr0",
            "sharedpr1000",
            "sharedps",
            "zzzzzzzzzzz",
        ];
        for text in words.iter().map(String::as_str).chain(absent) {
            let query = LexicalQuery::parse(text).unwrap();
            assert!(assert_same(&image, &live, &query, &options), "{text}");
        }
    }

    /// Lists of several runs: the block-max executor's skips are taken, and the result is still
    /// the exhaustive scorer's to the bit.
    #[test]
    fn long_lists_prune_without_changing_the_result() {
        let hash = |x: u32| {
            x.wrapping_mul(2654435761)
                .rotate_left(13)
                .wrapping_mul(40503)
        };
        let docs: Vec<Vec<u8>> = (0..9000u32)
            .map(|i| {
                let len = 3 + hash(i) % 90;
                (0..len)
                    .map(|j| {
                        let h = hash(i * 131 + j);
                        ((h >> 8) % 12).min((h >> 20) % 12) as u8
                    })
                    .collect()
            })
            .collect();
        let live = corpus(&docs, true);
        let image = image_of(&live);
        assert!(image.list(0, "w0").unwrap().blocks() > 2 * RUN_BLOCKS);
        for text in [
            "w0",
            "w3",
            "w0 w1",
            "w1 w2 w3",
            "w0 w5 w9",
            "w2 w7 -w1",
            "+w0 +w1",
            "+w1 +w2 w3",
            "+w0 +w2 +w4",
            "\"w1 w2\"",
            "+\"w0 w1\" w2",
            "w4 \"w1 w0\"",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            for k in [0usize, 1, 10, 100] {
                for count in [false, true] {
                    let options = LexicalOptions {
                        k,
                        count,
                        ..Default::default()
                    };
                    assert!(assert_same(&image, &live, &query, &options), "{text}");
                }
            }
        }
    }

    /// Single terms, conjunctions, a disjunction and a phrase over lists long enough for a rank
    /// table, which orders their runs and bounds their blocks, and over the same lists with
    /// another BM25 shape, which the table does not serve: the exhaustive scorer's result to
    /// the bit.
    #[test]
    fn ranked_lists_keep_the_result() {
        let hash = |x: u32| {
            x.wrapping_mul(2654435761)
                .rotate_left(7)
                .wrapping_mul(40503)
        };
        let docs: Vec<Vec<u8>> = (0..40_000u32)
            .map(|i| {
                let len = 2 + hash(i) % 40;
                (0..len)
                    .map(|j| {
                        let h = hash(i * 97 + j);
                        ((h >> 8) % 8).min((h >> 20) % 8) as u8
                    })
                    .collect()
            })
            .collect();
        let live = corpus(&docs, true);
        let image = image_of(&live).with_rank_tables();
        // The default options are the shape the tables were built for.
        assert!(prepare(&LexicalOptions::default()).unwrap() == Bm25::DEFAULT);
        let list = image.list(0, "w0").unwrap();
        assert!(list.blocks() >= crate::core::documents::lists::RANKED_RUNS * RUN_BLOCKS);
        let average = (image.word_count() as f64 / image.chunk_count() as f64).max(1.0);
        assert!(image
            .rank_table(&Cursor::open(list), Bm25::DEFAULT, average)
            .is_some());
        for text in [
            "w0",
            "w1",
            "w2",
            "w7",
            "+w0 +w1",
            "+w0 +w3 +w7",
            "w0 w2",
            "\"w0 w1\"",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            for (k1, b) in [(1.2, 0.75), (2.0, 0.3)] {
                for k in [1usize, 10, 100, 1000] {
                    for count in [false, true] {
                        let options = LexicalOptions {
                            k,
                            count,
                            bm25_k1: k1,
                            bm25_b: b,
                            ..Default::default()
                        };
                        assert!(assert_same(&image, &live, &query, &options), "{text}");
                    }
                }
            }
        }
    }

    /// Phrases repeating a word (the rarest one, another one, or only one word), and
    /// conjunctions over lists read through their frequency tables, frequencies of fifteen and
    /// more included: the exhaustive scorer's result to the bit.
    #[test]
    fn repeated_phrase_words_and_tabled_lists_keep_the_result() {
        let hash = |x: u32| {
            x.wrapping_mul(2654435761)
                .rotate_left(11)
                .wrapping_mul(40503)
        };
        let docs: Vec<Vec<u8>> = (0..12000u32)
            .map(|i| {
                let len = 4 + hash(i) % 50;
                let mut words: Vec<u8> = (0..len)
                    .map(|j| ((hash(i * 97 + j) >> 9) % 8) as u8)
                    .collect();
                if i % 5 == 0 {
                    words.extend([1, 1, 1, 2, 1, 2, 1]);
                }
                if i % 23 == 0 {
                    words.extend([8, 1, 8, 2, 8, 1, 8]);
                }
                if i % 17 == 0 {
                    words.extend(std::iter::repeat_n(0, 15 + (i % 7) as usize));
                }
                words
            })
            .collect();
        let live = corpus(&docs, true);
        let image = image_of(&live);
        let list = image.list(0, "w0").unwrap();
        assert!(list.blocks() > 2 * RUN_BLOCKS);
        assert!(image.frequency_table(list).is_some(), "w0 has a table");
        for text in [
            "\"w1 w1\"",
            "\"w1 w1 w1\"",
            "\"w1 w2 w1\"",
            "\"w2 w1 w2 w1\"",
            "\"w8 w1 w8\"",
            "\"w1 w8 w1 w8\"",
            "\"w8 w8\"",
            "+w0 +w3",
            "+w0 +w3 +w5",
            "+w8 +w0",
            "+w8 +w1 +w0",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            for k in [0usize, 1, 10, 100] {
                for count in [false, true] {
                    let options = LexicalOptions {
                        k,
                        count,
                        ..Default::default()
                    };
                    assert!(assert_same(&image, &live, &query, &options), "{text}");
                }
            }
        }
    }

    #[test]
    fn frontiers_dominate_every_posting_of_their_block_and_run() {
        let docs: Vec<Vec<u8>> = (0..9000u32)
            .map(|i| {
                let len = 2 + (i.wrapping_mul(2654435761) >> 7) % 60;
                (0..len)
                    .map(|j| ((i / 3 + j * j + j / 2) % 5) as u8)
                    .collect()
            })
            .collect();
        let live = corpus(&docs, false);
        let image = image_of(&live);
        let mut multi_point = 0;
        for term in ["w0", "w1", "w2", "w3", "w4"] {
            let list = image.list(0, term).expect("term");
            let mut cursor = Cursor::open(list);
            assert!(list.blocks() >= 2 * RUN_BLOCKS, "{term}");
            while !cursor.is_end() {
                let (slot, frequency) = (cursor.doc(), cursor.frequency());
                let words = image.slot_word_count(slot as usize);
                let (max_frequency, min_words) = cursor.block_bounds();
                let block: Vec<(u32, u32)> = cursor.block_points().collect();
                multi_point += usize::from(block.len() > 1);
                let run: Vec<(u32, u32)> = cursor.run_points(cursor.run()).collect();
                assert!(!run.is_empty());
                let covered = |points: &[(u32, u32)], fallback: (u32, u32)| {
                    let points = if points.is_empty() {
                        &[fallback][..]
                    } else {
                        points
                    };
                    points.iter().any(|&(f, w)| f >= frequency && w <= words)
                };
                assert!(covered(&block, (max_frequency, min_words)), "{term} {slot}");
                assert!(covered(&run, (max_frequency, min_words)), "{term} {slot}");
                assert!(block.windows(2).all(|p| p[0].0 > p[1].0 && p[0].1 > p[1].1));
                cursor.next();
            }
        }
        assert!(multi_point > 0, "some block has a real frontier");
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
