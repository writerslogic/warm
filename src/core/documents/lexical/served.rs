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

mod lazy;

use self::lazy::keep_common_lazy;
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
    fn new(clause: usize, phrase: bool, lists: &[ListRef<'a>], bound: Option<Bound>) -> Self {
        let mut order: Vec<usize> = (0..lists.len()).collect();
        order.sort_by_key(|&i| lists[i].count);
        let cursors: Vec<_> = order.iter().map(|&i| Cursor::open(lists[i])).collect();
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

/// Opens `clause`'s cursors and bound (and its idf when `k > 0`), or `None` when a word has no
/// list.
#[inline(always)]
fn open_clause<'a>(
    image: &'a ServingImage,
    clause: &Clause,
    index: usize,
    form: usize,
    k: usize,
    n: f64,
    shape: &Shape,
) -> Option<(Rt<'a>, f64)> {
    let lists: Option<Vec<ListRef<'a>>> = clause.words[form]
        .iter()
        .map(|word| image.list(form, word))
        .collect();
    let lists = lists?;
    let mut idf = 0.0;
    let bound = if k > 0 && clause.occur != Occur::MustNot {
        idf = lists
            .iter()
            .map(|list| Bm25::idf(n, f64::from(list.count)))
            .sum();
        Bound::new(idf, shape.average, shape.bm25)
    } else {
        None
    };
    let mut rt = Rt::new(index, clause.phrase, &lists, bound);
    if bound.is_some() {
        for cursor in &mut rt.cursors {
            let table = image.rank_table(cursor, shape.bm25, shape.average);
            cursor.use_ranks(table);
        }
    }
    Some((rt, idf))
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

    let k = options.k;
    let frequencies = vec![0; if k == 0 { 0 } else { query.clauses.len() }];
    let mut idfs = vec![0.0; frequencies.len()];
    let mut required = Vec::new();
    let mut excluded = Vec::new();
    let mut optional = Vec::new();
    for (index, clause) in query.clauses.iter().enumerate() {
        let Some((rt, idf)) = open_clause(image, clause, index, form, k, n, &shape) else {
            match clause.occur {
                Occur::Must => return Ok(Some(empty)),
                _ => continue,
            }
        };
        if k > 0 {
            idfs[index] = idf;
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
    // Optional clauses never change which documents match, so a count needs none of them.
    if k == 0 && !required.is_empty() {
        optional.clear();
    }
    let all_terms = optional.iter().all(|rt| !rt.phrase);
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
        frequencies,
        word_counts: if k > 0 { image.word_counts() } else { &[] },
    };
    let reopen = |index: usize| -> Option<Rt<'_>> {
        open_clause(image, &query.clauses[index], index, form, k, n, &shape).map(|(rt, _)| rt)
    };
    if k > 0
        && excluded.is_empty()
        && !required.is_empty()
        && !optional.is_empty()
        && required
            .iter()
            .chain(&optional)
            .all(|rt| !rt.phrase && rt.bound.is_some())
    {
        let (hits, count) = run.required_optional(required, optional, options.count, &reopen)?;
        return Ok(Some(LexicalResults {
            hits,
            count: options.count.then_some(count),
        }));
    }
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
    if !excluded.is_empty()
        && !required.is_empty()
        && optional.is_empty()
        && (k > 0 || options.count)
        && required
            .iter()
            .all(|rt| !rt.phrase && (k == 0 || rt.bound.is_some()))
        && excluded.iter().all(|rt| !rt.phrase && rt.cursors.len() == 1)
    {
        merge_twins(&mut required);
        let (hits, count) = run.negated_top(&mut required, &mut excluded)?;
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
                .then(|| union_count(image, &mut optional, image.slot_count()));
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
        required: Vec<Rt<'a>>,
        excluded: Vec<Rt<'a>>,
        optional: Vec<Rt<'a>>,
    ) -> Result<(Vec<LexicalHit>, usize)> {
        let mut top = Top::new(self.image, self.k);
        let count = self.conjunction_into(required, excluded, optional, &mut top)?;
        Ok((top.finish(), count))
    }

    /// [`Self::conjunction`] offering to `top`, which may already hold hits; returns the count.
    fn conjunction_into(
        &mut self,
        mut required: Vec<Rt<'a>>,
        mut excluded: Vec<Rt<'a>>,
        mut optional: Vec<Rt<'a>>,
        top: &mut Top<'a>,
    ) -> Result<usize> {
        required.sort_by_key(Rt::rarest);
        let scoring = self.k > 0;
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
        Ok(count)
    }

    /// The top `k` of the documents every term clause has, and with `self.count` their number:
    /// the rarest list a block at a time, filtered through the others (see [`keep_common`]).
    /// A block whose bound, with the bounds of the other lists at it, is below the k-th best
    /// score is skipped before the other lists are read, or when counting, filtered but not
    /// scored. `terms` is sorted rarest first and every clause has a bound.
    fn terms_top(&mut self, terms: &mut [Rt<'a>]) -> Result<(Vec<LexicalHit>, usize)> {
        let mut top = Top::new(self.image, self.k);
        let count = self.terms_top_into(terms, &mut top)?;
        Ok((top.finish(), count))
    }

    /// [`Self::terms_top`] offering to `top`, which may already hold hits; returns the count.
    fn terms_top_into(&mut self, terms: &mut [Rt<'a>], top: &mut Top<'a>) -> Result<usize> {
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
        let mut count = 0usize;
        let mut slots = [0u32; CHECKPOINT_BLOCK];
        let tables = frequency_tables(self.image, &terms[1..]);
        // The other lists in the order candidates are filtered through them: those with a
        // frequency table first, a lookup per candidate, then those whose blocks are decoded.
        let mut sequence: Vec<usize> = (0..lists - 1).collect();
        sequence.sort_by_key(|&l| tables[l].is_none());
        let tabled = tables.iter().flatten().count();
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
            let mut rest_sum = 0.0f64;
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
                        c.span_weight(last, bound, 8, !self.count)
                            .unwrap_or(other.ceiling)
                    };
                    sum += weight * t;
                    rest_sum += weight * t;
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
                // A candidate whose lead frequency, at the block's least word count, cannot
                // lift the block's weights to the k-th best score is dropped before it is
                // probed in the other lists.
                let mut need = 1u32;
                if let (false, Some(floor)) = (self.count, floor) {
                    let most = lead.cursors[0].block_bounds().0.min(NEED_PROBES);
                    let least = lead.cursors[0].block_bounds().1;
                    while need < most
                        && sum_bound(lead_bound.at(need, least) * times[0] + rest_sum, clauses)
                            < floor
                    {
                        need += 1;
                    }
                }
                if need > 1 {
                    #[cfg(test)]
                    PREFILTERED.with(|c| c.set(c.get() + 1));
                    n = keep_frequent(
                        lead.cursors[0].block_frequencies(),
                        &mut slots,
                        n,
                        &mut rows[0],
                        need,
                    );
                } else {
                    for (index, value) in rows[0][..n].iter_mut().enumerate() {
                        *value = index as u32;
                    }
                }
                for (p, &l) in sequence.iter().enumerate() {
                    if n == 0 {
                        break;
                    }
                    let (kept, more) = match &tables[l] {
                        Some(table) => (keep_tabled(table, &mut slots, n, &mut rows, p + 1), true),
                        None => keep_common_lazy(
                            &mut rest[l].cursors[0],
                            &mut slots,
                            n,
                            &mut rows,
                            p + 1,
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
                        if rows[1..1 + tabled].iter().any(|row| row[m] == SATURATED) {
                            // A tabled list holds this candidate fifteen times or more: bound
                            // its weight by the list's largest frequency, and read the list only
                            // when the bound reaches the k-th best score.
                            if let Some(floor) = top.floor() {
                                let words = self.words(slot);
                                let mut sum = lead_bound.at(lead_frequency, words) * times[0];
                                for (p, &l) in sequence.iter().enumerate() {
                                    let other = &rest[l];
                                    let bound = other.bound.expect("a bounded term");
                                    let found = rows[p + 1][m];
                                    let frequency = if p < tabled && found == SATURATED {
                                        other.cursors[0].list_bounds().0
                                    } else {
                                        found
                                    };
                                    sum += bound.at(frequency, words) * times[l + 1];
                                }
                                if sum_bound(sum, clauses) < floor {
                                    continue;
                                }
                            }
                            for (p, &l) in sequence[..tabled].iter().enumerate() {
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
        Ok(count)
    }

    /// The top `k` of the documents every required clause matches, scored with the optional
    /// ones, and with `count` their number. The documents that also match every optional clause
    /// go through the block-wise conjunction of all the terms. The others are split by the
    /// first optional clause they lack, taking the optional clauses strongest first: class `j`
    /// has the first `j` and lacks the next, so a document is in one class only. Each class is
    /// walked with the floor found so far, and the strongest clause's class, the widest, is the
    /// one its weight makes cheapest to prune.
    /// `reopen` opens fresh cursors on a clause by its index.
    #[inline(never)]
    fn required_optional(
        &mut self,
        required: Vec<Rt<'a>>,
        optional: Vec<Rt<'a>>,
        count: bool,
        reopen: &dyn Fn(usize) -> Option<Rt<'a>>,
    ) -> Result<(Vec<LexicalHit>, usize)> {
        // The ranking never counts: the count is that of the required clauses alone.
        self.count = false;
        let mut top = Top::new(self.image, self.k);
        let mut strongest: Vec<(f64, usize)> =
            optional.iter().map(|rt| (rt.ceiling, rt.clause)).collect();
        strongest.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let musts: Vec<usize> = required.iter().map(|rt| rt.clause).collect();
        let order: Vec<usize> = strongest.iter().map(|&(_, clause)| clause).collect();
        let mut all = required;
        all.extend(optional);
        merge_twins(&mut all);
        if all.len() >= 2 {
            #[cfg(test)]
            reached(0);
            self.terms_top_into(&mut all, &mut top)?;
            drop(all);
            // The narrow classes first: they raise the floor before the widest is walked.
            for (j, &lacked) in order.iter().enumerate().rev() {
                // The first pass left other clauses' frequencies in place.
                self.frequencies.fill(0);
                #[cfg(test)]
                if j > 0 {
                    reached(5);
                }
                let mut musts = self.opened(reopen, &musts);
                musts.extend(self.opened(reopen, &order[..j]));
                let excluded = self.opened(reopen, &[lacked]);
                let optional = self.opened(reopen, &order[j + 1..]);
                self.conjunction_into(musts, excluded, optional, &mut top)?;
            }
        } else {
            // One list: the clauses are twins, and a document with one has all.
            drop(all);
            let optional = self.opened(reopen, &order);
            self.conjunction_into(self.opened(reopen, &musts), Vec::new(), optional, &mut top)?;
        }
        let found = if count {
            self.joint(self.opened(reopen, &musts))
        } else {
            0
        };
        Ok((top.finish(), found))
    }

    /// Fresh cursors on the clauses at `clauses`.
    fn opened(&self, reopen: &dyn Fn(usize) -> Option<Rt<'a>>, clauses: &[usize]) -> Vec<Rt<'a>> {
        clauses.iter().filter_map(|&clause| reopen(clause)).collect()
    }

    /// How many documents every one of the term clauses `terms` has.
    fn joint(&self, mut terms: Vec<Rt<'a>>) -> usize {
        merge_twins(&mut terms);
        if terms.len() == 1 {
            terms[0].rarest()
        } else {
            intersect_count(self.image, &mut terms)
        }
    }

    /// [`Self::terms_top`] for a query that also excludes documents: the survivors of the
    /// required lists are dropped when an excluded list holds them, through its bitmap or
    /// frequency table or, failing both, its decoded blocks. Without a `k` only the count is
    /// made, and the clauses have no bounds.
    /// The top `k` of the documents every term clause has, and with `self.count` their number:
    /// the rarest list a block at a time, filtered through the others (see [`keep_common`]).
    /// A block whose bound, with the bounds of the other lists at it, is below the k-th best
    /// score is skipped before the other lists are read, or when counting, filtered but not
    /// scored. `terms` is sorted rarest first and every clause has a bound.
    #[inline(never)]
    fn negated_top(
        &mut self,
        terms: &mut [Rt<'a>],
        excluded: &mut [Rt<'a>],
    ) -> Result<(Vec<LexicalHit>, usize)> {
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
        let tabled = tables.iter().flatten().count();
        // Row 0: each candidate's index in the lead block; row `p`: its frequency in list
        // `sequence[p - 1]`.
        let mut rows = vec![[0u32; CHECKPOINT_BLOCK]; lists];
        let (lead, rest) = terms.split_first_mut().expect("two or more lists");
        let scoring = self.k > 0;
        let lead_bound = lead.bound.unwrap_or_else(|| {
            Bound::new(1.0, self.shape.average, self.shape.bm25).expect("a unit bound")
        });
        // Excluded lists tested cheapest first: bitmap, table, scatter bitmap, decoded blocks.
        let ratio = SCATTER_RATIO;
        let scattered = |rt: &Rt<'_>| self.count && rt.rarest() <= ratio * lead.rarest();
        excluded.sort_by_key(|rt| {
            let kind = if rt.cursors[0].dense().is_some() {
                0
            } else if self.image.frequency_table(rt.cursors[0].list()).is_some() {
                1
            } else if scattered(rt) {
                2
            } else {
                3
            };
            (kind, rt.rarest())
        });
        let excluded_tables = frequency_tables(self.image, excluded);
        let scatters = scatter_excluded(excluded, &excluded_tables, &mut lead.cursors[0], self.count, ratio);
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
            let mut scored = scoring;
            let mut rest_sum = 0.0f64;
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
                        c.span_weight(last, bound, 8, !self.count)
                            .unwrap_or(other.ceiling)
                    };
                    sum += weight * t;
                    rest_sum += weight * t;
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
                    let kept = drop_excluded(excluded, (&excluded_tables, &scatters), &mut slots, kept, &mut []);
                    count += kept;
                    if !more || !lead.cursors[0].advance_block(last.saturating_add(1)) {
                        break;
                    }
                    continue;
                }
                // A candidate whose lead frequency, at the block's least word count, cannot
                // lift the block's weights to the k-th best score is dropped before it is
                // probed in the other lists.
                let mut need = 1u32;
                if let Some(floor) = floor {
                    let most = lead.cursors[0].block_bounds().0.min(NEED_PROBES);
                    let least = lead.cursors[0].block_bounds().1;
                    while need < most
                        && sum_bound(lead_bound.at(need, least) * times[0] + rest_sum, clauses)
                            < floor
                    {
                        need += 1;
                    }
                }
                if need > 1 && !self.count {
                    #[cfg(test)]
                    PREFILTERED.with(|c| c.set(c.get() + 1));
                    n = keep_frequent(
                        lead.cursors[0].block_frequencies(),
                        &mut slots,
                        n,
                        &mut rows[0],
                        need,
                    );
                } else {
                    for (index, value) in rows[0][..n].iter_mut().enumerate() {
                        *value = index as u32;
                    }
                }
                for (p, &l) in sequence.iter().enumerate() {
                    if n == 0 {
                        break;
                    }
                    let (kept, more) = match &tables[l] {
                        Some(table) => (keep_tabled(table, &mut slots, n, &mut rows, p + 1), true),
                        None => keep_common_lazy(
                            &mut rest[l].cursors[0],
                            &mut slots,
                            n,
                            &mut rows,
                            p + 1,
                        ),
                    };
                    n = kept;
                    live &= more;
                    if n == 0 {
                        break;
                    }
                }
                n = drop_excluded(excluded, (&excluded_tables, &scatters), &mut slots, n, &mut rows);
                count += n;
                if n > 0 {
                    // The candidates are known: start loading their word counts together.
                    for &slot in &slots[..n] {
                        prefetch(self.word_counts, slot as usize);
                    }
                    let frequencies = lead.cursors[0].block_frequencies();
                    for (m, &slot) in slots[..n].iter().enumerate() {
                        let lead_frequency = frequencies[rows[0][m] as usize];
                        if lead_frequency < need {
                            // Counted, but too light to enter the top.
                            continue;
                        }
                        if rows[1..1 + tabled].iter().any(|row| row[m] == SATURATED) {
                            // A tabled list holds this candidate fifteen times or more: bound
                            // its weight by the list's largest frequency, and read the list only
                            // when the bound reaches the k-th best score.
                            if let Some(floor) = top.floor() {
                                let words = self.words(slot);
                                let mut sum = lead_bound.at(lead_frequency, words) * times[0];
                                for (p, &l) in sequence.iter().enumerate() {
                                    let other = &rest[l];
                                    let bound = other.bound.expect("a bounded term");
                                    let found = rows[p + 1][m];
                                    let frequency = if p < tabled && found == SATURATED {
                                        other.cursors[0].list_bounds().0
                                    } else {
                                        found
                                    };
                                    sum += bound.at(frequency, words) * times[l + 1];
                                }
                                if sum_bound(sum, clauses) < floor {
                                    continue;
                                }
                            }
                            for (p, &l) in sequence[..tabled].iter().enumerate() {
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
        let tables: Vec<Option<std::sync::Arc<Dense>>> = rest
            .iter()
            .map(|c| self.image.frequency_table(c.list()))
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
            // A word's bitmap or frequency table answers membership without decoding its list.
            for (c, other) in rest.iter().enumerate() {
                if twin[c + 1].is_some() {
                    continue;
                }
                let (mut i, mut k) = (0, 0);
                if let Some(bits) = other.dense() {
                    while i < n {
                        let slot = slots[i];
                        slots[k] = slot;
                        rows[0][k] = rows[0][i];
                        k += usize::from(holds(bits, slot));
                        i += 1;
                    }
                } else if let Some(table) = &tables[c] {
                    for &slot in &slots[..n] {
                        table.prefetch(slot as usize);
                    }
                    while i < n {
                        let slot = slots[i];
                        slots[k] = slot;
                        rows[0][k] = rows[0][i];
                        k += usize::from(table.get(slot as usize) != 0);
                        i += 1;
                    }
                } else {
                    continue;
                }
                n = k;
                if n == 0 {
                    break;
                }
            }
            let mut live = true;
            for (c, other) in rest.iter_mut().enumerate() {
                if twin[c + 1].is_some() || n == 0 {
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
    fn single_top(&self, mut rt: Rt<'a>) -> Result<Vec<LexicalHit>> {
        let bound = rt.bound.expect("a bounded term");
        let idf = bound.idf;
        let cursor = &mut rt.cursors[0];
        let blocks = cursor.block_count();
        let mut top = Top::new(self.image, self.k);
        let below = |top: &Top<'_>, weight: f64| {
            top.floor()
                .is_some_and(|floor| sum_bound(weight, 1) < floor)
        };
        let image = self.image;
        if let Some(table) = image.rank_table(cursor, self.shape.bm25, self.shape.average) {
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
                    self.offer_block(cursor, idf, &mut top)?;
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
                self.offer_block(cursor, idf, &mut top)?;
            }
        }
        Ok(top.finish())
    }

    /// Scores a block for the query's only clause with matching postings.
    fn offer_block(&self, cursor: &mut Cursor<'a>, idf: f64, top: &mut Top<'a>) -> Result<()> {
        let Shape { bm25, average } = *self.shape;
        let (slots, frequencies) = cursor.block_postings();
        for (&slot, &frequency) in slots.iter().zip(frequencies) {
            let norm = bm25.norm(f64::from(self.words(slot)), average);
            let score = 0.0 + bm25.weight(idf, f64::from(frequency), norm);
            ensure!(
                score.is_finite(),
                "lexical score is nonfinite; reduce BM25 parameters"
            );
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
    match words {
        [first, second] => return pair_occurrences(first, second, exact),
        [first, second, third] => return triple_occurrences([first, second, third], exact),
        _ => {}
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

/// A three-word phrase needs only two forward probes after choosing the shortest lead.
fn triple_occurrences(words: [&[u8]; 3], exact: bool) -> u32 {
    let mut order = [0usize, 1, 2];
    order.sort_unstable_by_key(|&word| (words[word].len(), word));
    let [lead, first, second] = order;
    let mut probes = [first, second].map(|word| (word as i64, -1, PositionIter::new(words[word])));
    let mut count = 0;
    'starts: for position in PositionIter::new(words[lead]) {
        let Some(start) = position.checked_sub(lead as u32) else {
            continue;
        };
        for (offset, at, positions) in &mut probes {
            let target = i64::from(start) + *offset;
            while *at < target {
                let Some(position) = positions.next() else {
                    break 'starts;
                };
                *at = i64::from(position);
            }
            if *at != target {
                continue 'starts;
            }
        }
        count += 1;
        if !exact {
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
        Record::Positions(row) => row,
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
        let block = other.block_slots();
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
                Record::Nothing => {}
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
fn intersect_count<'a, T: Lead<'a>>(image: &ServingImage, terms: &mut [T]) -> usize {
    let bitmaps: Vec<&[u8]> = terms.iter().filter_map(|rt| rt.lead().dense()).collect();
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
    let tables: Vec<_> = terms[1..]
        .iter()
        .map(|rt| image.frequency_table(rt.lead().list()))
        .collect();
    let (lead, rest) = terms.split_first_mut().expect("two or more lists");
    let lead = lead.lead_mut();
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
    k
}

/// A clause's leading cursor, or a bare cursor: what the counting kernels read.
trait Lead<'a> {
    fn lead(&self) -> &Cursor<'a>;
    fn lead_mut(&mut self) -> &mut Cursor<'a>;
}

impl<'a> Lead<'a> for Rt<'a> {
    fn lead(&self) -> &Cursor<'a> {
        &self.cursors[0]
    }
    fn lead_mut(&mut self) -> &mut Cursor<'a> {
        &mut self.cursors[0]
    }
}

impl<'a> Lead<'a> for Cursor<'a> {
    fn lead(&self) -> &Cursor<'a> {
        self
    }
    fn lead_mut(&mut self) -> &mut Cursor<'a> {
        self
    }
}

/// Most excluded postings per required posting for which the excluded list is decoded once into
/// a bitmap over the required range rather than probed per candidate.
const SCATTER_RATIO: usize = 12;

/// An excluded list's slots over the required list's range, a bit each.
struct Scatter {
    base: u32,
    bits: Vec<u64>,
}

impl Scatter {
    #[inline(always)]
    fn holds(&self, slot: u32) -> bool {
        let at = (slot - self.base) as usize;
        self.bits[at >> 6] >> (at & 63) & 1 != 0
    }
}

/// For each excluded list that is neither dense nor tabled and, when `counting` (every
/// required block is visited), small enough against the required list `lead`, its slots in the
/// range of `lead` as a bitmap. Leaves `lead` at its first block.
#[inline(never)]
fn scatter_excluded(
    excluded: &mut [Rt<'_>],
    tables: &[Option<std::sync::Arc<Dense>>],
    lead: &mut Cursor<'_>,
    counting: bool,
    ratio: usize,
) -> Vec<Option<Scatter>> {
    let mut out: Vec<Option<Scatter>> = excluded.iter().map(|_| None).collect();
    if !counting {
        return out;
    }
    let first = lead.block_first();
    lead.goto_block(lead.block_count() - 1);
    let last = lead.block_last();
    lead.goto_block(0);
    let required = lead.len();
    for ((rt, table), slot) in excluded.iter_mut().zip(tables).zip(&mut out) {
        let list = &mut rt.cursors[0];
        if list.dense().is_some() || table.is_some() || list.len() > ratio * required {
            continue;
        }
        let mut bits = vec![0u64; ((last - first) as usize >> 6) + 1];
        if list.advance_block(first) {
            loop {
                if list.block_first() > last {
                    break;
                }
                for &doc in list.block_slots() {
                    if (first..=last).contains(&doc) {
                        let at = (doc - first) as usize;
                        bits[at >> 6] |= 1 << (at & 63);
                    }
                }
                if !list.advance_block(list.block_last().saturating_add(1)) {
                    break;
                }
            }
        }
        #[cfg(test)]
        SCATTERED.with(|c| c.set(c.get() + 1));
        *slot = Some(Scatter { base: first, bits });
    }
    out
}

/// Keeps, in place, the candidates among `slots[..n]` that no list of `excluded` holds, moving
/// each kept candidate's entries in `rows` along; `tables` are the excluded lists' frequency
/// tables. A dense list is read from its bitmap, a tabled one from its table, any other
/// through its decoded blocks. Returns how many are kept.
#[inline(never)]
fn drop_excluded(
    excluded: &mut [Rt<'_>],
    (tables, scatters): (&[Option<std::sync::Arc<Dense>>], &[Option<Scatter>]),
    slots: &mut [u32; CHECKPOINT_BLOCK],
    mut n: usize,
    rows: &mut [[u32; CHECKPOINT_BLOCK]],
) -> usize {
    // Lists read without decoding are tested together over the block's candidates and the
    // survivors compacted once; the decoded ones follow, each on what is left.
    let mut keep = [true; CHECKPOINT_BLOCK];
    let mut direct = false;
    for ((rt, table), scatter) in excluded.iter_mut().zip(tables).zip(scatters) {
        let cursor = &rt.cursors[0];
        if let Some(bits) = cursor.dense() {
            for (flag, &slot) in keep.iter_mut().zip(&slots[..n]) {
                *flag &= !holds(bits, slot);
            }
        } else if let Some(scatter) = scatter {
            for (flag, &slot) in keep.iter_mut().zip(&slots[..n]) {
                *flag &= !scatter.holds(slot);
            }
        } else if let Some(table) = table {
            for &slot in &slots[..n] {
                table.prefetch(slot as usize);
            }
            for (flag, &slot) in keep.iter_mut().zip(&slots[..n]) {
                *flag &= table.get(slot as usize) == 0;
            }
        } else {
            continue;
        }
        direct = true;
    }
    if direct {
        n = compact(&keep, slots, n, rows);
    }
    for ((rt, table), scatter) in excluded.iter_mut().zip(tables).zip(scatters) {
        if n == 0 {
            break;
        }
        let cursor = &mut rt.cursors[0];
        if cursor.dense().is_some() || scatter.is_some() || table.is_some() {
            continue;
        }
        let mut keep = [false; CHECKPOINT_BLOCK];
        lack_each(cursor, &slots[..n], &mut keep);
        n = compact(&keep, slots, n, rows);
    }
    n
}

/// Moves the candidates among `slots[..n]` whose `keep` flag is set, and their `rows` entries,
/// to the front. Returns how many.
#[inline(never)]
fn compact(
    keep: &[bool; CHECKPOINT_BLOCK],
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
    rows: &mut [[u32; CHECKPOINT_BLOCK]],
) -> usize {
    let mut kept = 0usize;
    for i in 0..n {
        slots[kept] = slots[i];
        for row in rows.iter_mut() {
            row[kept] = row[i];
        }
        kept += usize::from(keep[i]);
    }
    kept
}

/// Sets `keep[i]` when the ascending `slots[i]` is not in `list`, reading only the blocks the
/// slots fall in.
fn lack_each(list: &mut Cursor<'_>, slots: &[u32], keep: &mut [bool; CHECKPOINT_BLOCK]) {
    for (i, &slot) in slots.iter().enumerate() {
        if list.block_last() < slot && !list.advance_block(slot) {
            // The list ended: it holds none of the rest.
            keep[i..slots.len()].fill(true);
            return;
        }
        keep[i] = list.block_first() > slot || list.block_slots().binary_search(&slot).is_err();
    }
}

/// The frequency nibble that [`keep_tabled`] leaves unread: fifteen or more. Frequencies
/// from decoded lists are exact and do not use this sentinel.
const SATURATED: u32 = 15;

/// Keeps, in place, the candidates among `slots[..n]` whose frequency in the lead block,
/// `frequencies[index]`, is at least `need`, and writes each one's index in the block to `row`.
#[inline(never)]
fn keep_frequent(
    frequencies: &[u32],
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
    row: &mut [u32; CHECKPOINT_BLOCK],
    need: u32,
) -> usize {
    let mut kept = 0usize;
    for (index, &frequency) in frequencies[..n].iter().enumerate() {
        slots[kept] = slots[index];
        row[kept] = index as u32;
        kept += usize::from(frequency >= need);
    }
    kept
}

/// Counts a branch of the required-optional path, for the tests.
#[cfg(test)]
fn reached(path: usize) {
    PATHS.with(|c| {
        let mut all = c.get();
        all[path] += 1;
        c.set(all);
    });
}

#[cfg(test)]
thread_local! {
    static PATHS: std::cell::Cell<[usize; 7]> = const { std::cell::Cell::new([0; 7]) };
    /// Blocks whose candidates `terms_top` filtered by lead frequency, on this thread.
    static PREFILTERED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Excluded lists `negated_top` scattered into a bitmap, on this thread.
    static SCATTERED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The most lead frequencies terms_top tests a block's candidates against.
const NEED_PROBES: u32 = 32;

/// Keeps the candidates among `slots[..n]` that every list of `rest` holds: tested against the
/// dense lists' bitmaps first, then the other lists' frequency tables (`tables`, one per list
/// of `rest`), then filtered through the remaining lists (see [`keep_common`]). The
/// second value is false when a list ended, so no later candidate can match.
fn keep_counted<'a, T: Lead<'a>>(
    rest: &mut [T],
    tables: &[Option<std::sync::Arc<Dense>>],
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
) -> (usize, bool) {
    let mut kept = n;
    for rt in rest.iter() {
        if let Some(bits) = rt.lead().dense() {
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
    let decoded = |rt: &T, table: &Option<_>| rt.lead().dense().is_none() && table.is_none();
    for (rt, table) in rest.iter().zip(tables) {
        if let (Some(table), true) = (table, rt.lead().dense().is_none()) {
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
        let (k, more) = keep_common(other.lead_mut(), slots, kept, &mut [], Record::Nothing);
        kept = k;
        live &= more;
    }
    (kept, live)
}

/// Documents with any of the term clauses' words: the largest list's postings, plus the
/// documents of the others that it lacks, probed through its skip table; or, when the lists
/// are of similar size, a bitmap over all slots.
#[inline(never)]
fn union_count(image: &ServingImage, optional: &mut [Rt<'_>], slots: usize) -> usize {
    if optional.len() == 1 {
        return optional[0].cursors[0].len();
    }
    if let [a, b] = optional {
        // |a ∪ b| = |a| + |b| - |a ∩ b|, the intersection counted from the rarer list by probes
        // into the other: neither list is unioned posting by posting.
        let (mut a, mut b) = (a.cursors[0].list(), b.cursors[0].list());
        if b.count < a.count {
            std::mem::swap(&mut a, &mut b);
        }
        let both = intersect_count(image, &mut [Cursor::open(a), Cursor::open(b)]);
        return a.count as usize + b.count as usize - both;
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
    let words = slots.div_ceil(64);
    // Dense lists are read from their bitmaps and tabled ones from their nibbles, a block of
    // words at a time as the count is made; only the other lists' postings are decoded.
    let mut dense: Vec<&[u8]> = Vec::new();
    let mut tabled: Vec<std::sync::Arc<Dense>> = Vec::new();
    let mut bits: Vec<u64> = Vec::new();
    for rt in optional.iter_mut() {
        if let Some(bitmap) = rt.cursors[0].dense() {
            dense.push(bitmap);
        } else if let Some(table) = image.frequency_table(rt.cursors[0].list()) {
            tabled.push(table);
        } else {
            if bits.is_empty() {
                bits = vec![0u64; words];
            }
            rt.cursors[0].for_each_block(|block| {
                for &doc in block {
                    let doc = doc as usize;
                    bits[doc / 64] |= 1 << (doc % 64);
                }
            });
        }
    }
    union_words(words, &bits, &dense, &tabled)
}

/// The number of bits set in the union of `bits`, the dense lists' bitmaps and the slots the
/// `tabled` lists hold, over `words` words of 64 slots.
#[inline(never)]
fn union_words(
    words: usize,
    bits: &[u64],
    dense: &[&[u8]],
    tabled: &[std::sync::Arc<Dense>],
) -> usize {
    const BLOCK: usize = 64;
    let mut total = 0usize;
    let mut start = 0usize;
    while start < words {
        let n = (words - start).min(BLOCK);
        let mut packed = [0u64; BLOCK];
        if !bits.is_empty() {
            packed[..n].copy_from_slice(&bits[start..start + n]);
        }
        for bitmap in dense {
            let (words, _) = bitmap[(8 * start).min(bitmap.len())..(8 * (start + n)).min(bitmap.len())]
                .as_chunks::<8>();
            for (word, chunk) in packed.iter_mut().zip(words) {
                *word |= u64::from_le_bytes(*chunk);
            }
        }
        if !tabled.is_empty() {
            // Four words of nibbles a word of slots: the lists' nibbles ORed first, since a
            // nibble of the OR is nonzero when any is, then one nibble a bit.
            let mut nibbles = [0u64; 4 * BLOCK];
            for table in tabled {
                let bytes = table.nibble_bytes();
                let from = (32 * start).min(bytes.len());
                let to = (32 * (start + n)).min(bytes.len());
                let (chunks, tail) = bytes[from..to].as_chunks::<8>();
                for (word, chunk) in nibbles.iter_mut().zip(chunks) {
                    *word |= u64::from_le_bytes(*chunk);
                }
                if !tail.is_empty() {
                    let mut last = [0u8; 8];
                    last[..tail.len()].copy_from_slice(tail);
                    nibbles[chunks.len()] |= u64::from_le_bytes(last);
                }
            }
            for (word, quad) in packed[..n].iter_mut().zip(nibbles.as_chunks::<4>().0) {
                *word |= nibble_bits(quad[0])
                    | nibble_bits(quad[1]) << 16
                    | nibble_bits(quad[2]) << 32
                    | nibble_bits(quad[3]) << 48;
            }
        }
        total += packed[..n]
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum::<usize>();
        start += n;
    }
    total
}

/// Bit `j` of the result is set when nibble `j` of `word` is nonzero.
#[inline(always)]
fn nibble_bits(word: u64) -> u64 {
    let mut t = word | word >> 1;
    t |= t >> 2;
    t &= 0x1111_1111_1111_1111;
    #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
    {
        // SAFETY: the target has BMI2, checked at compile time.
        unsafe { std::arch::x86_64::_pext_u64(t, 0x1111_1111_1111_1111) }
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
    {
        t = (t | t >> 3) & 0x0303_0303_0303_0303;
        t = (t | t >> 6) & 0x000F_000F_000F_000F;
        t = (t | t >> 12) & 0x0000_00FF_0000_00FF;
        (t | t >> 24) & 0xFFFF
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::insert;
    use super::*;
    use crate::core::documents::binary::put_varint;
    use crate::core::documents::image::{encode, Backing};
    use proptest::prelude::*;

    #[test]
    fn phrase_merges_keep_occurrences_across_position_gap_widths() {
        for limit in [1u64 << 7, 1 << 14] {
            for start in [limit - 1, limit, limit + 1] {
                let mut first = Vec::new();
                put_varint(&mut first, start);
                put_varint(&mut first, 2);
                let mut second = Vec::new();
                put_varint(&mut second, start + 1);
                put_varint(&mut second, 2);
                assert_eq!(pair_occurrences(&first, &second, true), 2);
                assert_eq!(pair_occurrences(&first, &second, false), 1);
                assert!(at_distance(&second, &first, -1));
                assert!(!at_distance(&first, &second, 2));
            }
        }

        assert_eq!(pair_occurrences(&[0xff, 0x80, 0], &[0x80, 1], true), 1);
    }

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

    #[test]
    fn required_and_optional_clauses_keep_the_exhaustive_result() {
        let docs: Vec<Vec<u8>> = (0..2600u32)
            .map(|i| {
                let i = i % 900;
                (0..4 + i % 9)
                    .map(|j| {
                        let x = i.wrapping_mul(2654435761).wrapping_add(j.wrapping_mul(40503)) >> 7;
                        if x % 3 == 0 {
                            0
                        } else if x % 7 == 0 {
                            1
                        } else {
                            ((x / 11) % 12) as u8
                        }
                    })
                    .collect()
            })
            .collect();
        let live = corpus(&docs, true);
        let image = image_of(&live);
        PATHS.with(|c| c.set([0; 7]));
        for text in [
            "+w0 w1",
            "+w1 w0",
            "+w0 w1 w2",
            "+w2 w0 w1 w3",
            "+w0 +w1 w2",
            "+w2 +w3 w0 w1",
            "+w5 w0 w1 w2 w3 w4 w6",
            "+w0 w0",
            "+w2 w3 w3",
            "+w3 w90",
            "+w3 w90 w1",
            "+w90 w1",
            "+w4 w5 -w1",
            "+w4 w5 +w6 -w1 -w2",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            for k in [0, 1, 10, 100] {
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
        let paths = PATHS.with(|c| c.get());
        for path in [0, 5] {
            assert!(paths[path] > 0, "path {path} never ran: {paths:?}");
        }
    }

    #[test]
    fn required_and_optional_clauses_over_ranked_and_sparse_lists_keep_the_result() {
        let hash = |x: u32| {
            x.wrapping_mul(2654435761)
                .rotate_left(7)
                .wrapping_mul(40503)
        };
        let docs: Vec<Vec<u8>> = (0..40_000u32)
            .map(|i| {
                let len = 2 + hash(i) % 40;
                let mut words: Vec<u8> = (0..len)
                    .map(|j| {
                        let h = hash(i * 97 + j);
                        ((h >> 8) % 8).min((h >> 20) % 8) as u8
                    })
                    .collect();
                if hash(i + 5) % 16 == 0 {
                    words.push(9);
                }
                words
            })
            .collect();
        let live = corpus(&docs, true);
        let image = image_of(&live).with_rank_tables();
        assert!(image.list(0, "w0").unwrap().blocks() >= 256);
        PATHS.with(|c| c.set([0; 7]));
        for text in [
            "+w0 w1 w9",
            "+w0 w9",
            "+w9 w0 w1",
            "+w1 w2 w9 w0",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            for k in [1, 10, 100] {
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
        let paths = PATHS.with(|c| c.get());
        for path in [0, 5] {
            assert!(paths[path] > 0, "path {path} never ran: {paths:?}");
        }
    }

    #[test]
    fn unranked_checkpoint_queries_leave_word_counts_unbuilt() {
        let live = corpus(&[vec![0, 1, 0], vec![0, 2], vec![1, 2]], true);
        let image = image_of(&live);
        let options = LexicalOptions {
            k: 0,
            count: true,
            ..Default::default()
        };
        for text in [
            "w0 w1",
            "+w0 +w1",
            "+w0 -w2",
            "\"w0 w1\"",
            "\"w0 missing\" w1",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            assert!(assert_same(&image, &live, &query, &options), "{text}");
            assert!(!image.has_word_counts(), "{text}");
        }
    }

    /// Terms sharing their first eight bytes across many dictionary blocks tie on the in-memory
    /// block keys: every present term is still found with its count, every absent one is not.
    #[test]
    fn block_key_ties_find_every_term() {
        let mut live = DocumentStore::default();
        let words: Vec<String> = (0..300)
            .map(|i| format!("sharedpr{i:03}"))
            .chain(
                [
                    "sharedp",
                    "sharedpr",
                    "sharedprz",
                    "aaaa",
                    "zzzzzzzzzz",
                    "ö",
                    "ø",
                    "øster",
                ]
                .map(String::from),
            )
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
            "aaaaa",
            "sharedo",
            "sharedpr0",
            "sharedpr1000",
            "sharedps",
            "zzzzzzzzzzz",
            "ó",
            "østers",
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
        let mut span = Cursor::open(list);
        let table = image
            .rank_table(&span, Bm25::DEFAULT, average)
            .expect("rank table");
        span.use_ranks(Some(table));
        let idf = 3.25;
        let bm25 = Bm25::DEFAULT;
        let bound = Bound::new(idf, average, bm25).expect("finite bound");
        let mut probe = Cursor::open(list);
        let mut greatest = 0.0f64;
        for block in 0..8 {
            probe.goto_block(block);
            let last = probe.block_last();
            let (slots, frequencies) = probe.block_postings();
            for (&slot, &frequency) in slots.iter().zip(frequencies) {
                let words = image.slot_word_count(slot as usize);
                let norm = bm25.norm(f64::from(words), average);
                greatest = greatest.max(bm25.weight(idf, f64::from(frequency), norm));
            }
            assert!(span.span_weight(last, bound, 8, true).unwrap() >= greatest);
        }
        probe.goto_block(8);
        assert_eq!(span.span_weight(probe.block_last(), bound, 8, true), None);
        for text in [
            "w0",
            "missing w0 -absent",
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

    /// Conjunctions with a floor and no count over lead lists of many blocks whose frequencies
    /// and lengths vary, so that the floor rises early and candidates whose lead frequency
    /// cannot reach it are dropped before the other lists are probed.
    #[test]
    fn conjunctions_drop_weak_lead_candidates_without_changing_the_result() {
        let hash = |x: u32| {
            x.wrapping_mul(2654435761)
                .rotate_left(15)
                .wrapping_mul(40503)
        };
        let docs: Vec<Vec<u8>> = (0..16000u32)
            .map(|i| {
                let len = 3 + hash(i) % 120;
                let mut words: Vec<u8> = (0..len)
                    .map(|j| match (hash(i * 59 + j) >> 7) % 16 {
                        0..=5 => 0,
                        6..=8 => 1,
                        9 | 10 => 2,
                        11 => 3,
                        x => (x % 5 + 4) as u8,
                    })
                    .collect();
                if i % 41 == 0 {
                    words.extend(std::iter::repeat_n(2, 6 + (i % 5) as usize));
                }
                if i % 97 == 0 {
                    words.extend([1, 2, 1, 2]);
                }
                words
            })
            .collect();
        let live = corpus(&docs, false);
        let image = image_of(&live);
        for word in ["w1", "w2"] {
            assert!(image.list(0, word).unwrap().blocks() > 3, "{word}");
        }
        let before = PREFILTERED.with(|c| c.get());
        for text in [
            "+w2 +w0",
            "+w2 +w1",
            "+w2 +w1 +w0",
            "+w3 +w2 +w0",
            "+w2 +w2 +w0",
            "+w2 +w0 w5",
            "+w2 +w1 -w4",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            for k in [1usize, 3, 10, 50] {
                let options = LexicalOptions {
                    k,
                    count: false,
                    ..Default::default()
                };
                assert!(assert_same(&image, &live, &query, &options), "{text}");
            }
        }
        assert!(
            PREFILTERED.with(|c| c.get()) > before,
            "the lead-frequency filter never ran"
        );
    }

    #[test]
    fn conjunction_reads_saturated_nibbles_and_exact_frequencies() {
        let docs: Vec<Vec<u8>> = (0..1024)
            .map(|i| {
                let frequency = 14 + i % 3;
                let mut words = vec![0; frequency];
                if i < 4 {
                    words.push(1);
                }
                if i < 16 {
                    words.extend(std::iter::repeat_n(2, frequency));
                }
                words
            })
            .collect();
        let live = corpus(&docs, false);
        let image = image_of(&live);
        assert!(image
            .frequency_table(image.list(0, "w0").unwrap())
            .is_some());
        assert!(image
            .frequency_table(image.list(0, "w2").unwrap())
            .is_none());
        let query = LexicalQuery::parse("+w0 +w1 +w2").unwrap();
        for count in [false, true] {
            let options = LexicalOptions {
                k: 10,
                count,
                ..Default::default()
            };
            assert!(assert_same(&image, &live, &query, &options));
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

    #[test]
    fn union_counts_over_many_blocks_match_the_exhaustive_count() {
        // Lists of every shape over 4000 documents: dense, periodic, sparse and clustered, each
        // spanning many blocks, so inclusion-exclusion meets lists that end early.
        let docs: Vec<Vec<u8>> = (0..4000u32)
            .map(|i| {
                let mut words = vec![9u8];
                for (word, holds) in [
                    (0u8, true),
                    (1, i % 2 == 0),
                    (2, i % 3 == 0),
                    (3, i % 7 == 0),
                    (4, i % 50 == 0),
                    (5, i % 5 == 0 && i < 1000),
                    (6, i > 3900),
                ] {
                    if holds {
                        words.push(word);
                    }
                }
                words
            })
            .collect();
        let live = corpus(&docs, false);
        let image = image_of(&live);
        let words = [0u8, 1, 2, 3, 4, 5, 6];
        let mut queries = Vec::new();
        for (i, a) in words.iter().enumerate() {
            for (j, b) in words.iter().enumerate().skip(i + 1) {
                queries.push(format!("w{a} w{b}"));
                for c in &words[j + 1..] {
                    queries.push(format!("w{a} w{b} w{c}"));
                }
            }
        }
        queries.push("w1 w3 w4 w6".into());
        for text in &queries {
            let query = LexicalQuery::parse(text).unwrap();
            let options = LexicalOptions {
                k: 0,
                count: true,
                ..Default::default()
            };
            assert!(assert_same(&image, &live, &query, &options), "{text}");
        }
    }

    #[test]
    fn union_counts_of_dense_tabled_and_decoded_lists_match_over_several_word_blocks() {
        // 9001 slots: more than one block of 64 bitmap words and a partial last word, with
        // dense, tabled and sparse lists in every combination.
        let docs: Vec<Vec<u8>> = (0..9001u32)
            .map(|i| {
                let mut words = vec![9u8];
                for (word, holds) in [
                    (0u8, i % 3 != 0),
                    (1, i % 2 == 0),
                    (2, i % 9 == 0),
                    (3, i % 31 == 0 || i == 9000),
                    (4, i % 97 == 0),
                    (5, i > 8990),
                ] {
                    if holds {
                        words.push(word);
                    }
                }
                words
            })
            .collect();
        let live = corpus(&docs, false);
        let image = image_of(&live);
        let tabled = |word: &str| image.frequency_table(image.list(0, word).unwrap()).is_some();
        assert!(tabled("w2") && tabled("w3") && !tabled("w5"));
        for text in [
            "w0 w1 w2",
            "w2 w3 w4",
            "w2 w3 w5",
            "w1 w2 w3 w4 w5",
            "w3 w4 w5",
            "w0 w2 w3",
            "w2 w3 w4 w5",
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            let options = LexicalOptions {
                k: 0,
                count: true,
                ..Default::default()
            };
            assert!(assert_same(&image, &live, &query, &options), "{text}");
        }
    }

    /// Required terms with excluded ones, whose lists are dense, tabled or only decoded, over
    /// lists of many blocks and varying frequencies: the exhaustive scorer's result to the bit,
    /// with and without counts, for k of 0, 1, 10 and 100.
    #[test]
    fn excluded_terms_keep_the_result() {
        let hash = |x: u32| {
            x.wrapping_mul(2654435761)
                .rotate_left(9)
                .wrapping_mul(40503)
        };
        let docs: Vec<Vec<u8>> = (0..9001u32)
            .map(|i| {
                let len = 3 + hash(i) % 60;
                let mut words: Vec<u8> = (0..len)
                    .map(|j| match (hash(i * 71 + j) >> 6) % 20 {
                        0..=7 => 0,
                        8..=10 => 1,
                        11 | 12 => 2,
                        13 => 3,
                        14 if i % 5 == 0 => 4,
                        _ => 9,
                    })
                    .collect();
                if i % 31 == 0 || i == 9000 {
                    words.extend([5, 5, 3]);
                }
                if i % 211 == 0 {
                    words.push(6);
                }
                words
            })
            .collect();
        let live = corpus(&docs, false);
        let image = image_of(&live);
        let tabled = |word: &str| image.frequency_table(image.list(0, word).unwrap()).is_some();
        assert!(tabled("w1") && tabled("w2") && tabled("w5") && !tabled("w6"));
        assert!(image.list(0, "w0").unwrap().blocks() > 3);
        for text in [
            "+w1 -w0",
            "+w2 -w1",
            "+w5 -w6",
            "+w6 -w5",
            "+w2 -w0 -w3",
            "+w3 -w4 -w6",
            "+w1 +w2 -w0",
            "+w1 +w2 -w5 -w6",
            "+w5 +w3 -w1",
            "+w6 -w0 -w1 -w2",
            "+w1 -w1",
            "+w2 +w2 -w6",
            "+w1 -w7",
            "+w4 -w3 -w3",
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

    /// Excluded lists that are only decoded and within SCATTER_RATIO times the required list
    /// are scattered into a bitmap when counting: the exhaustive scorer's result to the bit
    /// either side of the ratio, for k of 0, 10 and 100, and the scatter ran exactly then.
    #[test]
    fn scattered_excluded_lists_keep_the_result() {
        let hash = |x: u32| x.wrapping_mul(2654435761).rotate_left(9).wrapping_mul(40503);
        let docs: Vec<Vec<u8>> = (0..9001u32)
            .map(|i| {
                let len = 3 + hash(i) % 40;
                let mut words: Vec<u8> = (0..len)
                    .map(|j| match (hash(i * 71 + j) >> 6) % 12 {
                        0..=3 => 0,
                        4 | 5 => 1,
                        _ => 9,
                    })
                    .collect();
                // w6: 23 postings. w12: 276 = 12 x 23, w13: 277, w14: 275, all decoded.
                for (word, holds) in [
                    (6u8, i % 400 == 0),
                    (12, i % 33 == 0 || (1..=3).contains(&i)),
                    (13, i % 33 == 0 || (1..=4).contains(&i)),
                    (14, i % 33 == 0 || (1..=2).contains(&i)),
                ] {
                    if holds {
                        words.push(word);
                        words.push(word);
                    }
                }
                words
            })
            .collect();
        let live = corpus(&docs, false);
        let image = image_of(&live);
        let postings = |word: &str| image.list(0, word).unwrap().count as usize;
        assert_eq!(postings("w6") * SCATTER_RATIO, postings("w12"));
        assert_eq!(postings("w13"), postings("w12") + 1);
        for word in ["w12", "w13", "w14"] {
            let list = image.list(0, word).unwrap();
            assert!(image.frequency_table(list).is_none(), "{word}");
        }
        // (query, scatters expected when counting)
        for (text, scatters) in [
            ("+w6 -w12", 1),
            ("+w6 -w14", 1),
            ("+w6 -w13", 0),
            ("+w6 -w14 -w12", 2),
            ("+w6 -w13 -w14", 1),
            ("+w6 +w1 -w12", 1),
            ("+w1 -w12 -w13 -w14", 3),
        ] {
            let query = LexicalQuery::parse(text).unwrap();
            for k in [0usize, 10, 100] {
                for count in [false, true] {
                    let options = LexicalOptions {
                        k,
                        count,
                        ..Default::default()
                    };
                    let before = SCATTERED.with(|c| c.get());
                    assert!(assert_same(&image, &live, &query, &options), "{text}");
                    let ran = SCATTERED.with(|c| c.get()) - before;
                    assert_eq!(ran > 0, count && scatters > 0, "{text} k={k} count={count}");
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        #[test]
        fn triplet_occurrences_match_anchored_positions(
            masks in any::<[u16; 3]>(),
            base in prop::sample::select(vec![0u32, 126, 16_382, u32::MAX - 15]),
        ) {
            let words = masks.map(|mask| {
                let mut bytes = Vec::new();
                let mut previous = 0;
                for offset in 0..16 {
                    if mask & (1 << offset) != 0 {
                        let position = base + offset;
                        crate::core::documents::binary::put_varint(
                            &mut bytes, u64::from(position - previous),
                        );
                        previous = position;
                    }
                }
                bytes
            });
            let words = words.each_ref().map(Vec::as_slice);
            let [first, second, third] = masks;
            let expected = (first & (second >> 1) & (third >> 2)).count_ones();
            prop_assert_eq!(triple_occurrences(words, true), expected);
            prop_assert_eq!(triple_occurrences(words, false), u32::from(expected != 0));
        }

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
