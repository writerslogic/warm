// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Vector query dispatch for the document store.
//!
//! Mechanism: a vector-only, unfiltered query does not need the lexical capture in
//! `search.rs`, which clones one `Arc<StoredDocument>` per stored chunk and scores rows
//! through the document pointers on every query. It is answered from a [`VectorSnapshot`]:
//! the embedded chunks captured at one store generation as a row list over the store's own
//! `f32` embeddings ([`super::Embedding`], one shared allocation per chunk, so the snapshot
//! adds no copy of the matrix), with an optional vertex graph (`qgraph::VGraph`) over the
//! same rows. Every [`VectorIndex`] maps a query to row ids and the snapshot maps rows back
//! to documents. Readers share the snapshot through one `Arc`, so a query holds the store's
//! read lock only to read the generation and the text segment for fresh queries. Stale
//! exact queries derive live-row eligibility from a bounded changed-ID ledger and binary
//! ranges in the ID-ordered rows, then score captured immutable owners after releasing it.
//!
//! Data layout: `rows[i]` owns the document of row `i` and the chunk index inside it;
//! `exact.rows[i]` is that chunk's embedding handle, l2-normalised `f32` values rounded from
//! the `f64` normalisation at ingest. The graph build needs one contiguous matrix; it is
//! assembled for the build and freed with the builder, so it is transient memory, not part
//! of the snapshot.
//!
//! Consistency: the store bumps `generation` on every insert, replace and delete
//! (`index.rs`), and a snapshot records the generation it was captured at.
//! - current (`store.generation == snapshot.generation`): the rows are the live store, hits
//!   are returned as found.
//! - stale: exact captures invalidate the row ranges of changed IDs and capture their live
//!   owners. The generation-covered ledger retains IDs, not deleted payloads, independently
//!   of the dense index; overflow, a coverage gap or a different store takes the complete
//!   generic exact path. Every captured row is scored outside the lock, retaining the best
//!   `k` eligible base rows before merging bounded delta chunks. Graph queries retain their
//!   approximate candidate contract and validate owners against the live store; their delta
//!   discovery uses the covered change ledger and falls back to generic exact search if coverage
//!   or the bounded delta is unavailable. Exact capture holds a bit per snapshot row and at most
//!   the dense index's delta document/chunk limits. When fewer than `k` hits survive, the caller
//!   takes the generic exact path.
//!   `rebuild_recommended` asks maintenance for a fresh capture
//!   after [`REBUILD_AFTER_MUTATIONS`] mutations, so the stale window is bounded by the
//!   maintenance interval.
//! - third implementation slot: [`VectorIndexKind::Scan`] is reserved for the graph-free
//!   scan; nothing implements it here.
//!
//! Scores: exact-mode selection uses [`super::Embedding::cosine`], the canonical `f64`
//! accumulation. Graph membership remains approximate. Ties break on `(document id, chunk)`
//! ascending.

use std::{cmp::Ordering, collections::BinaryHeap, sync::Arc};

use crate::core::sync::{Mutex, MutexGuard, RwLock};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use super::dense::{MAX_DELTA_CHUNKS, MAX_DELTA_DOCUMENTS};
use super::index::{text_scrubbed, try_text_of, StoreIdentity};
use super::{DocumentResult, DocumentStore, Embedding, ScreenBound, StoredDocument, TextSegment};
use crate::core::qgraph::{BuildParams, Graph, VGraph, VSearchParams};
use crate::core::{EmbeddingSpace, VectorCandidateConfig};

/// Fewest embedded chunks worth a snapshot; smaller stores take the generic exact path.
pub const SNAPSHOT_MIN_ROWS: usize = 1;
/// Default `graph_min_rows`: a snapshot with fewer rows carries only the exact scan, which
/// beats a graph traversal at that size.
pub const DEFAULT_GRAPH_MIN_ROWS: usize = 4096;
/// Default beam width of a graph query. Recall depends on the corpus and the graph, so a
/// caller that needs a recall target measures it on its own data.
pub const DEFAULT_EF: usize = 128;
pub const MAX_EF: usize = 4096;
pub const MAX_K: usize = 1000;
/// Mutations since the capture after which maintenance builds a fresh snapshot.
pub const REBUILD_AFTER_MUTATIONS: u64 = 64;
const MAX_ROWS: usize = 1 << 22;
/// Upper bound on the graph build matrix: 1 Gi `f32` values (4 GiB), which holds 1M rows of 768
/// dimensions and 500K rows of 1536.
const MAX_MATRIX_VALUES: usize = 1024 * 1024 * 1024;

/// The engine's published snapshot and the build lock.
#[derive(Default)]
pub(crate) struct VectorIndexSlot {
    current: RwLock<Option<Arc<VectorSnapshot>>>,
    building: Mutex<()>,
}

impl VectorIndexSlot {
    pub(crate) fn get(&self) -> Option<Arc<VectorSnapshot>> {
        self.current.read().clone()
    }

    pub(crate) fn publish(&self, snapshot: Arc<VectorSnapshot>) {
        *self.current.write() = Some(snapshot);
    }

    pub(crate) fn clear(&self) {
        *self.current.write() = None;
    }

    /// `None` while another build is running.
    pub(crate) fn try_build(&self) -> Option<MutexGuard<'_, ()>> {
        self.building.try_lock()
    }

    /// The build lock, waiting for a running build to finish.
    pub(crate) fn build(&self) -> MutexGuard<'_, ()> {
        self.building.lock()
    }
}

mod rank;
mod scan;
mod snapshot;
#[cfg(test)]
mod tests;
mod types;
pub(crate) use self::rank::*;
pub(crate) use self::scan::*;
pub(crate) use self::snapshot::*;
pub use self::types::*;
