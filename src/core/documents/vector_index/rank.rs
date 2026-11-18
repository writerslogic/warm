// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(super) struct Hit {
    pub(super) document: Arc<StoredDocument>,
    pub(super) chunk: u32,
    /// Replaced by the canonical `f64` score before ranking.
    pub(super) score: f64,
}

pub(super) fn compare_hits(a: &Hit, b: &Hit) -> Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.document.id.cmp(&b.document.id))
        .then(a.chunk.cmp(&b.chunk))
}

pub(super) fn append_graph_delta(
    snapshot: &VectorSnapshot,
    document: &Arc<StoredDocument>,
    found: &mut Vec<Hit>,
) -> Result<()> {
    let row = snapshot
        .rows
        .partition_point(|row| row.document.id < document.id);
    if snapshot
        .rows
        .get(row)
        .is_some_and(|row| Arc::ptr_eq(&row.document, document))
    {
        return Ok(());
    }
    for (chunk, stored) in document.chunks.iter().enumerate() {
        let Some(embedding) = &stored.embedding else {
            continue;
        };
        if embedding.len() != snapshot.dim {
            continue;
        }
        found.push(Hit {
            document: Arc::clone(document),
            chunk: u32::try_from(chunk).context("chunk index over u32")?,
            score: 0.0,
        });
    }
    Ok(())
}

pub(super) fn to_result(hit: &Hit, segment: Option<&TextSegment>) -> Result<DocumentResult> {
    let document = &hit.document;
    let chunk = &document.chunks[hit.chunk as usize];
    let score = hit.score.clamp(-1.0, 1.0);
    Ok(DocumentResult {
        id: chunk.id.clone(),
        document_id: document.id.clone(),
        source_uri: document.source_uri.clone(),
        version: document.version.clone(),
        metadata: (*document.metadata).clone(),
        text: try_text_of(chunk, segment)?,
        start_byte: chunk.start_byte,
        end_byte: chunk.end_byte,
        score,
        lexical_score: 0.0,
        semantic_score: Some(score),
    })
}

/// Answers an unfiltered vector query from the snapshot. `query` is l2-normalised. `None`
/// means the snapshot is too stale to answer this query (the exact delta exceeds its
/// capture limits, fewer than `k` hits survived validation, or a hit's text was scrubbed
/// before it was read); the caller takes the generic exact path.
#[cfg(test)]
pub(crate) fn search_vector_snapshot(
    snapshot: &VectorSnapshot,
    store: &RwLock<DocumentStore>,
    query: &[f32],
    exact_query: &[f64],
    k: usize,
    options: &VectorQueryOptions,
) -> Result<Option<Vec<DocumentResult>>> {
    Ok(
        search_vector_snapshot_with_report(snapshot, store, query, exact_query, k, options)?
            .map(|response| response.results),
    )
}

pub(crate) fn search_vector_snapshot_with_report(
    snapshot: &VectorSnapshot,
    store: &RwLock<DocumentStore>,
    query: &[f32],
    exact_query: &[f64],
    k: usize,
    options: &VectorQueryOptions,
) -> Result<Option<VectorSearchResults>> {
    let Some(ranked) = rank_vector_snapshot(snapshot, store, query, exact_query, k, options)?
    else {
        return Ok(None);
    };
    let diagnostics = ranked.diagnostics;
    Ok(
        read_vector_hits(ranked, &store.read())?.map(|results| VectorSearchResults {
            results,
            diagnostics,
        }),
    )
}

/// Ranked snapshot hits; [`read_vector_hits`] reads their text.
pub(crate) struct RankedVectorHits {
    pub(super) hits: Vec<Hit>,
    pub(super) segment: Option<Arc<TextSegment>>,
    pub(super) diagnostics: VectorSearchDiagnostics,
}

pub(super) struct StaleExactCapture {
    pub(super) live_rows: Vec<u64>,
    pub(super) delta: Vec<Arc<StoredDocument>>,
    pub(super) segment: Option<Arc<TextSegment>>,
}

pub(super) fn capture_stale_exact(
    snapshot: &VectorSnapshot,
    store: &DocumentStore,
) -> Result<Option<StaleExactCapture>> {
    if snapshot.identity != store.identity {
        return Ok(None);
    }
    let changes = store.vector_changes.read();
    let Some(changes) = changes.since(snapshot.generation, store.generation) else {
        return Ok(None);
    };
    let mut live_rows = Vec::new();
    let words = snapshot.rows.len().div_ceil(64);
    live_rows
        .try_reserve_exact(words)
        .context("cannot allocate vector snapshot eligibility")?;
    live_rows.resize(words, u64::MAX);
    let mut delta = Vec::new();
    delta
        .try_reserve_exact(MAX_DELTA_DOCUMENTS.min(store.documents.len()))
        .context("cannot allocate vector snapshot delta owners")?;
    let mut chunks = 0;
    for id in changes {
        let start = snapshot
            .rows
            .partition_point(|row| row.document.id.as_str() < id);
        let end =
            start + snapshot.rows[start..].partition_point(|row| row.document.id.as_str() == id);
        if start < end {
            let first = start / 64;
            let last = (end - 1) / 64;
            let first_mask = u64::MAX << (start % 64);
            let last_mask = u64::MAX >> (63 - (end - 1) % 64);
            if first == last {
                live_rows[first] &= !(first_mask & last_mask);
            } else {
                live_rows[first] &= !first_mask;
                live_rows[first + 1..last].fill(0);
                live_rows[last] &= !last_mask;
            }
        }
        let Some(document) = store.get(id) else {
            continue;
        };
        let before = chunks;
        for chunk in &document.chunks {
            if chunk
                .embedding
                .as_ref()
                .is_some_and(|embedding| embedding.len() == snapshot.dim)
            {
                chunks += 1;
                if chunks > MAX_DELTA_CHUNKS {
                    return Ok(None);
                }
            }
        }
        if chunks != before {
            delta.push(Arc::clone(document));
        }
    }
    Ok(Some(StaleExactCapture {
        live_rows,
        delta,
        segment: store.text_segment(),
    }))
}

/// The hits with their text, or `None` when the text of one was scrubbed since the ranking
/// (its document replaced or deleted). `_store` is the store's read guard: logical hiding
/// is published under its write lock before physical erasure, preventing reads of hidden text.
pub(crate) fn read_vector_hits(
    ranked: RankedVectorHits,
    _store: &DocumentStore,
) -> Result<Option<Vec<DocumentResult>>> {
    let segment = ranked.segment.as_deref();
    ranked
        .hits
        .iter()
        .map(|hit| {
            let scrubbed = text_scrubbed(&hit.document.chunks[hit.chunk as usize], segment);
            if scrubbed {
                Ok(None)
            } else {
                to_result(hit, segment).map(Some)
            }
        })
        .collect()
}

pub(crate) fn rank_vector_snapshot(
    snapshot: &VectorSnapshot,
    store: &RwLock<DocumentStore>,
    query: &[f32],
    exact_query: &[f64],
    k: usize,
    options: &VectorQueryOptions,
) -> Result<Option<RankedVectorHits>> {
    ensure!(
        query.len() == snapshot.dim,
        "query has {} dimensions, the index {}",
        query.len(),
        snapshot.dim
    );
    ensure!(
        exact_query.len() == snapshot.dim,
        "canonical query dimension mismatch"
    );
    ensure!((1..=MAX_K).contains(&k), "require 1 <= k <= {MAX_K}");
    ensure!(
        (1..=MAX_EF).contains(&options.ef),
        "require 1 <= ef <= {MAX_EF}"
    );
    ensure!(
        options.min_score.is_finite() && (-1.0..=1.0).contains(&options.min_score),
        "minScore must be in [-1,1]"
    );
    let index = snapshot.index(options.mode)?;
    let (generation, mut segment) = {
        let store = store.read();
        if snapshot.identity != store.identity {
            return Ok(None);
        }
        (store.generation, store.text_segment())
    };
    let stale = staleness(generation, snapshot.generation);
    let exact_index = index.kind() == VectorIndexKind::Exact;
    let extra = usize::try_from(stale).unwrap_or(usize::MAX).min(4 * k);
    let fetch = (k + extra + 8).min(snapshot.rows());
    if stale != 0 && (exact_index || snapshot.candidate_identity.is_some()) {
        return rank_stale_exact_snapshot(
            snapshot,
            store,
            exact_query,
            k,
            fetch,
            options.min_score,
        );
    }
    let fetch = if !exact_index {
        if let Some(limit) = snapshot.rerank_candidates {
            ensure!(k <= limit, "k exceeds configured vector rerank candidates");
            limit.min(snapshot.rows())
        } else {
            fetch
        }
    } else {
        fetch
    };
    let graph_hits = if exact_index {
        Vec::new()
    } else {
        let Some(hits) = snapshot
            .graph
            .as_ref()
            .expect("selected graph missing")
            .search_candidate_ids(query, fetch, options.ef)?
        else {
            return Ok(None);
        };
        hits
    };
    let mut found: Vec<Hit> = Vec::with_capacity(fetch);
    if stale == 0 {
        let hits = if exact_index {
            snapshot
                .exact
                .search_canonical(exact_query, fetch, |_| true)
        } else {
            graph_hits.into_iter().map(|row| (row, 0.0)).collect()
        };
        for (row, score) in hits {
            let row = &snapshot.rows[row as usize];
            found.push(Hit {
                document: Arc::clone(&row.document),
                chunk: row.chunk,
                score,
            });
        }
    } else {
        #[cfg(test)]
        tests::before_stale_graph_capture();
        let store = store.read();
        if snapshot.identity != store.identity {
            return Ok(None);
        }
        segment = store.text_segment();
        for row in graph_hits {
            let row = &snapshot.rows[row as usize];
            let live = store
                .get(&row.document.id)
                .is_some_and(|live| Arc::ptr_eq(live, &row.document));
            if live {
                found.push(Hit {
                    document: Arc::clone(&row.document),
                    chunk: row.chunk,
                    score: 0.0,
                });
            }
        }
        let changed = {
            let changes = store.vector_changes.read();
            let Some(ids) = changes.since(snapshot.generation, store.generation) else {
                return Ok(None);
            };
            let mut changed = Vec::new();
            changed
                .try_reserve_exact(MAX_DELTA_DOCUMENTS.min(store.documents.len()))
                .context("cannot allocate graph delta owners")?;
            let mut chunks = 0usize;
            for document in ids.filter_map(|id| store.get(id)) {
                for chunk in &document.chunks {
                    if chunk
                        .embedding
                        .as_ref()
                        .is_some_and(|embedding| embedding.len() == snapshot.dim)
                    {
                        chunks += 1;
                        if chunks > MAX_DELTA_CHUNKS {
                            return Ok(None);
                        }
                    }
                }
                changed.push(Arc::clone(document));
            }
            changed
        };
        drop(store);
        for document in &changed {
            append_graph_delta(snapshot, document, &mut found)?;
        }
        if found.len() < k.min(snapshot.rows()) {
            return Ok(None);
        }
    }
    let mut ranked = finish_vector_hits(found, segment, exact_query, k, options.min_score);
    ranked.diagnostics = VectorSearchDiagnostics {
        route: if exact_index {
            if snapshot.zero_prefix && options.mode != VectorIndexMode::Exact {
                VectorSearchRoute::FallbackExact
            } else {
                VectorSearchRoute::FullExact
            }
        } else if snapshot
            .candidate_identity
            .as_ref()
            .is_some_and(|identity| identity.dimensions < snapshot.dim)
        {
            VectorSearchRoute::ProjectedGraph
        } else {
            VectorSearchRoute::FullGraph
        },
        candidate_dimensions: if exact_index {
            snapshot.dim
        } else {
            snapshot
                .candidate_identity
                .as_ref()
                .map_or(snapshot.dim, |identity| identity.dimensions)
        },
        shortlist_limit: (!exact_index).then_some(fetch),
        graph_degree: (!exact_index).then(|| {
            snapshot
                .graph
                .as_ref()
                .expect("selected graph missing")
                .build_degree
        }),
        graph_build_ef: (!exact_index).then(|| {
            snapshot
                .graph
                .as_ref()
                .expect("selected graph missing")
                .build_ef
        }),
    };
    Ok(Some(ranked))
}

#[inline(never)]
pub(super) fn rank_stale_exact_snapshot(
    snapshot: &VectorSnapshot,
    store: &RwLock<DocumentStore>,
    exact_query: &[f64],
    k: usize,
    fetch: usize,
    min_score: f64,
) -> Result<Option<RankedVectorHits>> {
    let captured = {
        let store = store.read();
        capture_stale_exact(snapshot, &store)?
    };
    let Some(captured) = captured else {
        return Ok(None);
    };
    let segment = captured.segment;
    let mut found = Vec::with_capacity(fetch);
    let hits = snapshot.exact.search_canonical(exact_query, k, |row| {
        captured.live_rows[row / 64] & (1 << (row % 64)) != 0
    });
    for (row, score) in hits {
        let row = &snapshot.rows[row as usize];
        found.push(Hit {
            document: Arc::clone(&row.document),
            chunk: row.chunk,
            score,
        });
    }
    for document in &captured.delta {
        for (chunk, stored) in document.chunks.iter().enumerate() {
            if stored
                .embedding
                .as_ref()
                .is_some_and(|embedding| embedding.len() == snapshot.dim)
            {
                found.push(Hit {
                    document: Arc::clone(document),
                    chunk: u32::try_from(chunk).context("chunk index over u32")?,
                    score: 0.0,
                });
            }
        }
    }
    if found.len() < k.min(snapshot.rows()) {
        return Ok(None);
    }

    let mut ranked = finish_vector_hits(found, segment, exact_query, k, min_score);
    ranked.diagnostics.route = VectorSearchRoute::StaleExact;
    Ok(Some(ranked))
}

pub(super) fn finish_vector_hits(
    mut found: Vec<Hit>,
    segment: Option<Arc<TextSegment>>,
    exact_query: &[f64],
    k: usize,
    min_score: f64,
) -> RankedVectorHits {
    for hit in &mut found {
        if let Some(embedding) = hit
            .document
            .chunks
            .get(hit.chunk as usize)
            .and_then(|chunk| chunk.embedding.as_ref())
        {
            hit.score = embedding.cosine(exact_query);
        }
    }
    found.sort_by(compare_hits);
    found.retain(|hit| hit.score >= min_score);
    found.truncate(k);
    RankedVectorHits {
        hits: found,
        segment,
        diagnostics: VectorSearchDiagnostics {
            route: VectorSearchRoute::FullExact,
            candidate_dimensions: exact_query.len(),
            shortlist_limit: None,
            graph_degree: None,
            graph_build_ef: None,
        },
    }
}
