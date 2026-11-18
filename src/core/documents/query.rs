// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Whether every query in `prepared` ranks from the checkpoint under `store` and the store's
/// own documents (see `search::overlay_serves`).
pub(crate) fn overlay_serves(store: &DocumentStore, prepared: &[PreparedSearch]) -> bool {
    prepared
        .iter()
        .all(|query| search::overlay_serves(query, store))
}

/// The hits of [`plain_lexical_search`]'s `prepared` ranked from the checkpoint under `store`;
/// `prepared` must pass [`overlay_serves`].
pub(crate) fn rank_lexical_overlaid(
    store: &DocumentStore,
    prepared: &PreparedSearch,
) -> LexicalResults {
    search::rank_overlaid(store, prepared)
}

/// Whether a store holding its documents is too large for the exhaustive per-slot ranking of
/// [`rank_lexical`], so a plain top-k query runs through the pruned [`execute_lexical`] instead
/// (the same hits, scores to the bit and tie order).
pub(crate) fn prefers_pruned_lexical(store: &DocumentStore) -> bool {
    store.slot_count > search::DENSE_SLOT_LIMIT
}

/// The hits of [`plain_lexical_search`]'s `prepared` ranked over a store holding its documents.
pub(crate) fn rank_lexical(store: &DocumentStore, prepared: &PreparedSearch) -> LexicalResults {
    search::rank_store(store, prepared)
}

pub(crate) fn capture_overlaid_search(
    store: &DocumentStore,
    prepared: Vec<PreparedSearch>,
) -> Result<CapturedSearch> {
    search::capture_overlaid_search(store, prepared)
}

/// The checkpoint of `store` for the generation whose name has CRC32 `generation`; `None` when
/// a chunk is not lexical-only or the store would not fit the layout.
pub(crate) fn encode_checkpoint(
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
    compress_text: bool,
) -> Result<Option<Vec<u8>>> {
    if compress_text {
        image::encode_compressed(store, generation, dimensions)
    } else {
        image::encode(store, generation, dimensions)
    }
}
/// Streams the checkpoint of `store` (with the checkpoint under it, if any) to `destination`;
/// see `image::merge`.
pub(crate) fn merge_checkpoint(
    store: &DocumentStore,
    generation: u32,
    dimensions: usize,
    destination: &std::path::Path,
    order: Option<&image::SlotOrder>,
) -> Result<Option<u64>> {
    let Some(base) = store.merge_base() else {
        return Ok(None);
    };
    image::merge(base, store, generation, dimensions, destination, order)
}

pub(crate) use image::SlotOrder;

/// The bisection slot order [`merge_checkpoint`] can write `store` in (see
/// `image::bisection_order`) and the estimate of the bytes it holds; `None` when the store's
/// checkpoint cannot be merged, no order when the estimate is over `budget` or the store has
/// fewer than [`REORDER_MIN_DOCUMENTS`] documents.
pub(crate) fn checkpoint_slot_order(
    store: &DocumentStore,
    budget: Option<usize>,
) -> Result<Option<(usize, Option<SlotOrder>)>> {
    let Some(base) = store.merge_base() else {
        return Ok(None);
    };
    image::bisection_order(base, store, index::REORDER_MIN_DOCUMENTS, budget).map(Some)
}
#[cfg(test)]
pub(crate) fn search(
    store: &DocumentStore,
    text: &str,
    options: &SearchOptions,
    embedding_dimensions: Option<usize>,
) -> Result<Vec<DocumentResult>> {
    let prepared = prepare_search(text, options, embedding_dimensions)?;
    let captured = capture_search(store, vec![prepared]);
    execute_search(&captured, 0)
}

#[cfg(test)]
pub(super) fn eligible_chunks<'a>(
    documents: impl Iterator<Item = &'a StoredDocument>,
    options: &SearchOptions,
) -> Vec<(&'a StoredDocument, &'a StoredChunk)> {
    documents
        .filter(|document| matches_document(document, options))
        .flat_map(|d| d.chunks.iter().map(move |chunk| (d, chunk)))
        .collect()
}

#[cfg(test)]
pub(super) fn matches_document(document: &StoredDocument, options: &SearchOptions) -> bool {
    options
        .document_ids
        .as_ref()
        .is_none_or(|ids| ids.contains(&document.id))
        && matches_document_metadata(document, options)
}

pub(super) fn matches_document_metadata(
    document: &StoredDocument,
    options: &SearchOptions,
) -> bool {
    options
        .source_uri
        .as_ref()
        .is_none_or(|uri| document.source_uri.as_ref() == Some(uri))
        && matches_metadata(&document.metadata, options.filter.as_ref())
}

pub(crate) fn matches_metadata(metadata: &Value, filter: Option<&Value>) -> bool {
    filter.and_then(Value::as_object).is_none_or(|filter| {
        filter
            .iter()
            .all(|(key, value)| metadata.get(key) == Some(value))
    })
}

pub(super) fn top_ranked<T>(
    values: &mut Vec<T>,
    limit: usize,
    mut compare: impl FnMut(&T, &T) -> std::cmp::Ordering,
) {
    if values.len() > limit {
        values.select_nth_unstable_by(limit - 1, &mut compare);
        values.truncate(limit);
    }
    values.sort_unstable_by(compare);
}

#[cfg(test)]
pub(super) fn search_impl(
    store: &DocumentStore,
    text: &str,
    options: &SearchOptions,
    embedding_dimensions: Option<usize>,
    indexed: bool,
) -> Result<Vec<DocumentResult>> {
    if indexed {
        return search(store, text, options, embedding_dimensions);
    }
    ensure!(text.len() <= MAX_DOCUMENT_BYTES, "query exceeds 8 MiB");
    let k = options.k.unwrap_or(10) as usize;
    let limit = options.candidate_limit.unwrap_or(100.max(k as u32)) as usize;
    ensure!(
        (1..=1000).contains(&k) && (k..=4096).contains(&limit),
        "require 1 <= k <= 1000 and k <= candidateLimit <= 4096"
    );
    ensure!(
        options.filter.as_ref().is_none_or(Value::is_object),
        "filter must be a JSON object of exact metadata matches"
    );
    let lexical_weight = options.lexical_weight.unwrap_or(1.0);
    let semantic_weight = options.semantic_weight.unwrap_or(1.0);
    ensure!(
        [lexical_weight, semantic_weight]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
            && lexical_weight + semantic_weight > 0.0,
        "search weights must be finite, nonnegative and not both zero"
    );
    let bm25 = Bm25::from_options(options)?;
    let minimum = options.min_semantic_score.unwrap_or(0.0);
    ensure!(
        minimum.is_finite() && (-1.0..=1.0).contains(&minimum),
        "minSemanticScore must be in [-1,1]"
    );
    let rrf = match options.fusion.as_deref() {
        None | Some("blend") => false,
        Some("rrf") => true,
        Some(other) => anyhow::bail!("fusion must be \"blend\" or \"rrf\", not {other:?}"),
    };
    let query_embedding = options
        .embedding
        .as_ref()
        .map(|v| {
            ensure!(
                embedding_dimensions.is_some(),
                "configure an embedding space before semantic search"
            );
            normalize_embedding(v, embedding_dimensions.unwrap_or(0))
        })
        .transpose()?;
    // Unstemmed and stemmed query forms: a chunk is scored with the form it was stored in.
    let query_terms = [query_words(text, false), query_words(text, true)];
    let candidates = if indexed {
        store.candidate_ids(options)
    } else {
        None
    };
    let eligible = match candidates {
        Some(ids) => eligible_chunks(
            ids.into_iter().map(|id| {
                store
                    .documents
                    .get(id)
                    .expect("document candidate is absent from store")
                    .as_ref()
            }),
            options,
        ),
        None => eligible_chunks(store.values(), options),
    };
    if eligible.is_empty() {
        return Ok(Vec::new());
    }
    let n = eligible.len() as f64;
    let average = eligible.iter().map(|(_, c)| c.word_count).sum::<usize>() as f64 / n;
    let posting_work: usize = query_terms
        .iter()
        .enumerate()
        .map(|(form, terms)| {
            terms
                .iter()
                .map(|word| store.postings[form].get(word).map_or(0, |p| p.len()))
                .sum::<usize>()
        })
        .sum();
    let scan_work = eligible.len().saturating_mul(
        query_terms
            .iter()
            .map(std::collections::BTreeSet::len)
            .sum(),
    );
    let use_postings = indexed && posting_work.saturating_add(store.slot_count) < scan_work;
    let mut positions = Vec::new();
    if use_postings {
        positions.resize(store.slot_count, usize::MAX);
        let mut previous = "";
        let mut chunk = 0;
        for (i, (document, _)) in eligible.iter().enumerate() {
            if document.id != previous {
                previous = &document.id;
                chunk = 0;
            }
            positions[store.slots[&document.id][chunk]] = i;
            chunk += 1;
        }
    }
    let idf: [BTreeMap<&String, f64>; 2] = std::array::from_fn(|form| {
        let stemmed = form == 1;
        query_terms[form]
            .iter()
            .map(|word| {
                let df = if use_postings {
                    store.postings[form].get(word).map_or(0, |posting| {
                        posting
                            .iter()
                            .filter(|&(slot, _)| positions[slot] != usize::MAX)
                            .count()
                    })
                } else {
                    eligible
                        .iter()
                        .filter(|(_, c)| c.stemmed == stemmed && c.terms.contains_key(word))
                        .count()
                } as f64;
                (word, Bm25::idf(n, df))
            })
            .collect()
    });
    let mut scores: Vec<(f64, Option<f64>)> = eligible
        .iter()
        .map(|(_, c)| {
            let lexical = if use_postings {
                0.0
            } else {
                idf[usize::from(c.stemmed)]
                    .iter()
                    .map(|(word, idf)| {
                        let tf = c.terms.get(word).copied().unwrap_or(0) as f64;
                        bm25.weight(*idf, tf, bm25.norm(c.word_count as f64, average.max(1.0)))
                    })
                    .sum()
            };
            let semantic = query_embedding
                .as_ref()
                .zip(c.embedding.as_ref())
                .map(|(q, v)| v.cosine(q));
            (lexical, semantic)
        })
        .collect();
    if use_postings {
        for (form, weights) in idf.iter().enumerate() {
            for (word, idf) in weights {
                if let Some(posting) = store.postings[form].get(word) {
                    for (slot, frequency) in posting.iter() {
                        let i = positions[slot];
                        if i == usize::MAX {
                            continue;
                        }
                        let c = eligible[i].1;
                        let tf = f64::from(frequency);
                        scores[i].0 +=
                            bm25.weight(*idf, tf, bm25.norm(c.word_count as f64, average.max(1.0)));
                    }
                }
            }
        }
    }
    let mut lexical: Vec<usize> = (0..scores.len())
        .filter(|&i| scores[i].0 > 0.0 && lexical_weight > 0.0)
        .collect();
    let mut semantic: Vec<usize> = (0..scores.len())
        .filter(|&i| semantic_weight > 0.0 && scores[i].1.is_some_and(|s| s >= minimum))
        .collect();
    top_ranked(&mut lexical, limit, |&a, &b| {
        scores[b].0.total_cmp(&scores[a].0).then(a.cmp(&b))
    });
    top_ranked(&mut semantic, limit, |&a, &b| {
        scores[b]
            .1
            .unwrap_or(-1.0)
            .total_cmp(&scores[a].1.unwrap_or(-1.0))
            .then(a.cmp(&b))
    });
    let mut fused = BTreeMap::<usize, f64>::new();
    for (list, (indices, weight)) in [(&lexical, lexical_weight), (&semantic, semantic_weight)]
        .into_iter()
        .enumerate()
    {
        let top = &indices[..indices.len().min(limit)];
        if rrf {
            for (rank, &i) in top.iter().enumerate() {
                *fused.entry(i).or_default() += weight / (60.0 + rank as f64 + 1.0);
            }
            continue;
        }
        // Min-max over the list's own top candidates; a flat list contributes its weight.
        let value = |i: usize| {
            if list == 0 {
                scores[i].0
            } else {
                scores[i].1.unwrap_or(-1.0)
            }
        };
        let (hi, lo) = match (top.first(), top.last()) {
            (Some(&f), Some(&l)) => (value(f), value(l)),
            _ => continue,
        };
        for &i in top {
            let norm = if hi > lo {
                (value(i) - lo) / (hi - lo)
            } else {
                1.0
            };
            *fused.entry(i).or_default() += weight * norm;
        }
    }
    let mut ranked: Vec<_> = fused.into_iter().collect();
    ranked.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(ranked
        .into_iter()
        .take(k)
        .map(|(i, score)| {
            let (d, c) = eligible[i];
            DocumentResult {
                id: c.id.clone(),
                document_id: d.id.clone(),
                source_uri: d.source_uri.clone(),
                version: d.version.clone(),
                metadata: (*d.metadata).clone(),
                text: c.text.clone(),
                start_byte: c.start_byte,
                end_byte: c.end_byte,
                score,
                lexical_score: scores[i].0,
                semantic_score: scores[i].1,
            }
        })
        .collect())
}
