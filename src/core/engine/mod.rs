// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

mod audit_delivery;
mod background_compaction;
pub use background_compaction::{
    CompactionHandle, CompactionOptions, CompactionProgress, CompactionReport, CompactionResource,
    CompactionStage, CompactionStatus,
};
mod compaction;
mod purge;
pub use compaction::{ReorderOutcome, ReorderReport};
pub use purge::{PurgeStatus, PurgeTicket};
pub(crate) mod concepts;
mod dense_vectors;
mod sparse_docs;
pub use sparse_docs::{RankingMode, SparseSearch};
mod document_api;
mod document_batch;
#[cfg(feature = "local-embed")]
mod embed_api;
mod experience_api;
mod governance;
mod history;
mod image;
mod index_cache;
pub use history::{
    HistoryBounds, HistoryError, HistoryOptions, HistoryRevision, HistoryView, JournalPosition,
    RevisionKind, TimeResolution,
};
pub(crate) mod journal;
pub(crate) mod knowledge;
pub(crate) mod memory_access;
pub(crate) mod multi_hop;
mod mutation;
mod mutation_gate;
mod vector_records;

#[cfg(test)]
mod deferred_cache_tests;
#[cfg(test)]
mod document_memory_census;
#[cfg(test)]
mod document_snapshot_tests;
#[cfg(test)]
mod fold_tests;
#[cfg(test)]
mod reorder_tests;

#[cfg(all(test, feature = "local-models"))]
mod document_stage_tests;
pub(crate) mod query;
pub(crate) mod router;
pub(crate) mod shard;
pub(crate) mod structural;

use anyhow::{Context, Result};

/// Mutations decoded and applied together while replaying a journal frame.
const REPLAY_CHUNK: usize = 512;
use crate::core::sync::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::admission::AdmissionControl;
use super::agency::goals::GoalStore;
use super::agency::planner::{Plan, Planner};
use super::agency::questions::{Question, QuestionGenerator};
use super::agency::self_modify::{ProposalKind, SelfModifier};
use super::atom_memory::AtomMemory;
use super::audit::{AuditLog, AuditOp};
use super::cognition::governor::GovernanceReport;
use super::cognition::r#loop::{CognitionConfig as CognitionLoopConfig, CognitionLoop, Insight};
use super::composite_memory::CompositeMemory;
use super::config::WarmConfig;
use super::decompose::Decomposer;
use super::diffusion::DiffusionFactorizer;
use super::entangled::EntangledHVec;
use super::graph::RelationStore;
use super::ivf::IVFIndex;
use super::role::RoleRegistry;
use super::rules::RuleStore;
use super::storage::PersistentArena;
use super::store_lock::StoreLock;
use super::text::TextProcessor;
use super::triple_store::TripleStore;
use super::types::{GraphPath, Relation, RelationType, TextMetrics};

use mutation::Mutation;
use shard::{ShardManager, ShardSet};

type SignFn<'a> = Box<dyn Fn(&[u8]) -> super::audit::SignatureBytes + 'a>;

/// A configuration whose store holds `dimensions`-wide fixture embeddings.
#[cfg(test)]
pub(crate) fn fixture_config(dimensions: usize) -> WarmConfig {
    WarmConfig {
        embedding_space: Some(super::EmbeddingSpace {
            model: "fixture".into(),
            revision: "1".into(),
            dimensions,
            normalization: "l2".into(),
            metric: "cosine".into(),
        }),
        ..Default::default()
    }
}

/// Persistent operations take mutation_gate before ShardSet and component locks.
pub struct WarmCore {
    config: WarmConfig,
    mutation_gate: mutation_gate::MutationGate,
    /// Word interner carried from one document batch to the next.
    stem_memos: crate::core::documents::StemMemoPool,
    compaction: crate::core::sync::Mutex<()>,
    /// Generation and offset where retained history starts (see `history`).
    history_floor: RwLock<(String, usize)>,
    /// Views built in this generation, shared by later views at the same position.
    history_cache: crate::core::sync::Mutex<history::ViewCache>,
    /// The index metadata (`indices.json`) last written by this process: identical metadata
    /// means identical index files at the same checkpoint, so a repeated persist skips the
    /// writes.
    persisted_indices: crate::core::sync::Mutex<Vec<u8>>,
    revision: std::sync::atomic::AtomicU64,
    vector_record_instance: [u8; 16],
    /// The frame format this store is written in (its `store.json` schema); both are read.
    journal: journal::JournalFormat,
    /// `store.json` schema version of the open store.
    schema: u32,
    /// The store's lexical analyzer, recorded in `store.json` (see [`documents::Analyzer`]).
    analyzer: crate::core::documents::Analyzer,
    pub(crate) arena: PersistentArena,
    pub(crate) dimensions: usize,
    pub(crate) storage_path: PathBuf,
    shards: RwLock<ShardSet>,
    graph: RelationStore,
    documents: RwLock<super::documents::DocumentStore>,
    image: image::ImageState,
    /// Snapshot answering unfiltered vector queries (`search_vectors`); built by maintenance.
    vector_index: super::documents::VectorIndexSlot,
    dense: dense_vectors::DenseVectors,
    sparse: crate::core::index::sparse_store::SparseStore,
    #[cfg(feature = "local-models")]
    model_stages: RwLock<Option<Arc<crate::core::models::ModelStages>>>,
    #[cfg(feature = "local-embed")]
    text_embedder: RwLock<Option<Arc<crate::core::embed::Embedder>>>,
    derived: RwLock<std::collections::BTreeMap<String, Vec<String>>>,
    atom_memory: Option<Arc<AtomMemory>>,
    composite_memory: Option<Arc<CompositeMemory>>,
    triple_store: Option<Arc<TripleStore>>,
    role_registry: Option<RoleRegistry>,
    rule_store: Option<RuleStore>,
    decomposer: Option<Decomposer>,
    admission: Option<AdmissionControl>,
    cognition_loop: crate::core::sync::Mutex<Option<CognitionLoop>>,
    goal_store: Option<GoalStore>,
    self_modifier: Option<SelfModifier>,
    audit: Option<AuditLog>,
    audit_effects: Option<super::audit_effects::AuditEffectStream>,
    audit_pending: RwLock<std::collections::VecDeque<super::audit_effects::AuditIntent>>,
    audit_delivery: mutation_gate::MutationGate,
    #[cfg(feature = "security")]
    signing: Option<super::security::SigningManager>,
    #[cfg(feature = "security")]
    encryption: Option<std::sync::Arc<super::security::EncryptionManager>>,
    #[cfg(feature = "provenance")]
    agents: parking_lot::RwLock<super::provenance::access::AgentRegistry>,
    #[cfg(feature = "provenance")]
    provenance: Option<super::provenance::ProvenanceManager>,
    /// Experimental opt-in plastic relation store (lazily created on first use).
    #[cfg(feature = "experimental")]
    connection_graph: crate::core::sync::Mutex<Option<super::connection_graph::ConnectionGraph>>,
    /// Experimental opt-in phasor relational memory (lazily created on first use).
    #[cfg(feature = "experimental")]
    phase_graph: crate::core::sync::Mutex<Option<super::phase_graph::PhaseGraph>>,
    // Declared last so it is dropped after every mmap, index, and store
    // component. No subsequent instance can enter while teardown is flushing.
    /// Purges waiting for a scrubbing compaction (`purge.pending`).
    purge: Arc<purge::PurgeLedger>,
    /// Serializes scrubbing compactions, so a purge whose ticket another one already cleared
    /// does not compact again.
    scrub_gate: crate::core::sync::Mutex<()>,
    /// The experience store of tree-structured observations; absent until the store holds
    /// one, so a store that never receives an observation has no experience files.
    experience: RwLock<Option<super::experience::store::ExperienceStore>>,
    _store_lock: StoreLock,
    /// Opened by `open_read_only`: every mutation is refused and nothing is written back to
    /// the shared store (the engine runs over `reader_snapshot`).
    read_only: bool,
    // Dropped after the lock: removes the private copy the engine ran over.
    reader_snapshot: Option<super::reader_snapshot::Snapshot>,
}

impl WarmCore {}

/// Serialized index files and the metadata binding them to an arena checkpoint.
pub(super) struct EncodedIndices {
    files: Vec<(PathBuf, Vec<u8>)>,
    metadata: Vec<u8>,
}

impl WarmCore {}

impl Drop for WarmCore {
    /// Closing writes a checkpoint when one is due (see `checkpoint_if_due`), so the next open
    /// maps it instead of replaying the frames. A failure leaves the journal as it was.
    fn drop(&mut self) {
        if let Err(error) = self.checkpoint_if_due() {
            tracing::warn!(%error, "document checkpoint not written at close");
        } else {
            self.release_text_cache();
        }
    }
}

mod cognition;
#[cfg(all(test, feature = "experimental"))]
mod connection_graph_tests;
mod construct;
mod core_api;
mod graph_api;
mod indices;
mod meaning_api;
mod provenance_api;
mod reader;
mod replay;
mod training;
