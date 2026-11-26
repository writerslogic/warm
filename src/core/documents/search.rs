// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::postings::{PostingList, Postings};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::core::hash::{FastMap, FastSet};
use anyhow::{ensure, Result};
use serde_json::Value;

use super::dense::DenseIndex;
use super::image::{MetadataCache, ServingImage};
use super::lexical::{LexicalHit, LexicalResults};
use super::overlay::Overlaid;
use super::screen::ScreenTable;
use super::{
    matches_document_metadata, normalize_embedding, query_words_both, terms_with, top_ranked,
    Analyzer, Bm25, DocumentResult, DocumentStore, Embedding, ScreenBound, SearchOptions,
    StoredChunk, StoredDocument, MAX_DOCUMENT_BYTES,
};

pub(crate) struct PreparedSearch {
    filters: SearchOptions,
    /// The query's distinct words per form; BM25 here ignores query term frequency.
    query_terms: [BTreeSet<String>; 2],
    embedding: Option<Vec<f64>>,
    k: usize,
    limit: usize,
    lexical_weight: f64,
    semantic_weight: f64,
    minimum: f64,
    rrf: bool,
    lexical_only: bool,
    unfiltered: bool,
    bm25: Bm25,
}

impl PreparedSearch {
    fn matches(&self, document: &StoredDocument) -> bool {
        self.filters
            .document_ids
            .as_ref()
            .is_none_or(|ids| ids.binary_search(&document.id).is_ok())
            && matches_document_metadata(document, &self.filters)
    }
}

pub(crate) struct CapturedSearch {
    queries: Vec<CapturedQuery>,
}

/// Where a captured chunk lives.
#[derive(Clone)]
enum ChunkOwner {
    /// A live store's document, holding the chunk at the capture's `index`.
    Live(Arc<StoredDocument>),
    /// A checkpoint chunk, decoded at capture straight into the unscored result it reports,
    /// so nothing reads the mapped checkpoint once the capture's locks are released (a purge
    /// compaction zeroes the file of the generation it retires). Only lexical-only queries
    /// without an embedding capture these ([`overlay_serves`]), and always with their lexical
    /// scores, so nothing reads such a chunk's terms or embedding.
    Image(Box<DocumentResult>),
}

#[derive(Clone)]
struct CapturedChunk {
    owner: ChunkOwner,
    index: usize,
    word_count: u32,
    semantic_cache: Option<f64>,
}

impl CapturedChunk {
    fn live(document: Arc<StoredDocument>, index: usize, word_count: u32) -> Self {
        Self {
            owner: ChunkOwner::Live(document),
            index,
            word_count,
            semantic_cache: None,
        }
    }

    /// The stored chunk, `None` for a checkpoint chunk.
    fn stored(&self) -> Option<&StoredChunk> {
        match &self.owner {
            ChunkOwner::Live(document) => Some(&document.chunks[self.index]),
            ChunkOwner::Image(..) => None,
        }
    }

    fn order(&self) -> (&str, usize) {
        match &self.owner {
            ChunkOwner::Live(document) => (&document.id, self.index),
            ChunkOwner::Image(hit) => (&hit.document_id, self.index),
        }
    }

    #[cfg(test)]
    fn document(&self) -> &Arc<StoredDocument> {
        match &self.owner {
            ChunkOwner::Live(document) => document,
            ChunkOwner::Image(..) => panic!("a checkpoint slot has no stored document"),
        }
    }
}

/// What scores a screened capture's pending lexical scores: BM25 inverse document frequencies per form and the
/// average chunk length of the captured store state.
struct ScreenedLexical {
    /// Present when no lexical list was ranked (zero lexical weight), so the scores of returned chunks are computed.
    pending: Option<PendingLexical>,
}

struct PendingLexical {
    idf: [BTreeMap<String, f64>; 2],
    average: f64,
}

impl ScreenedLexical {
    fn score(&self, prepared: &PreparedSearch, chunk: &CapturedChunk) -> f64 {
        let Some(pending) = &self.pending else {
            return 0.0;
        };
        let stored = chunk.stored().expect(LIVE_ONLY);
        let norm = prepared
            .bm25
            .norm(f64::from(chunk.word_count), pending.average);
        pending.idf[usize::from(stored.stemmed)]
            .iter()
            .map(|(word, idf)| {
                let tf = f64::from(stored.terms.get(word).copied().unwrap_or(0));
                prepared.bm25.weight(*idf, tf, norm)
            })
            .sum()
    }
}

struct CapturedPosting {
    chunk: u32,
    frequency: u32,
}

struct EligibleOwners(Vec<usize>);

impl EligibleOwners {
    fn new(mut owners: Vec<usize>) -> Self {
        owners.sort_unstable();
        owners.dedup();
        Self(owners)
    }

    fn contains(&self, owner: &usize) -> bool {
        self.0.binary_search(owner).is_ok()
    }
}

/// Slot to captured-chunk position: one `u32` per store slot while that table is small enough
/// to fill per query, a hash map beyond.
enum SlotPositions {
    Dense(Vec<u32>),
    Sparse(FastMap<usize, u32>),
}

pub(super) const DENSE_SLOT_LIMIT: usize = 1 << 19;
/// Below this many slots the exhaustive hot-list ranking beats the windowed pruned ranker.
/// Above `DENSE_SLOT_LIMIT` the pruned ranker is the only one that avoids hydrating the store.
const PRUNE_SLOT_FLOOR_SERVING: usize = 1 << 15;
const PRUNE_SLOT_FLOOR: usize = if cfg!(test) {
    0
} else {
    PRUNE_SLOT_FLOOR_SERVING
};
fn prune_above(slots: usize, floor: usize) -> bool {
    slots > floor
}
fn prune_wanted(slots: usize) -> bool {
    prune_above(slots, PRUNE_SLOT_FLOOR)
}
impl SlotPositions {
    fn new(slot_count: usize) -> Self {
        if slot_count <= DENSE_SLOT_LIMIT {
            Self::Dense(vec![u32::MAX; slot_count])
        } else {
            Self::Sparse(FastMap::default())
        }
    }

    fn get(&self, slot: usize) -> Option<u32> {
        match self {
            Self::Dense(table) => table.get(slot).copied().filter(|&p| p != u32::MAX),
            Self::Sparse(map) => map.get(&slot).copied(),
        }
    }

    fn insert(&mut self, slot: usize, position: u32) {
        match self {
            Self::Dense(table) => table[slot] = position,
            Self::Sparse(map) => {
                map.insert(slot, position);
            }
        }
    }
}

struct CapturedQuery {
    prepared: PreparedSearch,
    chunks: Vec<CapturedChunk>,
    postings: [BTreeMap<String, Vec<CapturedPosting>>; 2],
    count: usize,
    words: usize,
    use_postings: bool,
    dense: Option<Arc<DenseIndex>>,
    scan: Option<Vec<Arc<StoredDocument>>>,
    eligible: Option<EligibleOwners>,
    excluded_owners: FastSet<usize>,
    dfs: Option<[BTreeMap<String, usize>; 2]>,
    /// The store's text segment, read for result text after the store lock is released (under
    /// the mutation gate for a live search, see [`read_ranked`]).
    text: Option<Arc<super::text::TextSegment>>,
    /// Set when the capture already scored the query (an unfiltered lexical query): `chunks`
    /// are then its top `limit` lexical chunks, best first, and these their BM25 scores.
    lexical_scores: Option<Vec<f64>>,
    /// Set by [`capture_screened`]: the chunks are already the screen's survivors, and a lexical score that is NaN
    /// (the chunk is outside the lexical list) is computed only if the chunk is returned.
    screened: Option<Box<ScreenedLexical>>,
}

#[cfg(test)]
pub(crate) fn prepare_search(
    text: &str,
    options: &SearchOptions,
    embedding_dimensions: Option<usize>,
) -> Result<PreparedSearch> {
    prepare_search_with(text, options, embedding_dimensions, Analyzer::Legacy)
}

/// The query, validated, with its terms as a store with `analyzer` stores them.
pub(crate) fn prepare_search_with(
    text: &str,
    options: &SearchOptions,
    embedding_dimensions: Option<usize>,
    analyzer: Analyzer,
) -> Result<PreparedSearch> {
    let mut prepared = validate_search(text, options, embedding_dimensions)?;
    prepared.query_terms = query_words_both(analyzer, text);
    Ok(prepared)
}

/// A lexical-only search of unfiltered `terms` (one set per form), validated as `options` are.
pub(super) fn prepare_terms(
    terms: [BTreeSet<String>; 2],
    options: &SearchOptions,
) -> Result<PreparedSearch> {
    let mut prepared = validate_search("", options, None)?;
    prepared.query_terms = terms;
    Ok(prepared)
}

pub(crate) fn validate_search(
    text: &str,
    options: &SearchOptions,
    embedding_dimensions: Option<usize>,
) -> Result<PreparedSearch> {
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
    let total_weight = lexical_weight + semantic_weight;
    ensure!(
        [lexical_weight, semantic_weight]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
            && total_weight.is_finite()
            && total_weight > 0.0,
        "search weights must be finite, nonnegative and not both zero"
    );
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
    let embedding = options
        .embedding
        .as_ref()
        .map(|values| {
            ensure!(
                embedding_dimensions.is_some(),
                "configure an embedding space before semantic search"
            );
            normalize_embedding(values, embedding_dimensions.unwrap_or(0))
        })
        .transpose()?;
    let unfiltered = options.source_uri.is_none()
        && options.document_ids.is_none()
        && options
            .filter
            .as_ref()
            .and_then(Value::as_object)
            .is_none_or(|filter| filter.is_empty());
    let lexical_only = unfiltered && (embedding.is_none() || semantic_weight == 0.0);
    let bm25 = Bm25::from_options(options)?;
    let document_ids = options.document_ids.as_ref().map(|ids| {
        let mut ids = ids.clone();
        ids.sort_unstable();
        ids.dedup();
        ids
    });
    Ok(PreparedSearch {
        filters: SearchOptions {
            filter: options.filter.clone(),
            source_uri: options.source_uri.clone(),
            document_ids,
            ..Default::default()
        },
        query_terms: Default::default(),
        embedding,
        k,
        limit,
        lexical_weight,
        semantic_weight,
        minimum,
        rrf,
        lexical_only,
        unfiltered,
        bm25,
    })
}

/// The number of documents with at least one chunk containing a term of `text` at a positive
/// frequency, in either stored form: the documents an unfiltered lexical search scores above
/// zero.
pub(crate) fn count_matches(
    store: &DocumentStore,
    text: &str,
    analyzer: Analyzer,
) -> Result<usize> {
    ensure!(text.len() <= MAX_DOCUMENT_BYTES, "query exceeds 8 MiB");
    let terms = [
        terms_with(analyzer, text, false),
        terms_with(analyzer, text, true),
    ];
    Ok(super::lexical::execution::count_terms(
        store,
        std::array::from_fn(|form| terms[form].keys().map(String::as_str).collect()),
    ))
}

mod capture;
mod execute;
#[cfg(test)]
mod latency_bench;
mod rank;
mod source;
#[cfg(test)]
mod tests;

pub(crate) use self::capture::capture_search;
#[cfg(test)]
use self::capture::*;
pub(super) use self::capture::{
    capture_overlaid_search, overlay_serves, rank_overlaid, rank_store,
};
use self::execute::*;
pub(crate) use self::execute::{execute_search, rank_search, read_ranked};
use self::rank::*;
use self::source::*;
pub(super) use self::source::{LexicalSource, PostingSet};
