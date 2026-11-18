// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(crate) fn capture_search(
    store: &DocumentStore,
    prepared: Vec<PreparedSearch>,
) -> CapturedSearch {
    CapturedSearch {
        queries: prepared
            .into_iter()
            .map(|prepared| capture_query(store, prepared))
            .collect(),
    }
}

pub(super) fn capture_query(store: &DocumentStore, prepared: PreparedSearch) -> CapturedQuery {
    if prepared.semantic_weight > 0.0 {
        if let Some(index) = store.dense_index.as_ref().filter(|index| {
            prepared
                .embedding
                .as_ref()
                .is_some_and(|query| query.len() == index.dimensions)
        }) {
            return capture_planned_query(store, prepared, Some(Arc::clone(index)));
        }
    }
    if prepared.lexical_only {
        return capture_ranked_lexical(store, prepared);
    }
    if prepared.embedding.is_none() || prepared.semantic_weight == 0.0 {
        return capture_planned_query(store, prepared, None);
    }
    if prepared.unfiltered {
        let table = prepared
            .embedding
            .as_ref()
            .and_then(|query| store.screen_table(query.len()));
        if let Some(table) = table {
            return match capture_screened(store, prepared, &table) {
                Ok(captured) => captured,
                Err(prepared) => capture_postings(store, *prepared),
            };
        }
    }
    capture_postings(store, prepared)
}

/// Captures an unfiltered hybrid query without a slot per chunk: the lexical list's chunks and every chunk the
/// quantized screen leaves able to rank in the semantic list, with the exact BM25 score of each. Every chunk outside
/// that set is below both lists' cut, so [`execute_chunks`] ranks the same hits as over the whole store. Gives the
/// query back when the screen cannot bound it.
pub(super) fn capture_screened(
    store: &DocumentStore,
    prepared: PreparedSearch,
    table: &ScreenTable,
) -> Result<CapturedQuery, Box<PreparedSearch>> {
    let query = prepared
        .embedding
        .as_deref()
        .expect("a screened query has an embedding");
    let Some(candidates) = table.candidates(query, prepared.limit, prepared.minimum) else {
        return Err(Box::new(prepared));
    };
    let (ranked, extras) = if prepared.lexical_weight > 0.0 {
        let mut extras = Vec::with_capacity(candidates.len());
        let ranked = LEXICAL_SCRATCH.with(|scratch| {
            scratch
                .borrow_mut()
                .rank_with(store, &prepared, true, &candidates, &mut extras)
        });
        (ranked, extras)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut scored: FastMap<usize, f64> = candidates.iter().copied().zip(extras).collect();
    scored.extend(ranked.iter().map(|&(score, slot)| (slot as usize, score)));
    let mut slots: Vec<usize> = ranked
        .iter()
        .map(|&(_, slot)| slot as usize)
        .chain(candidates)
        .collect();
    slots.sort_unstable();
    slots.dedup();
    let pending = (prepared.lexical_weight <= 0.0).then(|| {
        let n = store.chunk_count as f64;
        let idf: [BTreeMap<String, f64>; 2] = std::array::from_fn(|form| {
            prepared.query_terms[form]
                .iter()
                .map(|word| {
                    let df = store.postings[form].get(word).map_or(0, PostingList::len) as f64;
                    (word.clone(), Bm25::idf(n, df))
                })
                .collect()
        });
        PendingLexical {
            idf,
            average: (store.word_count as f64 / n).max(1.0),
        }
    });
    let mut chunks = Vec::with_capacity(slots.len());
    let mut scores = Vec::with_capacity(slots.len());
    for slot in slots {
        let (document, index) = store.owned_chunk(slot);
        scores.push(scored.get(&slot).copied().unwrap_or(f64::NAN));
        chunks.push(CapturedChunk::live(
            document,
            index,
            store.slot_word_count(slot),
        ));
    }
    Ok(CapturedQuery {
        prepared,
        chunks,
        postings: Default::default(),
        count: store.chunk_count,
        words: store.word_count,
        use_postings: true,
        dense: None,
        scan: None,
        eligible: None,
        excluded_owners: FastSet::default(),
        dfs: None,
        text: store.text_segment(),
        lexical_scores: Some(scores),
        screened: Some(Box::new(ScreenedLexical { pending })),
    })
}

/// Captures the chunks a query may rank and, where postings are cheaper than a scan of the
/// candidates, their postings, for [`execute_chunks`] to score.
pub(super) fn capture_postings(store: &DocumentStore, prepared: PreparedSearch) -> CapturedQuery {
    let mut chunks = Vec::new();
    let mut positions = SlotPositions::new(store.slot_count);
    let (count, words, use_postings) = if prepared.lexical_only {
        (store.chunk_count, store.word_count, true)
    } else {
        let candidates = store.candidate_ids(&prepared.filters);
        let documents: Vec<_> = match candidates {
            Some(ids) => ids.into_iter().map(|id| &store.documents[id]).collect(),
            None => store.documents.values().collect(),
        };
        let mut words = 0;
        for document in documents
            .into_iter()
            .filter(|document| prepared.matches(document))
        {
            for (index, (&slot, chunk)) in store.slots[&document.id]
                .iter()
                .zip(&document.chunks)
                .enumerate()
            {
                positions.insert(slot, chunks.len() as u32);
                words += chunk.word_count;
                chunks.push(CapturedChunk::live(
                    Arc::clone(document),
                    index,
                    super::super::index::slot_words(chunk.word_count),
                ));
            }
        }
        let posting_work: usize = prepared
            .query_terms
            .iter()
            .enumerate()
            .map(|(form, terms)| {
                terms
                    .iter()
                    .map(|word| store.postings[form].get(word).map_or(0, PostingList::len))
                    .sum::<usize>()
            })
            .sum();
        let scan_work = chunks
            .len()
            .saturating_mul(prepared.query_terms.iter().map(BTreeSet::len).sum());
        (
            chunks.len(),
            words,
            posting_work.saturating_add(store.slot_count) < scan_work,
        )
    };
    let mut postings: [BTreeMap<String, Vec<CapturedPosting>>; 2] = Default::default();
    if use_postings && (prepared.lexical_weight > 0.0 || !prepared.lexical_only) {
        for (form, terms) in prepared.query_terms.iter().enumerate() {
            for word in terms {
                let Some(source) = store.postings[form].get(word) else {
                    continue;
                };
                let mut captured = Vec::with_capacity(source.len().min(count));
                source.for_each(|slot, frequency| {
                    let index = match positions.get(slot) {
                        Some(index) => index,
                        None if prepared.lexical_only => {
                            let (document, index) = store.owned_chunk(slot);
                            let position = chunks.len() as u32;
                            chunks.push(CapturedChunk::live(
                                document,
                                index,
                                store.slot_word_count(slot),
                            ));
                            positions.insert(slot, position);
                            position
                        }
                        None => return,
                    };
                    captured.push(CapturedPosting {
                        chunk: index,
                        frequency,
                    });
                });
                postings[form].insert(word.clone(), captured);
            }
        }
    }
    CapturedQuery {
        prepared,
        chunks,
        postings,
        count,
        words,
        use_postings,
        dense: None,
        scan: None,
        eligible: None,
        excluded_owners: FastSet::default(),
        dfs: None,
        text: store.text_segment(),
        lexical_scores: None,
        screened: None,
    }
}

/// Captures an unfiltered lexical query already ranked: the postings of its terms are scored
/// into this thread's slot table exactly as [`execute_chunks`] scores captured postings, and
/// only the top `limit` chunks are captured, so the capture holds `limit` documents rather than
/// one per matching chunk.
pub(super) fn capture_ranked_lexical(
    store: &DocumentStore,
    prepared: PreparedSearch,
) -> CapturedQuery {
    let mut chunks = Vec::new();
    let mut scores = Vec::new();
    if prepared.lexical_weight > 0.0 {
        let ranked =
            LEXICAL_SCRATCH.with(|scratch| scratch.borrow_mut().rank(store, &prepared, false));
        chunks.reserve_exact(ranked.len());
        scores.reserve_exact(ranked.len());
        for (score, slot) in ranked {
            let slot = slot as usize;
            let (document, index) = store.owned_chunk(slot);
            chunks.push(CapturedChunk::live(
                document,
                index,
                store.slot_word_count(slot),
            ));
            scores.push(score);
        }
    }
    CapturedQuery {
        prepared,
        chunks,
        postings: Default::default(),
        count: store.chunk_count,
        words: store.word_count,
        use_postings: true,
        dense: None,
        scan: None,
        eligible: None,
        excluded_owners: FastSet::default(),
        dfs: None,
        text: store.text_segment(),
        lexical_scores: Some(scores),
        screened: None,
    }
}

/// [`capture_ranked_lexical`] over a serving image, for queries [`overlay_serves`] accepts.
pub(super) fn capture_image(
    image: &ServingImage,
    prepared: PreparedSearch,
) -> anyhow::Result<CapturedQuery> {
    let mut chunks = Vec::new();
    let mut scores = Vec::new();
    if prepared.lexical_weight > 0.0 {
        let ranked =
            LEXICAL_SCRATCH.with(|scratch| scratch.borrow_mut().rank(image, &prepared, true));
        chunks.reserve_exact(ranked.len());
        scores.reserve_exact(ranked.len());
        let mut metadata = MetadataCache::default();
        for (score, slot) in ranked {
            let slot = slot as usize;
            chunks.push(CapturedChunk {
                owner: ChunkOwner::Image(Box::new(image.hit(slot, &mut metadata)?)),
                index: image.slot_order(slot).1,
                word_count: image.slot_word_count(slot),
                semantic_cache: None,
            });
            scores.push(score);
        }
    }
    Ok(CapturedQuery {
        prepared,
        chunks,
        postings: Default::default(),
        count: image.chunk_count(),
        words: image.word_count(),
        use_postings: true,
        dense: None,
        scan: None,
        eligible: None,
        excluded_owners: FastSet::default(),
        dfs: None,
        text: None,
        lexical_scores: Some(scores),
        screened: None,
    })
}

/// Whether `prepared` ranks from the checkpoint under `store` together with the store's own
/// documents as [`capture_ranked_lexical`] would from the store rebuilt from them: unfiltered,
/// lexical only, without a query embedding (so no chunk embedding is consulted), on a store
/// small enough for the per-slot ranking.
pub(in super::super) fn overlay_serves(prepared: &PreparedSearch, store: &DocumentStore) -> bool {
    store.base.as_ref().is_some_and(|base| {
        prepared.lexical_only
            && prepared.embedding.is_none()
            && (base.image.slot_total() + store.slot_count <= DENSE_SLOT_LIMIT
                || (base.intact() && store.documents.is_empty() && base.image.has_impacts()))
    })
}

/// [`capture_image`] over the checkpoint under `store` and the store's own documents.
pub(super) fn capture_overlaid(
    store: &DocumentStore,
    prepared: PreparedSearch,
) -> anyhow::Result<CapturedQuery> {
    let base = store.base.as_ref().expect("checked by overlay_serves");
    if base.intact() && store.documents.is_empty() {
        return capture_image(&base.image, prepared);
    }
    let source = Overlaid { base, store };
    let split = source.split();
    let mut chunks = Vec::new();
    let mut scores = Vec::new();
    if prepared.lexical_weight > 0.0 {
        let ranked =
            LEXICAL_SCRATCH.with(|scratch| scratch.borrow_mut().rank(&source, &prepared, false));
        chunks.reserve_exact(ranked.len());
        scores.reserve_exact(ranked.len());
        let mut metadata = MetadataCache::default();
        for (score, slot) in ranked {
            let slot = slot as usize;
            let word_count = source.slot_word_count(slot);
            chunks.push(match slot.checked_sub(split) {
                None => CapturedChunk {
                    owner: ChunkOwner::Image(Box::new(base.image.hit(slot, &mut metadata)?)),
                    index: base.image.slot_order(slot).1,
                    word_count,
                    semantic_cache: None,
                },
                Some(own) => {
                    let (document, index) = store.owned_chunk(own);
                    CapturedChunk::live(document, index, word_count)
                }
            });
            scores.push(score);
        }
    }
    Ok(CapturedQuery {
        prepared,
        chunks,
        postings: Default::default(),
        count: source.chunk_count(),
        words: source.word_count(),
        use_postings: true,
        dense: None,
        scan: None,
        eligible: None,
        excluded_owners: FastSet::default(),
        dfs: None,
        text: store.text_segment(),
        lexical_scores: Some(scores),
        screened: None,
    })
}

/// The top `prepared.limit` chunks [`LexicalScratch::rank`] scores from the checkpoint under
/// `store` and the store's own documents, as [`capture_overlaid`] ranks them but without
/// decoding a result per hit. For a query [`overlay_serves`] accepts.
pub(in super::super) fn rank_overlaid(
    store: &DocumentStore,
    prepared: &PreparedSearch,
) -> LexicalResults {
    let base = store.base.as_ref().expect("checked by overlay_serves");
    if base.intact() && store.documents.is_empty() {
        return ranked_hits(&*base.image, prepared, true);
    }
    ranked_hits(&Overlaid { base, store }, prepared, false)
}

/// [`rank_overlaid`] over a store holding its own documents.
pub(in super::super) fn rank_store(
    store: &DocumentStore,
    prepared: &PreparedSearch,
) -> LexicalResults {
    ranked_hits(store, prepared, false)
}

pub(super) fn ranked_hits<S: LexicalSource>(
    source: &S,
    prepared: &PreparedSearch,
    hot: bool,
) -> LexicalResults {
    let ranked = LEXICAL_SCRATCH.with(|scratch| scratch.borrow_mut().rank(source, prepared, hot));
    let hits = ranked
        .into_iter()
        .map(|(score, slot)| {
            let (document, chunk_index) = source.slot_order(slot as usize);
            LexicalHit {
                document_id: document.to_owned(),
                chunk_index,
                score,
            }
        })
        .collect();
    LexicalResults { hits, count: None }
}

pub(in super::super) fn capture_overlaid_search(
    store: &DocumentStore,
    prepared: Vec<PreparedSearch>,
) -> anyhow::Result<CapturedSearch> {
    Ok(CapturedSearch {
        queries: prepared
            .into_iter()
            .map(|prepared| capture_overlaid(store, prepared))
            .collect::<anyhow::Result<_>>()?,
    })
}

pub(super) fn capture_planned_query(
    store: &DocumentStore,
    prepared: PreparedSearch,
    dense: Option<Arc<DenseIndex>>,
) -> CapturedQuery {
    let mut eligible = None;
    let mut scan = None;
    let (count, words, dfs, ranked) = if prepared.unfiltered {
        let dfs = std::array::from_fn(|form| {
            prepared.query_terms[form]
                .iter()
                .map(|word| {
                    (
                        word.clone(),
                        store.postings[form].get(word).map_or(0, PostingList::len),
                    )
                })
                .collect()
        });
        let ranked = if prepared.lexical_weight > 0.0 {
            LEXICAL_SCRATCH
                .with(|scratch| scratch.borrow_mut().rank(store, &prepared, false))
                .into_iter()
                .map(|(score, slot)| (score, slot as usize))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        (store.chunk_count, store.word_count, dfs, ranked)
    } else {
        let candidates = store.candidate_ids(&prepared.filters);
        let mut documents: Vec<_> = match candidates {
            Some(ids) => ids.into_iter().map(|id| &store.documents[id]).collect(),
            None => store.documents.values().collect(),
        };
        documents.retain(|document| prepared.matches(document));
        let count: usize = documents.iter().map(|document| document.chunks.len()).sum();
        if count < super::super::dense::MIN_ROWS {
            return capture_postings(store, prepared);
        }
        if dense.is_some()
            && prepared.lexical_weight == 0.0
            && prepared.query_terms.iter().all(BTreeSet::is_empty)
        {
            scan = Some(
                documents
                    .iter()
                    .map(|document| Arc::clone(document))
                    .collect(),
            );
        }
        let posting_work = prepared
            .query_terms
            .iter()
            .enumerate()
            .flat_map(|(form, terms)| {
                terms
                    .iter()
                    .map(move |word| store.postings[form].get(word).map_or(0, PostingList::len))
            })
            .fold(0usize, usize::saturating_add);
        let scan_work = count.saturating_mul(prepared.query_terms.iter().map(BTreeSet::len).sum());
        let slots = (store.slot_count <= DENSE_SLOT_LIMIT
            && posting_work.saturating_add(store.slot_count) < scan_work)
            .then(|| {
                let mut slots = vec![false; store.slot_count];
                for document in &documents {
                    for &slot in &store.slots[&document.id] {
                        slots[slot] = true;
                    }
                }
                slots
            });
        let words = documents
            .iter()
            .flat_map(|document| &document.chunks)
            .map(|chunk| chunk.word_count)
            .sum();
        let dfs: [BTreeMap<String, usize>; 2] = std::array::from_fn(|form| {
            prepared.query_terms[form]
                .iter()
                .map(|word| {
                    let df = if let Some(slots) = &slots {
                        let mut count = 0;
                        if let Some(list) = store.postings[form].get(word) {
                            list.for_each(|slot, _| count += usize::from(slots[slot]));
                        }
                        count
                    } else {
                        documents
                            .iter()
                            .flat_map(|document| &document.chunks)
                            .filter(|chunk| {
                                chunk.stemmed == (form == 1) && chunk.terms.contains_key(word)
                            })
                            .count()
                    };
                    (word.clone(), df)
                })
                .collect()
        });
        let mut ranked = Vec::new();
        if let Some(slots) = slots {
            if prepared.lexical_weight > 0.0 {
                let source = FilteredLexical {
                    store,
                    slots: &slots,
                    dfs: &dfs,
                    count,
                    words,
                    identity: super::super::index::StoreIdentity::default(),
                };
                ranked = LEXICAL_SCRATCH
                    .with(|scratch| scratch.borrow_mut().rank(&source, &prepared, false))
                    .into_iter()
                    .map(|(score, slot)| (score, slot as usize))
                    .collect();
            }
            drop(slots);
            if dense.is_some() && documents.len() <= DENSE_SLOT_LIMIT {
                // The tree pins its owners, so current-owner membership also excludes replaced rows.
                eligible = Some(EligibleOwners::new(
                    documents
                        .into_iter()
                        .map(|document| Arc::as_ptr(document) as usize)
                        .collect(),
                ));
            }
        } else if prepared.lexical_weight > 0.0 {
            let idfs: [Vec<_>; 2] = std::array::from_fn(|form| {
                dfs[form]
                    .iter()
                    .map(|(word, &df)| (word, Bm25::idf(count as f64, df as f64)))
                    .collect()
            });
            let average = (words as f64 / count as f64).max(1.0);
            let compare = |a: &(f64, usize), b: &(f64, usize)| {
                b.0.total_cmp(&a.0)
                    .then_with(|| store.slot_order(a.1).cmp(&store.slot_order(b.1)))
            };
            for document in documents {
                for (&slot, chunk) in store.slots[&document.id].iter().zip(&document.chunks) {
                    let norm = prepared.bm25.norm(chunk.word_count as f64, average);
                    let score: f64 = idfs[usize::from(chunk.stemmed)]
                        .iter()
                        .map(|(word, idf)| {
                            prepared.bm25.weight(
                                *idf,
                                f64::from(chunk.terms.get(word).copied().unwrap_or(0)),
                                norm,
                            )
                        })
                        .sum();
                    if score > 0.0 {
                        ranked.push((score, slot));
                        if ranked.len() >= 2 * prepared.limit {
                            top_ranked(&mut ranked, prepared.limit, compare);
                        }
                    }
                }
            }
            top_ranked(&mut ranked, prepared.limit, compare);
        }
        (count, words, dfs, ranked)
    };
    let mut chunks: Vec<_> = ranked
        .into_iter()
        .map(|(_, slot)| {
            let (document, index) = store.owned_chunk(slot);
            CapturedChunk::live(document, index, store.slot_word_count(slot))
        })
        .collect();
    if dense.is_some() && !store.dense_changes.is_empty() {
        let mut captured_slots: FastSet<_> = chunks
            .iter()
            .map(|chunk| {
                let (id, index) = chunk.order();
                store.slots[id][index]
            })
            .collect();
        for id in store.dense_changes.keys() {
            let Some(document) = store
                .documents
                .get(id)
                .filter(|document| prepared.matches(document))
            else {
                continue;
            };
            for (index, (&slot, chunk)) in store.slots[id].iter().zip(&document.chunks).enumerate()
            {
                if chunk.embedding.is_some() && captured_slots.insert(slot) {
                    chunks.push(CapturedChunk::live(
                        Arc::clone(document),
                        index,
                        store.slot_word_count(slot),
                    ));
                }
            }
        }
    }
    let excluded_owners = if dense.is_some() && eligible.is_none() {
        store
            .dense_changes
            .values()
            .filter_map(|change| change.previous_owner)
            .collect()
    } else {
        FastSet::default()
    };
    CapturedQuery {
        prepared,
        chunks,
        postings: Default::default(),
        count,
        words,
        use_postings: false,
        dense,
        scan,
        eligible,
        excluded_owners,
        dfs: Some(dfs),
        text: store.text_segment(),
        lexical_scores: None,
        screened: None,
    }
}
