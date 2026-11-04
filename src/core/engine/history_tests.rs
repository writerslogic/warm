// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The view builder against the frame-at-a-time replay it replaced, kept here as the oracle:
//! every frame applied to a document store in turn, consecutive document writes installed
//! together.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use super::*;
use crate::core::documents::{EncodeMode, IngestOptions};
use crate::core::WarmConfig;
use crate::{DocumentInput, EmbeddingSpace};

/// The replaced applier, unchanged.
struct ReplayState {
    documents: DocumentStore,
    pending: Vec<Arc<StoredDocument>>,
    pending_ids: HashSet<String>,
    sizes: HashMap<String, usize>,
    vectors: BTreeMap<String, EntangledHVec>,
    derived: BTreeMap<String, Vec<String>>,
    bytes: usize,
    limit: usize,
}

impl ReplayState {
    fn new(limit: usize) -> Self {
        Self {
            documents: DocumentStore::default(),
            pending: Vec::new(),
            pending_ids: HashSet::new(),
            sizes: HashMap::new(),
            vectors: BTreeMap::new(),
            derived: BTreeMap::new(),
            bytes: 0,
            limit,
        }
    }

    fn grow(&mut self, add: usize, remove: usize) -> Result<()> {
        grow(&mut self.bytes, self.limit, add, remove)
    }

    fn remove_vector(&mut self, id: &str) -> Result<()> {
        if let Some(vector) = self.vectors.remove(id) {
            self.grow(0, vector_bytes(id, &vector))?;
        }
        Ok(())
    }

    fn remove_derived(&mut self, id: &str) -> Result<()> {
        for child in self.derived.remove(id).into_iter().flatten() {
            self.remove_vector(&child)?;
        }
        Ok(())
    }

    fn put_document(&mut self, document: Arc<StoredDocument>) -> Result<()> {
        let size = document_bytes(&document);
        let old = self.sizes.insert(document.id.clone(), size).unwrap_or(0);
        self.grow(size, old)?;
        if !self.pending_ids.insert(document.id.clone()) {
            self.flush()?;
            self.pending_ids.insert(document.id.clone());
        }
        self.pending.push(document);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.pending_ids.clear();
        if self.pending.is_empty() {
            return Ok(());
        }
        self.documents
            .insert_batch(std::mem::take(&mut self.pending))
    }

    fn apply(&mut self, mutation: &Mutation) -> Result<()> {
        if !matches!(mutation, Mutation::Document(_)) {
            self.flush()?;
        }
        match mutation {
            Mutation::Document(document) => self.put_document(Arc::clone(document))?,
            Mutation::DeleteDocument { id } => {
                self.documents.remove(id);
                let old = self.sizes.remove(id).unwrap_or(0);
                self.grow(0, old)?;
            }
            Mutation::EncodeDocument {
                id,
                version,
                fingerprint,
                vectors,
            } => {
                let Some(current) = self.documents.get(id).cloned() else {
                    return Ok(());
                };
                let applicable = current.version == *version
                    && current.chunks.len() == vectors.len()
                    && current
                        .chunks
                        .iter()
                        .any(|chunk| chunk.vector_state == VectorState::Pending)
                    && documents::document_fingerprint(&self.documents, &current)?
                        == Some(*fingerprint);
                if applicable {
                    let mut encoded = StoredDocument::clone(&current);
                    for (chunk, vector) in encoded.chunks.iter_mut().zip(vectors) {
                        if chunk.vector_state == VectorState::Pending {
                            chunk.vector = vector.clone();
                            chunk.vector_state = VectorState::Encoded;
                        }
                    }
                    self.put_document(Arc::new(encoded))?;
                }
            }
            Mutation::Vector { id, vector } => {
                self.remove_derived(id)?;
                self.remove_vector(id)?;
                self.grow(vector_bytes(id, vector), 0)?;
                self.vectors.insert(id.clone(), vector.clone());
            }
            Mutation::Delete { id } => {
                self.remove_derived(id)?;
                self.remove_vector(id)?;
            }
            Mutation::Derived { id, children } => {
                self.derived.insert(id.clone(), children.clone());
            }
            _ => {}
        }
        Ok(())
    }
}

/// The view the replaced builder produced at `at`.
fn oracle_view<'a>(core: &'a WarmCore, at: &JournalPosition) -> Result<HistoryView<'a>> {
    let pinned = core.pin_history()?;
    let target = pinned.check(at)?;
    let mut state = ReplayState::new(DEFAULT_MAX_STATE_BYTES);
    let mut offset = 0;
    while offset < target {
        let frame = core.history_frame(offset)?;
        for mutation in &frame.mutations {
            state.apply(mutation)?;
        }
        state.flush()?;
        offset = frame.next;
    }
    anyhow::ensure!(offset == target, "not a frame boundary");
    Ok(HistoryView {
        core,
        position: at.clone(),
        state: Arc::new(ViewState {
            documents: state.documents,
            vectors: state.vectors,
        }),
    })
}

const QUERIES: [&str; 6] = [
    "ownership threads",
    "alpha bravo",
    "charlie delta echo",
    "revised",
    "frost",
    "golf hotel india juliet",
];
const WORDS: [&str; 12] = [
    "alpha",
    "bravo",
    "charlie",
    "delta",
    "echo",
    "frost",
    "golf",
    "hotel",
    "india",
    "juliet",
    "ownership",
    "threads",
];

/// Everything a view answers, plus every stored document as serialized, for exact comparison.
fn observe(view: &HistoryView<'_>) -> Value {
    let mut searches = Vec::new();
    for query in QUERIES {
        for options in [
            SearchOptions::default(),
            SearchOptions {
                k: Some(50),
                ..Default::default()
            },
            SearchOptions {
                embedding: Some(vec![0.7, 0.2, 0.3]),
                ..Default::default()
            },
        ] {
            searches.push(
                serde_json::to_value(view.search_documents(query, &options).unwrap()).unwrap(),
            );
        }
    }
    let vectors: Vec<(String, Vec<u32>)> = view
        .vector_ids()
        .unwrap()
        .into_iter()
        .map(|id| {
            let vector = view.get_vector(&id).unwrap().unwrap();
            (id, vector.indices().to_vec())
        })
        .collect();
    let stored: Vec<Value> = view
        .state
        .documents
        .shared_values()
        .map(|document| serde_json::to_value(&*document).unwrap())
        .collect();
    json!({
        "count": view.document_count().unwrap(),
        "listing": serde_json::to_value(view.list_documents(None, 10_000).unwrap()).unwrap(),
        "searches": searches,
        "vectors": serde_json::to_value(vectors).unwrap(),
        "stored": stored,
    })
}

/// A small deterministic generator, so a failure names a reproducible seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn text(rng: &mut Rng, words: usize) -> String {
    (0..words)
        .map(|_| WORDS[rng.below(WORDS.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

/// One random write: a batch, an insert, an update keeping or changing the chunk count, a
/// delete (of a stored or a missing id), a deferred encoding, or a raw vector write or delete.
/// Returns what kind it was.
fn step(core: &WarmCore, rng: &mut Rng, revision: &mut usize) -> Result<&'static str> {
    *revision += 1;
    let id = format!("d{}", rng.below(24));
    let kind = rng.below(10);
    let options = IngestOptions {
        unstemmed: rng.below(4) == 0,
        positions: rng.below(3) == 0,
        ..Default::default()
    };
    let single = |rng: &mut Rng, id: &str, chunks: u32| -> Result<DocumentInput> {
        let words = 6 * chunks as usize;
        let mut value = json!({
            "id": id,
            "text": text(rng, words),
            "version": revision.to_string(),
            "chunkWords": 6,
            "overlapWords": 0,
        });
        if chunks == 1 && rng.below(2) == 0 {
            value["embeddings"] = json!([[rng.below(10) as f64 / 10.0, 0.5, 0.25]]);
        }
        Ok(serde_json::from_value(value)?)
    };
    Ok(match kind {
        0 | 1 => {
            let ids: std::collections::BTreeSet<String> = (0..1 + rng.below(4))
                .map(|_| format!("d{}", rng.below(24)))
                .collect();
            let inputs = ids
                .iter()
                .map(|id| single(rng, id, 1))
                .collect::<Result<Vec<_>>>()?;
            core.memorize_documents_with(inputs, &options)?;
            "batch"
        }
        2 | 3 => {
            core.memorize_document_with(single(rng, &id, 1)?, &options)?;
            "write"
        }
        4 => {
            let chunks = 1 + rng.below(3) as u32;
            core.memorize_document_with(single(rng, &id, chunks)?, &options)?;
            "write chunks"
        }
        5 | 6 => {
            core.delete_document(&id)?;
            "delete"
        }
        7 => {
            core.memorize_document_with(
                single(rng, &id, 1)?,
                &IngestOptions {
                    encode: EncodeMode::Deferred,
                    ..options
                },
            )?;
            if rng.below(2) == 0 {
                core.encode_pending_documents()?;
            }
            "deferred"
        }
        8 => {
            let seed = rng.next();
            core.memorize(
                format!("v{}", rng.below(4)),
                EntangledHVec::new_deterministic(256, seed),
            )?;
            "vector"
        }
        _ => {
            core.delete(&format!("v{}", rng.below(4)))?;
            "vector delete"
        }
    })
}

fn config() -> WarmConfig {
    let mut config = WarmConfig {
        embedding_space: Some(EmbeddingSpace {
            model: "fixture".into(),
            revision: "1".into(),
            dimensions: 3,
            normalization: "l2".into(),
            metric: "cosine".into(),
        }),
        ..Default::default()
    };
    config.document_checkpoint.enabled = false;
    config
}

/// The budget the reuse tests opt in with.
const OPT_IN_BUDGET: usize = 256 << 20;

fn assert_matches_oracle(core: &WarmCore, at: &JournalPosition, when: &str) {
    let oracle = observe(&oracle_view(core, at).unwrap());
    core.clear_history_cache();
    let built = core.history_view(at).unwrap();
    assert_eq!(observe(&built), oracle, "{when} at {at}, built");
    let reused = core.history_view(at).unwrap();
    assert!(
        Arc::ptr_eq(&built.state, &reused.state),
        "{when} at {at}: reused"
    );
    assert_eq!(observe(&reused), oracle, "{when} at {at}, reused");
}

/// At every position of a random journal, before and after a compaction publishes a new
/// generation, the bulk-indexed view answers and stores exactly what frame-at-a-time replay
/// built: counts, listing, searches (lexical, deeper k, embedding), raw vectors and every
/// stored document.
#[test]
fn views_match_frame_at_a_time_replay_at_every_position() {
    for seed in [1_u64, 7, 42] {
        let dir = tempfile::tempdir().unwrap();
        let core =
            WarmCore::new(256, Some(dir.path().display().to_string()), Some(config())).unwrap();
        core.set_history_cache_budget(OPT_IN_BUDGET);
        let mut rng = Rng(seed);
        let mut revision = 0;
        let mut positions = vec![(core.journal_position(), "start")];
        for _ in 0..60 {
            let kind = step(&core, &mut rng, &mut revision).unwrap();
            let position = core.journal_position();
            if positions.last().is_some_and(|(p, _)| *p != position) {
                positions.push((position, kind));
            }
        }
        for (i, (at, kind)) in positions.iter().enumerate() {
            assert_matches_oracle(&core, at, &format!("seed {seed}, step {i} after {kind}"));
        }
        assert!(!core.history_cache.lock().entries.is_empty());
        core.compact().unwrap();
        assert!(
            core.history_cache.lock().entries.is_empty(),
            "a compaction empties the view cache"
        );
        let floor = core.history_bounds().unwrap().oldest;
        assert_ne!(floor.generation, positions[0].0.generation);
        assert_matches_oracle(&core, &floor, &format!("seed {seed}, compaction publish"));
        let mut after = vec![floor];
        for _ in 0..20 {
            let kind = step(&core, &mut rng, &mut revision).unwrap();
            let position = core.journal_position();
            assert_matches_oracle(
                &core,
                &position,
                &format!("seed {seed}, after compaction, {kind}"),
            );
            after.push(position);
        }
        let latest = core.journal_position();
        for at in &after {
            assert_matches_oracle(&core, at, &format!("seed {seed}, revisit"));
        }
        assert_eq!(
            serde_json::to_value(
                core.history_view(&latest)
                    .unwrap()
                    .list_documents(None, 10_000)
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(core.list_documents(None, 10_000).unwrap()).unwrap(),
            "seed {seed}: the latest view lists the live store"
        );
    }
}

/// The state bound trips at the same positions as frame-at-a-time replay, for views built
/// and for views reused from one built under a larger bound.
#[test]
fn the_state_bound_trips_where_replay_tripped() {
    let dir = tempfile::tempdir().unwrap();
    let core = WarmCore::new(256, Some(dir.path().display().to_string()), Some(config())).unwrap();
    core.set_history_cache_budget(OPT_IN_BUDGET);
    let mut rng = Rng(3);
    let mut revision = 0;
    let mut positions = Vec::new();
    for _ in 0..30 {
        step(&core, &mut rng, &mut revision).unwrap();
        positions.push(core.journal_position());
    }
    let open = |at: &JournalPosition, limit: usize| {
        core.history_view_with(
            at,
            &HistoryOptions {
                max_state_bytes: limit,
            },
        )
        .err()
        .map(|e| e.downcast::<HistoryError>().expect("a history error"))
    };
    let mut outcomes = HashSet::new();
    for at in &positions {
        for limit in [2_000, 8_000, 20_000] {
            let pinned = core.pin_history().unwrap();
            let target = pinned.check(at).unwrap();
            let mut state = ReplayState::new(limit);
            let mut offset = 0;
            let mut replayed = None;
            while offset < target {
                let frame = core.history_frame(offset).unwrap();
                if let Err(error) = frame
                    .mutations
                    .iter()
                    .try_for_each(|m| state.apply(m))
                    .and_then(|()| state.flush())
                {
                    replayed = Some(error.downcast::<HistoryError>().unwrap());
                    break;
                }
                offset = frame.next;
            }
            drop(pinned);
            core.clear_history_cache();
            let built = open(at, limit);
            assert_eq!(built, replayed, "limit {limit} at {at}, built");
            assert!(open(at, DEFAULT_MAX_STATE_BYTES).is_none());
            let reused = open(at, limit);
            assert_eq!(reused, replayed, "limit {limit} at {at}, reused");
            outcomes.insert(replayed.is_some());
        }
    }
    assert_eq!(outcomes.len(), 2, "the limits both trip and pass");
}

/// By default no view outlives its caller: nothing is kept, a reopen builds a new state, and
/// a budget set to 0 again releases what an opt-in kept.
#[test]
fn views_are_not_retained_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let core = WarmCore::new(256, Some(dir.path().display().to_string()), Some(config())).unwrap();
    let mut rng = Rng(11);
    let mut revision = 0;
    for _ in 0..10 {
        step(&core, &mut rng, &mut revision).unwrap();
    }
    let at = core.journal_position();
    let first = core.history_view(&at).unwrap();
    let observed = observe(&first);
    let state = Arc::downgrade(&first.state);
    drop(first);
    assert!(core.history_cache.lock().entries.is_empty());
    assert!(
        state.upgrade().is_none(),
        "the dropped view's state is freed"
    );
    assert_eq!(observe(&core.history_view(&at).unwrap()), observed);
    assert!(core.history_cache.lock().entries.is_empty());

    core.set_history_cache_budget(OPT_IN_BUDGET);
    drop(core.history_view(&at).unwrap());
    assert_eq!(core.history_cache.lock().entries.len(), 1);
    core.set_history_cache_budget(0);
    assert!(core.history_cache.lock().entries.is_empty());
    drop(core.history_view(&at).unwrap());
    assert!(core.history_cache.lock().entries.is_empty());
}
