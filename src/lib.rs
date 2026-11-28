// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#![deny(clippy::all)]

#[cfg(feature = "mimalloc")]
pub mod allocator;
mod bindings;
pub mod compatibility;
pub mod core;

#[cfg(feature = "node-api")]
pub use bindings::node::*;

#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::audit::{AuditEntry, AuditOp};
pub use crate::core::documents::{
    analyze, chunk_document, stem, Analyzer, DocumentChunk, DocumentChunkRecord, DocumentInfo,
    DocumentInput, DocumentReadChanged, DocumentRecord, DocumentResult, EmbeddingModelInfo,
    SearchOptions, UnsupportedFilterOperator, SUPPORTED_FILTER_OPERATORS,
};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::engine::{BatchOp, BatchResult};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::engine::{
    CompactionHandle, CompactionOptions, CompactionProgress, CompactionReport, CompactionResource,
    CompactionStage, CompactionStatus,
};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::engine::{RankingMode, ReorderOutcome, ReorderReport, SparseSearch};
pub use crate::core::entangled::EntangledHVec;
pub use crate::core::error::{
    CommittedMutationError, IndeterminateMutationError, ReadOnlyError, StoreLockedError, WarmError,
};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::index::sparse_store::SparseVector;
#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::memory_api::{Memory, MemoryConfig};
pub use crate::core::types::{
    ConceptCandidate, HardwareCapabilities, IndexStatus, MemorizeBatchItem, QueryExplanation,
    RetrievalResult, StorageHealth, TextMetrics,
};
pub use crate::core::EmbeddingSpace;
#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::{
    HistoryBounds, HistoryError, HistoryOptions, HistoryRevision, HistoryView, JournalPosition,
    RevisionKind, TimeResolution,
};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::core::{Refresh, WarmCore};
#[cfg(not(target_arch = "wasm32"))]
pub use WarmCore as HmsCore;
pub use WarmError as HmsError;
#[cfg(feature = "node-api")]
pub use WarmMemorySystem as HolographicMemorySystem;

#[cfg(feature = "provenance")]
pub use crate::core::provenance::status_list::StatusListCredential;
#[cfg(feature = "provenance")]
pub use crate::core::provenance::types::{
    DeletionRecord, ProvenanceRecord, StoreManifest, VerificationDetail, VerificationResult,
};
#[cfg(feature = "provenance")]
pub use crate::core::provenance::TripleProvenanceParams;

#[cfg(test)]
mod tests;

/// Compiles and runs the Rust blocks of the README so the documented quickstart cannot rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
