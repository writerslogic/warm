// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Read-only historical views over the journal (docs/memory-api/TIME-TRAVEL.md).
//!
//! A view replays the live generation's frames from offset 0 into a private in-memory
//! document store and vector map. Nothing here writes the live store: frames are read with
//! `read_frame`, decoded and validated as replay does, and applied by this module's own small
//! applier. Building and every read hold the compaction lock and check that the generation
//! is still live, so a compaction (a purge included) can neither swap frames under a build
//! nor let an earlier view keep serving what it removed.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::mutation::{self, Mutation};
use super::{journal, WarmCore};
use crate::core::documents::{self, DocumentStore, StoredDocument, VectorState};
use crate::core::entangled::EntangledHVec;
use crate::core::memory_api::Millis;
use crate::{DocumentRecord, DocumentResult, SearchOptions};

/// Default bound on the estimated decoded state of one view.
pub const DEFAULT_MAX_STATE_BYTES: usize = 1 << 30;
/// Upper bound on `history_revisions`' `limit`.
pub const MAX_REVISIONS: usize = 100_000;

/// A frame boundary in one journal generation: the state after every frame that starts
/// before `offset`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalPosition {
    pub generation: String,
    pub offset: u64,
}

impl fmt::Display for JournalPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.generation, self.offset)
    }
}

/// Why a history call refused to answer. Malformed journal data is reported as the replay
/// error it is, not as one of these.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error("journal position {requested} names a generation that is not retained; history covers {oldest}..={latest}")]
    GenerationNotRetained {
        requested: JournalPosition,
        oldest: JournalPosition,
        latest: JournalPosition,
    },
    #[error("journal position {requested} is older than the oldest retained position {oldest}")]
    BeforeRetained {
        requested: JournalPosition,
        oldest: JournalPosition,
    },
    #[error("journal position {requested} is past the end of the journal {latest}")]
    PastEnd {
        requested: JournalPosition,
        latest: JournalPosition,
    },
    #[error("journal position {requested} is not a frame boundary")]
    NotFrameBoundary { requested: JournalPosition },
    #[error("no journal frame is stamped at or before {time_ms} ms; query by position")]
    NoTimeAnchor { time_ms: Millis },
    #[error("this generation begins with a document checkpoint; its history cannot be replayed")]
    CheckpointBase,
    #[error("the historical state exceeds the {limit}-byte limit")]
    StateTooLarge { limit: usize },
    #[error("the store compacted since this view was built; open a new view")]
    Compacted,
    #[error("interval start {from} is after its end {to}")]
    IntervalInverted {
        from: JournalPosition,
        to: JournalPosition,
    },
    #[error("more than {limit} revisions are live in the interval")]
    TooManyRevisions { limit: usize },
}

/// Bounds on building a view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryOptions {
    /// Bound on the estimated bytes of decoded document and vector state.
    pub max_state_bytes: usize,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            max_state_bytes: DEFAULT_MAX_STATE_BYTES,
        }
    }
}

/// The retained history of the live generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryBounds {
    pub oldest: JournalPosition,
    pub latest: JournalPosition,
}

/// What a wall-clock time resolved to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeResolution {
    pub position: JournalPosition,
    /// Effective time of the anchor frame; `None` for the empty start of a never-compacted
    /// journal.
    pub anchor_ms: Option<Millis>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RevisionKind {
    Document,
    Vector,
}

/// One write of an id, live at some position of an interval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRevision {
    pub kind: RevisionKind,
    pub id: String,
    /// The document version; `None` for vectors.
    pub version: Option<String>,
    /// The position after the frame that wrote it; `None` when older than the oldest
    /// retained position.
    pub written: Option<JournalPosition>,
    /// The position after the frame that replaced or deleted it, when within the interval.
    pub ended: Option<JournalPosition>,
    pub deleted: bool,
}

/// A frame's offsets and decoded, validated mutations.
struct Frame {
    next: usize,
    mutations: Vec<Mutation>,
}

/// The compaction lock and the generation it was checked against.
struct Pinned<'a> {
    _compaction: crate::core::sync::MutexGuard<'a, ()>,
    generation: String,
    end: usize,
    floor: usize,
}

impl WarmCore {
    /// The current end of the journal: a view there is the current state.
    pub fn journal_position(&self) -> JournalPosition {
        let _transaction = self.mutation_gate.read();
        let (generation, offset) = self.arena.checkpoint();
        JournalPosition {
            generation,
            offset: offset as u64,
        }
    }

    /// The oldest and latest positions a view can be opened at.
    pub fn history_bounds(&self) -> Result<HistoryBounds> {
        let pinned = self.pin_history()?;
        Ok(HistoryBounds {
            oldest: pinned.position(pinned.floor),
            latest: pinned.position(pinned.end),
        })
    }

    /// The state at `at`, with the default bounds.
    pub fn history_view(&self, at: &JournalPosition) -> Result<HistoryView<'_>> {
        self.history_view_with(at, &HistoryOptions::default())
    }

    /// The state at `at`. Fails closed: a position that is not retained, not a frame boundary
    /// or past the end, and any frame that does not decode or validate, is an error.
    pub fn history_view_with(
        &self,
        at: &JournalPosition,
        options: &HistoryOptions,
    ) -> Result<HistoryView<'_>> {
        let pinned = self.pin_history()?;
        let target = pinned.check(at)?;
        let mut state = State::new(options.max_state_bytes);
        let mut offset = 0;
        while offset < target {
            let frame = self.history_frame(offset)?;
            for mutation in &frame.mutations {
                state.apply(mutation)?;
            }
            offset = frame.next;
        }
        if offset != target {
            return Err(HistoryError::NotFrameBoundary {
                requested: at.clone(),
            }
            .into());
        }
        Ok(HistoryView {
            core: self,
            position: at.clone(),
            documents: state.documents,
            vectors: state.vectors,
        })
    }

    /// The position a wall-clock time (memory-API ingest milliseconds) maps to.
    pub fn resolve_history_time(&self, time_ms: Millis) -> Result<TimeResolution> {
        let pinned = self.pin_history()?;
        self.resolve_pinned(&pinned, time_ms)
    }

    /// The state as of a wall-clock time; see [`Self::resolve_history_time`].
    pub fn history_view_at_time(
        &self,
        time_ms: Millis,
        options: &HistoryOptions,
    ) -> Result<HistoryView<'_>> {
        let position = self.resolve_history_time(time_ms)?.position;
        self.history_view_with(&position, options)
    }

    /// Every document and raw-vector revision live at some position in `[from, to]`, ordered
    /// by id and then by when it was written. More than `limit` is an error, never a
    /// truncated list.
    pub fn history_revisions(
        &self,
        from: &JournalPosition,
        to: &JournalPosition,
        limit: usize,
    ) -> Result<Vec<HistoryRevision>> {
        anyhow::ensure!(
            (1..=MAX_REVISIONS).contains(&limit),
            "revision limit must be in 1..={MAX_REVISIONS}"
        );
        let pinned = self.pin_history()?;
        let start = pinned.check(from)?;
        let end = pinned.check(to)?;
        if start > end {
            return Err(HistoryError::IntervalInverted {
                from: from.clone(),
                to: to.clone(),
            }
            .into());
        }
        // Live revisions: (kind, id) -> (version, written offset, live at a boundary >= start).
        let mut live: HashMap<(RevisionKind, String), (Option<String>, usize, bool)> =
            HashMap::new();
        let mut derived: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut out = Vec::new();
        let mut offset = 0;
        let mut seen_start = start == 0;
        while offset < end {
            let frame = self.history_frame(offset)?;
            let next = frame.next;
            let ended = |key: (RevisionKind, String),
                             deleted: bool,
                             live: &mut HashMap<_, (Option<String>, usize, bool)>,
                             out: &mut Vec<HistoryRevision>|
             -> Result<()> {
                if let Some((version, written, in_interval)) = live.remove(&key) {
                    if in_interval {
                        anyhow::ensure!(
                            out.len() < limit,
                            HistoryError::TooManyRevisions { limit }
                        );
                        out.push(HistoryRevision {
                            kind: key.0,
                            id: key.1,
                            version,
                            written: (written > pinned.floor).then(|| pinned.position(written)),
                            ended: Some(pinned.position(next)),
                            deleted,
                        });
                    }
                }
                Ok(())
            };
            for mutation in &frame.mutations {
                match mutation {
                    Mutation::Document(document) => {
                        let key = (RevisionKind::Document, document.id.clone());
                        ended(key.clone(), false, &mut live, &mut out)?;
                        live.insert(key, (Some(document.version.clone()), next, false));
                    }
                    Mutation::DeleteDocument { id } => {
                        ended(
                            (RevisionKind::Document, id.clone()),
                            true,
                            &mut live,
                            &mut out,
                        )?;
                    }
                    Mutation::Vector { id, .. } => {
                        for child in derived.remove(id).into_iter().flatten() {
                            ended((RevisionKind::Vector, child), true, &mut live, &mut out)?;
                        }
                        let key = (RevisionKind::Vector, id.clone());
                        ended(key.clone(), false, &mut live, &mut out)?;
                        live.insert(key, (None, next, false));
                    }
                    Mutation::Delete { id } => {
                        for child in derived.remove(id).into_iter().flatten() {
                            ended((RevisionKind::Vector, child), true, &mut live, &mut out)?;
                        }
                        ended(
                            (RevisionKind::Vector, id.clone()),
                            true,
                            &mut live,
                            &mut out,
                        )?;
                    }
                    Mutation::Derived { id, children } => {
                        derived.insert(id.clone(), children.clone());
                    }
                    _ => {}
                }
            }
            offset = next;
            if offset == start {
                seen_start = true;
            }
            if offset >= start {
                // Every revision still live here is live at a boundary in [start, end].
                for value in live.values_mut() {
                    value.2 = true;
                }
                anyhow::ensure!(
                    out.len() + live.len() <= limit,
                    HistoryError::TooManyRevisions { limit }
                );
            }
        }
        if !seen_start || offset != end {
            return Err(HistoryError::NotFrameBoundary {
                requested: if seen_start { to.clone() } else { from.clone() },
            }
            .into());
        }
        for ((kind, id), (version, written, in_interval)) in live {
            if in_interval {
                out.push(HistoryRevision {
                    kind,
                    id,
                    version,
                    written: (written > pinned.floor).then(|| pinned.position(written)),
                    ended: None,
                    deleted: false,
                });
            }
        }
        anyhow::ensure!(out.len() <= limit, HistoryError::TooManyRevisions { limit });
        out.sort_by(|a, b| {
            (a.kind as u8, &a.id, a.written.as_ref().map(|p| p.offset)).cmp(&(
                b.kind as u8,
                &b.id,
                b.written.as_ref().map(|p| p.offset),
            ))
        });
        Ok(out)
    }

    /// Takes the compaction lock (no generation swap until it drops) and reads the live
    /// generation's end under the mutation gate.
    fn pin_history(&self) -> Result<Pinned<'_>> {
        let compaction = self.compaction.lock();
        let (generation, end) = {
            let _transaction = self.mutation_gate.read();
            self.arena.checkpoint()
        };
        if self.arena.data_dir().join(super::image::FILE).exists() {
            return Err(HistoryError::CheckpointBase.into());
        }
        let floor = {
            let floor = self.history_floor.read();
            anyhow::ensure!(
                floor.0 == generation,
                "history floor names another generation"
            );
            floor.1
        };
        Ok(Pinned {
            _compaction: compaction,
            generation,
            end,
            floor,
        })
    }

    /// Where history starts in the generation just opened or published: offset 0 for a
    /// journal that was never compacted, else its current end (see TIME-TRAVEL.md).
    pub(super) fn initial_history_floor(&self) -> (String, usize) {
        let (generation, end) = self.arena.checkpoint();
        if generation.starts_with("gen-") {
            (generation, end)
        } else {
            (generation, 0)
        }
    }

    fn resolve_pinned(&self, pinned: &Pinned<'_>, time_ms: Millis) -> Result<TimeResolution> {
        let compacted = pinned.generation.starts_with("gen-");
        let mut anchor: Option<(usize, Option<Millis>)> = (!compacted).then_some((0, None));
        let mut high_water: Option<Millis> = None;
        let mut offset = 0;
        while offset < pinned.end {
            let frame = self.history_frame(offset)?;
            let stamp = frame
                .mutations
                .iter()
                .filter_map(|mutation| match mutation {
                    Mutation::Document(document) => {
                        crate::core::memory_api::ingest_time(&document.version, &document.metadata)
                    }
                    _ => None,
                })
                .max();
            if let Some(stamp) = stamp {
                let effective = high_water.map_or(stamp, |h| h.max(stamp));
                high_water = Some(effective);
                if effective > time_ms {
                    // Effective times never decrease: no later anchor can qualify.
                    break;
                }
                anchor = Some((frame.next, Some(effective)));
            }
            offset = frame.next;
        }
        let Some((position, anchor_ms)) = anchor else {
            return Err(HistoryError::NoTimeAnchor { time_ms }.into());
        };
        if position < pinned.floor {
            return Err(HistoryError::BeforeRetained {
                requested: pinned.position(position),
                oldest: pinned.position(pinned.floor),
            }
            .into());
        }
        Ok(TimeResolution {
            position: pinned.position(position),
            anchor_ms,
        })
    }

    /// Reads, decrypts, decodes and validates the frame at `offset`, as replay does. The
    /// caller holds the compaction lock.
    fn history_frame(&self, offset: usize) -> Result<Frame> {
        let (data, _) = self
            .arena
            .read_frame(offset)
            .with_context(|| format!("cannot read journal frame at {offset}"))?;
        let payload = self
            .maybe_decrypt(data)
            .with_context(|| format!("cannot decrypt journal frame at {offset}"))?;
        let mutations: Vec<Mutation> =
            if let Some(json) = payload.strip_prefix(mutation::TRANSACTION_MAGIC) {
                serde_json::from_slice(json)
                    .with_context(|| format!("cannot decode journal frame at {offset}"))?
            } else if let Some(body) = payload.strip_prefix(journal::BINARY_MAGIC) {
                let mut decoder = journal::BinaryDecoder::new(body)
                    .with_context(|| format!("cannot decode journal frame at {offset}"))?;
                let mut mutations = Vec::new();
                while let Some(mutation) = decoder
                    .next_mutation()
                    .with_context(|| format!("cannot decode journal frame at {offset}"))?
                {
                    mutations.push(mutation);
                }
                mutations
            } else {
                anyhow::bail!("journal frame at {offset} has an unsupported transaction format");
            };
        for mutation in &mutations {
            self.validate_mutation(mutation)
                .with_context(|| format!("invalid mutation in journal frame at {offset}"))?;
        }
        let next = self.arena.next_offset(offset)?;
        anyhow::ensure!(next > offset, "journal frame at {offset} does not advance");
        Ok(Frame { next, mutations })
    }
}

impl Pinned<'_> {
    fn position(&self, offset: usize) -> JournalPosition {
        JournalPosition {
            generation: self.generation.clone(),
            offset: offset as u64,
        }
    }

    /// The offset of `at` if it is in the retained range of the live generation.
    fn check(&self, at: &JournalPosition) -> Result<usize> {
        if at.generation != self.generation {
            return Err(HistoryError::GenerationNotRetained {
                requested: at.clone(),
                oldest: self.position(self.floor),
                latest: self.position(self.end),
            }
            .into());
        }
        let offset = usize::try_from(at.offset).unwrap_or(usize::MAX);
        if offset > self.end {
            return Err(HistoryError::PastEnd {
                requested: at.clone(),
                latest: self.position(self.end),
            }
            .into());
        }
        if offset < self.floor {
            return Err(HistoryError::BeforeRetained {
                requested: at.clone(),
                oldest: self.position(self.floor),
            }
            .into());
        }
        Ok(offset)
    }
}

/// The document and raw-vector state being replayed.
struct State {
    documents: DocumentStore,
    sizes: HashMap<String, usize>,
    vectors: BTreeMap<String, EntangledHVec>,
    derived: BTreeMap<String, Vec<String>>,
    bytes: usize,
    limit: usize,
}

fn document_bytes(document: &StoredDocument) -> usize {
    let chunks: usize = document
        .chunks
        .iter()
        .map(|chunk| {
            128 + chunk.id.len()
                + chunk.text.as_ref().map_or(0, String::len)
                + chunk.embedding.as_ref().map_or(0, |e| 4 * e.len())
                + 4 * chunk.vector.indices.len()
                + 24 * (&chunk.terms).into_iter().count()
        })
        .sum();
    256 + document.id.len() + document.version.len() + chunks
}

fn vector_bytes(id: &str, vector: &EntangledHVec) -> usize {
    64 + id.len() + 4 * vector.indices.len()
}

impl State {
    fn new(limit: usize) -> Self {
        Self {
            documents: DocumentStore::default(),
            sizes: HashMap::new(),
            vectors: BTreeMap::new(),
            derived: BTreeMap::new(),
            bytes: 0,
            limit,
        }
    }

    fn grow(&mut self, add: usize, remove: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_sub(remove).saturating_add(add);
        if self.bytes > self.limit {
            return Err(HistoryError::StateTooLarge { limit: self.limit }.into());
        }
        Ok(())
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
        self.documents.insert_batch(vec![document])
    }

    fn apply(&mut self, mutation: &Mutation) -> Result<()> {
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
                // The engine's rule (`install_deferred_encoding`): only the revision the
                // vectors were computed for, while it still has pending chunks.
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
            // Meaning memory, relations and the audit queue are not part of a view.
            _ => {}
        }
        Ok(())
    }
}

/// The document and raw-vector state at one journal position. Read-only; every read fails
/// with [`HistoryError::Compacted`] once the store has compacted since the view was built.
pub struct HistoryView<'a> {
    core: &'a WarmCore,
    position: JournalPosition,
    documents: DocumentStore,
    vectors: BTreeMap<String, EntangledHVec>,
}

impl HistoryView<'_> {
    pub fn position(&self) -> &JournalPosition {
        &self.position
    }

    /// Holds the compaction lock for one read, after checking the view's generation is live.
    fn live(&self) -> Result<crate::core::sync::MutexGuard<'_, ()>> {
        let guard = self.core.compaction.lock();
        if self.core.arena.checkpoint().0 != self.position.generation {
            return Err(HistoryError::Compacted.into());
        }
        Ok(guard)
    }

    pub fn get_document(&self, id: &str) -> Result<Option<DocumentRecord>> {
        let _live = self.live()?;
        documents::get_record(&self.documents, id)?
            .map(|record| record.read().1)
            .transpose()
    }

    /// Up to `limit` documents with ids at or after `from`, in id order.
    pub fn list_documents(&self, from: Option<&str>, limit: usize) -> Result<Vec<DocumentRecord>> {
        let _live = self.live()?;
        documents::list_records(&self.documents, from, limit)?
            .into_iter()
            .map(|record| record.read().1)
            .collect()
    }

    pub fn document_count(&self) -> Result<usize> {
        let _live = self.live()?;
        Ok(documents::count(&self.documents))
    }

    /// Document search over the historical state: the built-in lexical and exact dense path.
    pub fn search_documents(
        &self,
        text: &str,
        options: &SearchOptions,
    ) -> Result<Vec<DocumentResult>> {
        let _live = self.live()?;
        let prepared = documents::prepare_search(
            text,
            options,
            self.core
                .config
                .embedding_space
                .as_ref()
                .map(|s| s.dimensions),
        )?;
        let captured = documents::capture_search(&self.documents, vec![prepared]);
        documents::execute_search(&captured, 0)
    }

    pub fn get_vector(&self, id: &str) -> Result<Option<EntangledHVec>> {
        let _live = self.live()?;
        Ok(self.vectors.get(id).cloned())
    }

    /// Ids of the raw vectors, sorted.
    pub fn vector_ids(&self) -> Result<Vec<String>> {
        let _live = self.live()?;
        Ok(self.vectors.keys().cloned().collect())
    }
}
