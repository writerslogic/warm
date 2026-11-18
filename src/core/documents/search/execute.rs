// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl CapturedSearch {
    pub(crate) fn len(&self) -> usize {
        self.queries.len()
    }

    pub(crate) fn into_prepared(self) -> Vec<PreparedSearch> {
        self.queries
            .into_iter()
            .map(|query| query.prepared)
            .collect()
    }
}

/// A ranked result whose chunk text is not read yet (see [`read_ranked`]).
pub(crate) struct RankedHit {
    /// The scored result; a checkpoint hit's other fields stay in the capture until read.
    result: DocumentResult,
    source: HitSource,
}

pub(super) enum HitSource {
    /// A live chunk, its text read from its document.
    Live(Arc<StoredDocument>, usize),
    /// The capture's chunk at this position, a checkpoint chunk decoded at capture.
    Image(usize),
}

impl RankedHit {
    /// Whether the hit's text was scrubbed since the capture; a checkpoint hit's was read at
    /// capture.
    fn scrubbed(&self, segment: Option<&super::super::text::TextSegment>) -> bool {
        match &self.source {
            HitSource::Live(document, chunk) => {
                super::super::index::text_scrubbed(&document.chunks[*chunk], segment)
            }
            HitSource::Image(_) => false,
        }
    }

    /// The result, a checkpoint hit's fields from `image(position)`.
    fn read(
        self,
        segment: Option<&super::super::text::TextSegment>,
        image: impl FnOnce(usize) -> DocumentResult,
    ) -> Result<DocumentResult> {
        let mut result = self.result;
        match self.source {
            HitSource::Live(document, chunk) => {
                result.text = super::super::index::try_text_of(&document.chunks[chunk], segment)?;
            }
            HitSource::Image(position) => {
                result = DocumentResult {
                    score: result.score,
                    lexical_score: result.lexical_score,
                    semantic_score: result.semantic_score,
                    ..image(position)
                };
            }
        }
        Ok(result)
    }
}

/// A result with no fields read yet.
pub(super) fn unread_result() -> DocumentResult {
    DocumentResult {
        id: String::new(),
        document_id: String::new(),
        source_uri: None,
        version: String::new(),
        metadata: Value::Null,
        text: None,
        start_byte: 0,
        end_byte: 0,
        score: 0.0,
        lexical_score: 0.0,
        semantic_score: None,
    }
}

pub(super) fn image_hit(chunks: &mut [CapturedChunk], position: usize) -> &mut DocumentResult {
    match &mut chunks[position].owner {
        ChunkOwner::Image(hit) => hit,
        ChunkOwner::Live(_) => unreachable!("an image hit names a checkpoint chunk"),
    }
}

/// Ranks and reads a capture. A replace or delete scrubs the text it supersedes, so a capture
/// that outlived the store's locks reads such text as empty (see [`read_ranked`]).
pub(crate) fn execute_search(
    captured: &CapturedSearch,
    query: usize,
) -> Result<Vec<DocumentResult>> {
    let captured_query = &captured.queries[query];
    let segment = captured_query.text.as_deref();
    rank_search(captured, query)
        .into_iter()
        .map(|hit| {
            hit.read(segment, |position| {
                match &captured_query.chunks[position].owner {
                    ChunkOwner::Image(hit) => DocumentResult::clone(hit),
                    ChunkOwner::Live(_) => unreachable!("an image hit names a checkpoint chunk"),
                }
            })
        })
        .collect()
}

/// [`execute_search`] without reading the hits' text, so ranking needs no lock.
pub(crate) fn rank_search(captured: &CapturedSearch, query: usize) -> Vec<RankedHit> {
    execute_query(&captured.queries[query])
}

/// The hits with their text, or `None` when the text of one was scrubbed since the capture
/// (its document replaced or deleted). The caller holds the mutation gate's read side, which
/// excludes the writes that scrub, so a read never overlaps one. Checkpoint hits move out of
/// the capture, which is not ranked again.
pub(crate) fn read_ranked(
    captured: &mut CapturedSearch,
    query: usize,
    hits: Vec<RankedHit>,
) -> Result<Option<Vec<DocumentResult>>> {
    let CapturedQuery { text, chunks, .. } = &mut captured.queries[query];
    let segment = text.as_deref();
    hits.into_iter()
        .map(|hit| {
            if hit.scrubbed(segment) {
                Ok(None)
            } else {
                hit.read(segment, |position| {
                    std::mem::replace(image_hit(chunks, position), unread_result())
                })
                .map(Some)
            }
        })
        .collect()
}

pub(super) fn execute_query(captured: &CapturedQuery) -> Vec<RankedHit> {
    if let Some(index) = &captured.dense {
        let query = captured
            .prepared
            .embedding
            .as_ref()
            .expect("indexed search lost its query embedding");
        let matches = |document: &StoredDocument| {
            captured.eligible.as_ref().map_or_else(
                || {
                    !captured
                        .excluded_owners
                        .contains(&(document as *const StoredDocument as usize))
                        && captured.prepared.matches(document)
                },
                |eligible| eligible.contains(&(document as *const StoredDocument as usize)),
            )
        };
        let (hits, _) = if let Some(documents) = &captured.scan {
            let work = captured.count.saturating_mul(query.len() + 1) / 16;
            match index.search_where_limited(
                query,
                captured.prepared.limit,
                captured.prepared.minimum,
                matches,
                work,
            ) {
                Some(result) => result,
                None => {
                    let mut chunks = Vec::with_capacity(captured.count);
                    for document in documents {
                        for (index, chunk) in document.chunks.iter().enumerate() {
                            chunks.push(CapturedChunk::live(
                                Arc::clone(document),
                                index,
                                super::super::index::slot_words(chunk.word_count),
                            ));
                        }
                    }
                    return execute_chunks(captured, &chunks);
                }
            }
        } else if captured.prepared.unfiltered && captured.excluded_owners.is_empty() {
            index.search(query, captured.prepared.limit, captured.prepared.minimum)
        } else {
            index.search_where(
                query,
                captured.prepared.limit,
                captured.prepared.minimum,
                matches,
            )
        };
        let mut chunks = captured.chunks.clone();
        let mut positions: FastMap<_, _> = captured
            .chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| (chunk.order(), i))
            .collect();
        for hit in &hits {
            let key = (hit.document.id.as_str(), hit.chunk);
            if let Some(&i) = positions.get(&key) {
                chunks[i].semantic_cache = Some(hit.score);
            } else {
                positions.insert(key, chunks.len());
                chunks.push(CapturedChunk {
                    semantic_cache: Some(hit.score),
                    ..CapturedChunk::live(
                        Arc::clone(&hit.document),
                        hit.chunk,
                        super::super::index::slot_words(hit.document.chunks[hit.chunk].word_count),
                    )
                });
            }
        }
        return execute_chunks(captured, &chunks);
    }
    execute_chunks(captured, &captured.chunks)
}

/// Chunk count from which the semantic scan screens every embedding in `f32` and scores exactly only the chunks that
/// can reach the ranked list; below it every chunk is scored exactly. The result is identical either way.
pub(super) const SCREEN_MIN_ROWS: usize = 256;

/// Which chunks `semantic_scores` must score exactly besides the ones that can enter the semantic list.
pub(super) struct SemanticNeed<'a> {
    /// Chunk positions whose reported semantic score is read (the lexical list's).
    pub(super) reported: &'a [usize],
    /// The semantic list's length and floor, or `None` when the semantic list is empty.
    pub(super) list: Option<(usize, f64)>,
    /// The chunks are already a screen's survivors, so none is screened again.
    pub(super) survivors: bool,
}

/// The semantic score of the chunks that can be read: its cached score, else the cosine of its stored embedding with
/// the query. A chunk not read is `None`. The chunks read are those of the `reported` positions and, when `list` is
/// set, every chunk that can be among the `list.0` best scores at or above `list.1`.
///
/// Every embedding is first screened with an `f32` kernel whose error against the exact score is bounded
/// ([`ScreenBound`]). A chunk whose screened upper bound is below both the floor and the screened lower bound of the
/// list's `limit`-th chunk cannot rank, so only the remaining chunks are scored exactly. Four are scored at once so
/// the four sequential `f64` reductions overlap in the pipeline; each chunk's own summation order is unchanged, so
/// every score is bit-identical to `Embedding::cosine`.
pub(super) fn semantic_scores(
    chunks: &[CapturedChunk],
    query: Option<&[f64]>,
    need: &SemanticNeed,
) -> Vec<Option<f64>> {
    let mut scores: Vec<Option<f64>> = chunks.iter().map(|chunk| chunk.semantic_cache).collect();
    let Some(query) = query else {
        return scores;
    };
    let rows: Vec<(usize, &Embedding)> = chunks
        .iter()
        .enumerate()
        .filter(|(i, _)| scores[*i].is_none())
        .filter_map(|(i, chunk)| Some((i, chunk.stored()?.embedding.as_ref()?)))
        .collect();
    let mut exact = vec![false; chunks.len()];
    match need.list {
        Some((limit, floor))
            if !need.survivors && rows.len() >= SCREEN_MIN_ROWS && limit < rows.len() =>
        {
            let query32: Vec<f32> = query.iter().map(|&q| q as f32).collect();
            let embeddings: Vec<&Embedding> = rows.iter().map(|row| row.1).collect();
            let mut screened = Vec::new();
            Embedding::screen(&embeddings, &query32, &mut screened);
            let bound = ScreenBound::new(query);
            let spans: Vec<(f64, f64)> = screened
                .iter()
                .map(|&(dot, norm)| {
                    let (dot, slack) = (f64::from(dot), bound.of(norm));
                    if dot.is_finite() && slack.is_finite() {
                        ((dot - slack).max(-1.0), (dot + slack).min(1.0))
                    } else {
                        (f64::NEG_INFINITY, f64::INFINITY)
                    }
                })
                .collect();
            // The list holds at least `limit` chunks scoring at least their lower bound, so a chunk whose upper
            // bound is below that bound's `limit`-th largest value, or below the floor, cannot be in it. Chunks
            // with a cached score count with their exact value.
            let mut lower: Vec<f64> = spans.iter().map(|span| span.0).collect();
            lower.extend(scores.iter().flatten().copied());
            let cut = if lower.len() > limit {
                let (_, nth, _) = lower.select_nth_unstable_by(limit - 1, |a, b| b.total_cmp(a));
                *nth
            } else {
                f64::NEG_INFINITY
            };
            let threshold = cut.max(floor);
            for (&(i, _), span) in rows.iter().zip(&spans) {
                exact[i] = span.1 >= threshold;
            }
        }
        Some(_) => {
            for &(i, _) in &rows {
                exact[i] = true;
            }
        }
        None => {}
    }
    for &i in need.reported {
        exact[i] = true;
    }
    let rows: Vec<(usize, &Embedding)> = rows.into_iter().filter(|&(i, _)| exact[i]).collect();
    let (groups, remainder) = rows.as_chunks::<4>();
    for group in groups {
        let batch = [group[0].1, group[1].1, group[2].1, group[3].1];
        for (&(i, _), score) in group.iter().zip(Embedding::cosine4(batch, query)) {
            scores[i] = Some(score);
        }
    }
    for &(i, embedding) in remainder {
        scores[i] = Some(embedding.cosine(query));
    }
    scores
}

pub(super) fn execute_chunks(captured: &CapturedQuery, chunks: &[CapturedChunk]) -> Vec<RankedHit> {
    if chunks.is_empty() {
        return Vec::new();
    }
    let prepared = &captured.prepared;
    let lexical_scores: Vec<f64> = match &captured.lexical_scores {
        Some(lexical) => lexical.clone(),
        None => score_chunks(captured, chunks),
    };
    let order = |a: usize, b: usize| chunks[a].order().cmp(&chunks[b].order());
    // Scores travel with their indices so the selection compares without indirection.
    let mut lexical: Vec<(f64, usize)> = if prepared.lexical_weight > 0.0 {
        lexical_scores
            .iter()
            .enumerate()
            .filter(|(_, &score)| score > 0.0)
            .map(|(i, &score)| (score, i))
            .collect()
    } else {
        Vec::new()
    };
    let by_score =
        |a: &(f64, usize), b: &(f64, usize)| b.0.total_cmp(&a.0).then_with(|| order(a.1, b.1));
    top_ranked(&mut lexical, prepared.limit, by_score);
    let reported: Vec<usize> = lexical.iter().map(|&(_, i)| i).collect();
    let semantic_scores = semantic_scores(
        chunks,
        prepared.embedding.as_deref(),
        &SemanticNeed {
            reported: &reported,
            list: (prepared.semantic_weight > 0.0).then_some((prepared.limit, prepared.minimum)),
            survivors: captured.screened.is_some(),
        },
    );
    let scores: Vec<(f64, Option<f64>)> = lexical_scores
        .iter()
        .zip(&semantic_scores)
        .map(|(&lexical, &semantic)| (lexical, semantic))
        .collect();
    let mut semantic: Vec<(f64, usize)> = if prepared.semantic_weight > 0.0 {
        scores
            .iter()
            .enumerate()
            .filter_map(|(i, score)| {
                score
                    .1
                    .filter(|&value| value >= prepared.minimum)
                    .map(|value| (value, i))
            })
            .collect()
    } else {
        Vec::new()
    };
    top_ranked(&mut semantic, prepared.limit, by_score);
    // Each list's contribution per chunk, lexical first; a stable sort by chunk then sums them
    // in that order, starting from 0.
    let mut contributions = Vec::with_capacity(lexical.len() + semantic.len());
    for (indices, weight) in [
        (&lexical, prepared.lexical_weight),
        (&semantic, prepared.semantic_weight),
    ] {
        if prepared.rrf {
            for (rank, &(_, i)) in indices.iter().enumerate() {
                contributions.push((i, weight / (60.0 + rank as f64 + 1.0)));
            }
            continue;
        }
        let (high, low) = match (indices.first(), indices.last()) {
            (Some(&(first, _)), Some(&(last, _))) => (first, last),
            _ => continue,
        };
        for &(value, i) in indices {
            let normalized = if high > low {
                (value - low) / (high - low)
            } else {
                1.0
            };
            contributions.push((i, weight * normalized));
        }
    }
    // With one list contributing, each chunk appears once and the final selection orders
    // them, so only a fusion of both lists needs grouping by chunk.
    if !lexical.is_empty() && !semantic.is_empty() {
        contributions.sort_by_key(|&(i, _)| i);
    }
    let mut ranked: Vec<(usize, f64)> = Vec::with_capacity(contributions.len());
    for (i, contribution) in contributions {
        match ranked.last_mut() {
            Some(fused) if fused.0 == i => fused.1 += contribution,
            _ => ranked.push((i, 0.0 + contribution)),
        }
    }
    top_ranked(&mut ranked, prepared.k, |a, b| {
        b.1.total_cmp(&a.1).then_with(|| order(a.0, b.0))
    });
    ranked
        .into_iter()
        .map(|(i, score)| {
            let (mut lexical_score, semantic_score) = scores[i];
            if lexical_score.is_nan() {
                if let Some(screened) = &captured.screened {
                    lexical_score = screened.score(prepared, &chunks[i]);
                }
            }
            match &chunks[i].owner {
                ChunkOwner::Live(document) => {
                    let chunk = &document.chunks[chunks[i].index];
                    RankedHit {
                        result: DocumentResult {
                            id: chunk.id.clone(),
                            document_id: document.id.clone(),
                            source_uri: document.source_uri.clone(),
                            version: document.version.clone(),
                            metadata: (*document.metadata).clone(),
                            text: None,
                            start_byte: chunk.start_byte,
                            end_byte: chunk.end_byte,
                            score,
                            lexical_score,
                            semantic_score,
                        },
                        source: HitSource::Live(Arc::clone(document), chunks[i].index),
                    }
                }
                // Only the capture's own chunks are checkpoint chunks (the dense path, which
                // extends them, never captures any), so `i` is a position in it.
                ChunkOwner::Image(_) => RankedHit {
                    result: DocumentResult {
                        score,
                        lexical_score,
                        semantic_score,
                        ..unread_result()
                    },
                    source: HitSource::Image(i),
                },
            }
        })
        .collect()
}

/// Captures without lexical scores hold only live chunks (see [`ChunkOwner::Image`]).
pub(super) const LIVE_ONLY: &str = "a capture scored here holds only live chunks";

/// Lexical (BM25) scores of captured `chunks` whose lexical scores the capture did not compute.
pub(super) fn score_chunks(captured: &CapturedQuery, chunks: &[CapturedChunk]) -> Vec<f64> {
    let prepared = &captured.prepared;
    let n = captured.count as f64;
    let average = (captured.words as f64 / n).max(1.0);
    let idf: [BTreeMap<&String, f64>; 2] = std::array::from_fn(|form| {
        prepared.query_terms[form]
            .iter()
            .map(|word| {
                let df = if let Some(dfs) = &captured.dfs {
                    dfs[form][word]
                } else if captured.use_postings {
                    captured.postings[form].get(word).map_or(0, Vec::len)
                } else {
                    chunks
                        .iter()
                        .filter(|chunk| {
                            chunk.stored().expect(LIVE_ONLY).stemmed == (form == 1)
                                && chunk.stored().expect(LIVE_ONLY).terms.contains_key(word)
                        })
                        .count()
                } as f64;
                (word, Bm25::idf(n, df))
            })
            .collect()
    });
    let mut norms = Vec::with_capacity(if captured.use_postings {
        chunks.len()
    } else {
        0
    });
    let mut scores: Vec<f64> = chunks
        .iter()
        .map(|captured_chunk| {
            let norm = prepared
                .bm25
                .norm(captured_chunk.word_count as f64, average);
            let lexical = if captured.use_postings {
                norms.push(norm);
                0.0
            } else {
                let chunk = captured_chunk.stored().expect(LIVE_ONLY);
                idf[usize::from(chunk.stemmed)]
                    .iter()
                    .map(|(word, idf)| {
                        let tf = f64::from(chunk.terms.get(word).copied().unwrap_or(0));
                        prepared.bm25.weight(*idf, tf, norm)
                    })
                    .sum()
            };
            lexical
        })
        .collect();
    if captured.use_postings {
        for (form, weights) in idf.iter().enumerate() {
            for (word, idf) in weights {
                if let Some(posting) = captured.postings[form].get(*word) {
                    for posting in posting {
                        let chunk = posting.chunk as usize;
                        let tf = f64::from(posting.frequency);
                        scores[chunk] += prepared.bm25.weight(*idf, tf, norms[chunk]);
                    }
                }
            }
        }
    }
    scores
}
