// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// The store state and BM25 constants a thread's cached length normalizations belong to.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct NormKey {
    pub(super) store: super::super::index::StoreIdentity,
    pub(super) generation: u64,
    pub(super) k1: u64,
    pub(super) one_minus_b: u64,
    pub(super) b: u64,
}

/// Per-thread working memory of [`capture_ranked_lexical`], reused across queries so a query
/// neither allocates nor clears a table per store slot. It grows to the largest store a thread
/// queries (20 bytes per slot plus 4 per posting of the longest query: no more than the
/// store's own owner and word-count tables per slot) and is not shrunk. It holds no result of
/// an earlier query: every table is rewritten before a query reads it.
#[derive(Default)]
pub(super) struct LexicalScratch {
    /// BM25 sum per slot; every entry is 0 between queries.
    pub(super) scores: Vec<f64>,
    /// Length normalization per slot under `key`, valid where `norm_epochs` holds `epoch`.
    pub(super) norms: Vec<f64>,
    pub(super) norm_epochs: Vec<u32>,
    pub(super) epoch: u32,
    pub(super) key: Option<NormKey>,
    /// Whether every slot's norm is valid under `key`.
    pub(super) complete: bool,
    /// Slots a sparse query reached, with repeats; read back once each by the selection.
    pub(super) touched: Vec<u32>,
    /// Selection candidates and the bounded heap of the best scores (as `f64` bits) seen.
    pub(super) candidates: Vec<(f64, u32)>,
    pub(super) best: std::collections::BinaryHeap<std::cmp::Reverse<u64>>,
    /// Set while a query runs; a query that finds it set (an earlier one panicked midway)
    /// zeroes every score first.
    pub(super) dirty: bool,
    pub(super) pruned: super::super::impact::PruneScratch,
}

/// One term's postings in a query.
pub(super) struct Listed<L> {
    idf: f64,
    list: L,
}

thread_local! {
    pub(super) static LEXICAL_SCRATCH: RefCell<LexicalScratch> = RefCell::new(LexicalScratch::default());
}

impl LexicalScratch {
    /// The query's top `limit` chunks with a positive BM25 score as `(score, slot)`, ordered by
    /// score descending, then document id and chunk index ascending. Scores are the sums
    /// `execute_chunks` computes from captured postings: the same terms in the same order
    /// (form 0 terms, then form 1 terms, each sorted), the same expressions, from 0.
    ///
    /// With `hot` set, `store` is an immutable checkpoint, or a live store whose generation
    /// every mutation advances, and its long posting lists may be kept decoded and weighted by
    /// this thread between queries (see [`HotList`]).
    pub(super) fn rank<S: LexicalSource>(
        &mut self,
        store: &S,
        prepared: &PreparedSearch,
        hot: bool,
    ) -> Vec<(f64, u32)> {
        self.rank_with(store, prepared, hot, &[], &mut Vec::new())
    }

    /// [`Self::rank`], also pushing to `extras` the lexical score of each slot of `extra` (zero for a slot no query
    /// term reaches), in order, as accumulated before the selection clears them.
    pub(super) fn rank_with<S: LexicalSource>(
        &mut self,
        store: &S,
        prepared: &PreparedSearch,
        hot: bool,
        extra: &[usize],
        extras: &mut Vec<f64>,
    ) -> Vec<(f64, u32)> {
        if hot
            && extra.is_empty()
            && store
                .impact_image()
                .is_some_and(|image| prune_wanted(image.slot_total()))
        {
            let limit = prepared.limit;
            if super::super::impact::rank(
                store,
                &prepared.query_terms,
                prepared.bm25,
                limit,
                &mut self.pruned,
            )
            .is_some()
            {
                let candidates = &mut self.pruned.candidates;
                top_by_order(store, candidates, limit);
                return candidates.clone();
            }
        }
        let slots = store.slot_count();
        if self.dirty {
            self.scores.fill(0.0);
        }
        if self.scores.len() < slots {
            self.scores.resize(slots, 0.0);
            self.norms.resize(slots, 0.0);
            self.norm_epochs.resize(slots, 0);
            self.complete = false;
        }
        self.dirty = true;
        let n = store.chunk_count() as f64;
        let average = (store.word_count() as f64 / n).max(1.0);
        let bm25 = prepared.bm25;
        let (identity, generation) = store.state();
        let key = NormKey {
            store: identity,
            generation,
            k1: bm25.k1.to_bits(),
            one_minus_b: bm25.one_minus_b.to_bits(),
            b: bm25.b.to_bits(),
        };
        // Nothing computed for an earlier query is read: the norms are valid for this query
        // alone.
        self.key = Some(key);
        self.complete = false;
        self.epoch = match self.epoch.checked_add(1) {
            Some(epoch) => epoch,
            None => {
                self.norm_epochs.fill(0);
                1
            }
        };
        let mut lists = Vec::new();
        let mut postings = 0usize;
        for (form, terms) in prepared.query_terms.iter().enumerate() {
            for word in terms {
                if let Some(list) = store.postings(form, word) {
                    let df = list.len() as f64;
                    postings = postings.saturating_add(list.len());
                    lists.push(Listed {
                        idf: Bm25::idf(n, df),
                        list,
                    });
                }
            }
        }
        // A query whose postings cover a good part of the store reads every slot's norm and
        // score; a sparser one keeps its norms lazy and its selection to the slots it reached.
        let dense = postings >= slots / 2;
        if !self.complete && dense {
            for slot in 0..slots {
                if self.norm_epochs[slot] != self.epoch {
                    self.norm_epochs[slot] = self.epoch;
                    self.norms[slot] = bm25.norm(f64::from(store.slot_word_count(slot)), average);
                }
            }
            self.complete = true;
        }
        let scores = &mut self.scores[..slots];
        self.touched.clear();
        if dense {
            let norms = &self.norms[..slots];
            for listed in &lists {
                accumulate(&listed.list, bm25, listed.idf, norms, scores);
            }
        } else {
            self.touched.resize(postings, 0);
            let (norms, norm_epochs, touched) =
                (&mut self.norms, &mut self.norm_epochs, &mut self.touched);
            let (epoch, complete) = (self.epoch, self.complete);
            let simple = bm25.k1 <= 1.0 || bm25 == Bm25::DEFAULT;
            let mut reached = 0;
            for listed in &lists {
                let (idf, list) = (listed.idf, &listed.list);
                list.for_each(|slot, frequency| {
                    if !complete && norm_epochs[slot] != epoch {
                        norm_epochs[slot] = epoch;
                        norms[slot] = bm25.norm(f64::from(store.slot_word_count(slot)), average);
                    }
                    // Weights are never -0, so a slot reads +0 until its first nonzero
                    // weight; a slot recorded twice reads 0 the second time in the
                    // selection.
                    let score = scores[slot];
                    touched[reached] = slot as u32;
                    reached += usize::from(score == 0.0);
                    let weight = weigh_one(simple, bm25, idf, f64::from(frequency), norms[slot]);
                    scores[slot] = score + weight;
                });
            }
            self.touched.truncate(reached);
        }
        extras.extend(extra.iter().map(|&slot| scores[slot]));
        // Selection: a bounded heap holds the best `limit` scores read so far (as `f64` bits,
        // which order positive values); once it is full its smallest is the floor a later
        // score must reach. Every score at or above the floor when read is kept, so all chunks
        // scoring at least the final floor (the `limit`-th best score) are among the kept
        // ones, ties included, and the exact order decides between them.
        let limit = prepared.limit;
        let (candidates, best) = (&mut self.candidates, &mut self.best);
        candidates.clear();
        best.clear();
        // The least admissible score as bits: any positive one until the heap is full, then the
        // floor. One unsigned range test admits positive scores (infinity included) at or
        // above it and turns away zero, NaN and the scores below it without branching on each.
        const INFINITY: u64 = 0x7FF0_0000_0000_0000;
        let floor_cell = std::cell::Cell::new(1u64);
        let mut consider = |slot: u32, score: f64| {
            let floor = floor_cell.get();
            let bits = score.to_bits();
            if bits.wrapping_sub(floor) > INFINITY - floor {
                return;
            }
            candidates.push((score, slot));
            if best.len() < limit {
                best.push(std::cmp::Reverse(bits));
            } else if bits > floor {
                *best.peek_mut().expect("full heap") = std::cmp::Reverse(bits);
            } else {
                return;
            }
            if best.len() == limit {
                floor_cell.set(best.peek().expect("full heap").0);
            }
        };
        if dense {
            const SCAN: usize = 8;
            for (index, chunk) in scores.chunks_mut(SCAN).enumerate() {
                let floor = floor_cell.get();
                let admitted = chunk.iter().fold(false, |any, score| {
                    any | (score.to_bits().wrapping_sub(floor) <= INFINITY - floor)
                });
                if admitted {
                    for (offset, score) in chunk.iter().enumerate() {
                        consider((index * SCAN + offset) as u32, *score);
                    }
                }
                chunk.fill(0.0);
            }
        } else {
            for &slot in &self.touched {
                consider(slot, std::mem::replace(&mut scores[slot as usize], 0.0));
            }
        }
        let bar = if best.len() < limit {
            0
        } else {
            best.peek().expect("full heap").0
        };
        candidates.retain(|&(score, _)| score.to_bits() >= bar);
        top_by_order(store, candidates, limit);
        self.dirty = false;
        candidates.clone()
    }
}

/// `candidates` cut to the best `limit` and ordered by score descending, then by the source's
/// tie order. A source that gives the order as an integer is read once per candidate rather
/// than twice per comparison.
fn top_by_order<S: LexicalSource>(store: &S, candidates: &mut Vec<(f64, u32)>, limit: usize) {
    let keyed: Option<Vec<(f64, u64, u32)>> = candidates
        .iter()
        .map(|&(score, slot)| {
            store
                .slot_tie_key(slot as usize)
                .map(|key| (score, key, slot))
        })
        .collect();
    let Some(mut keyed) = keyed else {
        top_ranked(candidates, limit, |a, b| {
            b.0.total_cmp(&a.0).then_with(|| {
                store
                    .slot_order(a.1 as usize)
                    .cmp(&store.slot_order(b.1 as usize))
            })
        });
        return;
    };
    top_ranked(&mut keyed, limit, |a, b| {
        b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1))
    });
    candidates.clear();
    candidates.extend(keyed.iter().map(|&(score, _, slot)| (score, slot)));
}

pub(super) const WEIGH_BLOCK: usize = 128;

/// One posting's weight: `Bm25::weight`, with its parameter test hoisted into `simple`.
#[inline(always)]
pub(super) fn weigh_one(simple: bool, bm25: Bm25, idf: f64, tf: f64, norm: f64) -> f64 {
    if !simple {
        bm25.weight(idf, tf, norm)
    } else if tf == 0.0 {
        0.0
    } else {
        idf * tf * bm25.k1_plus_one / (tf + bm25.k1 * norm)
    }
}

/// Adds each posting's BM25 weight to its slot's score, in posting order, and appends the slots
/// and weights to `keep` when given. Postings are weighed a block at a time; every weight is
/// the `Bm25::weight` expression evaluated on the same operands, and each slot appears once per
/// list, so the scores are those of a posting-by-posting loop.
pub(super) fn accumulate(
    list: &impl PostingSet,
    bm25: Bm25,
    idf: f64,
    norms: &[f64],
    scores: &mut [f64],
) {
    assert!(norms.len() == scores.len() && norms.len() <= i32::MAX as usize);
    let simple = bm25.k1 <= 1.0 || bm25 == Bm25::DEFAULT;
    if list.len() < WEIGH_BLOCK / 4 {
        list.for_each(|slot, frequency| {
            let weight = weigh_one(simple, bm25, idf, f64::from(frequency), norms[slot]);
            scores[slot] += weight;
        });
        return;
    }
    let mut slots = [0u32; WEIGH_BLOCK];
    let mut tfs = [0f64; WEIGH_BLOCK];
    let mut weights = [0f64; WEIGH_BLOCK];
    let mut filled = 0usize;
    let mut flush = |slots: &[u32], tfs: &[f64], weights: &mut [f64]| {
        let count = slots.len();
        let highest = slots.iter().copied().max().unwrap_or(0) as usize;
        assert!(
            count == 0 || highest < norms.len(),
            "posting slot past the store"
        );
        if simple {
            // SAFETY: every slot is below `norms.len()`, checked above.
            unsafe { weigh_simple(bm25, idf, norms, slots, tfs, weights) };
        } else {
            for index in 0..count {
                weights[index] = bm25.weight(idf, tfs[index], norms[slots[index] as usize]);
            }
        }
        for (&slot, &weight) in slots.iter().zip(&weights[..count]) {
            scores[slot as usize] += weight;
        }
    };
    list.for_each(|slot, frequency| {
        slots[filled] = slot as u32;
        tfs[filled] = f64::from(frequency);
        filled += 1;
        if filled == WEIGH_BLOCK {
            flush(&slots, &tfs, &mut weights);
            filled = 0;
        }
    });
    if filled > 0 {
        flush(&slots[..filled], &tfs[..filled], &mut weights);
    }
}

/// `weights[i] = bm25.weight(idf, tfs[i], norms[slots[i]])` for the default saturation form
/// `idf * tf * (k1 + 1) / (tf + k1 * norm)`, with 0 where `tf` is 0.
///
/// # Safety
/// Every slot must index `norms`.
#[inline]
unsafe fn weigh_simple(
    bm25: Bm25,
    idf: f64,
    norms: &[f64],
    slots: &[u32],
    tfs: &[f64],
    weights: &mut [f64],
) {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        // SAFETY: the feature is present; the slot invariant is the caller's.
        return unsafe { weigh_simple_avx2(bm25, idf, norms, slots, tfs, weights) };
    }
    // SAFETY: the slot invariant is the caller's.
    unsafe { weigh_simple_generic(bm25, idf, norms, slots, tfs, weights) }
}

#[inline(always)]
unsafe fn weigh_simple_generic(
    bm25: Bm25,
    idf: f64,
    norms: &[f64],
    slots: &[u32],
    tfs: &[f64],
    weights: &mut [f64],
) {
    for index in 0..slots.len() {
        let tf = tfs[index];
        // SAFETY: the caller guarantees the slot indexes `norms`.
        let norm = unsafe { *norms.get_unchecked(slots[index] as usize) };
        let weight = idf * tf * bm25.k1_plus_one / (tf + bm25.k1 * norm);
        weights[index] = f64::from_bits(weight.to_bits() & u64::from(tf != 0.0).wrapping_neg());
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn weigh_simple_avx2(
    bm25: Bm25,
    idf: f64,
    norms: &[f64],
    slots: &[u32],
    tfs: &[f64],
    weights: &mut [f64],
) {
    use std::arch::x86_64::*;
    let count = slots.len();
    let (idf_v, k1p1_v, k1_v) = (
        _mm256_set1_pd(idf),
        _mm256_set1_pd(bm25.k1_plus_one),
        _mm256_set1_pd(bm25.k1),
    );
    let zero = _mm256_setzero_pd();
    let mut index = 0;
    while index + 4 <= count {
        // SAFETY: four slots, tfs and weights are in range; the slots index `norms`.
        unsafe {
            let at = _mm_loadu_si128(slots.as_ptr().add(index).cast());
            let norm = _mm256_i32gather_pd::<8>(norms.as_ptr(), at);
            let tf = _mm256_loadu_pd(tfs.as_ptr().add(index));
            let numerator = _mm256_mul_pd(_mm256_mul_pd(idf_v, tf), k1p1_v);
            let denominator = _mm256_add_pd(tf, _mm256_mul_pd(k1_v, norm));
            let weight = _mm256_div_pd(numerator, denominator);
            let weight = _mm256_and_pd(weight, _mm256_cmp_pd::<_CMP_NEQ_OQ>(tf, zero));
            _mm256_storeu_pd(weights.as_mut_ptr().add(index), weight);
        }
        index += 4;
    }
    // SAFETY: the slot invariant is the caller's.
    unsafe {
        weigh_simple_generic(
            bm25,
            idf,
            norms,
            &slots[index..],
            &tfs[index..],
            &mut weights[index..],
        )
    };
}
