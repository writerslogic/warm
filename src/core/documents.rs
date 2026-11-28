// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use super::{encoding::TextEncoder, entangled::EntangledHVec};

pub(crate) mod binary;
#[cfg(test)]
mod compact_term_tests;
mod packed_terms;
use packed_terms::StoredTerms;

pub mod analysis;
#[cfg(test)]
mod census;
mod dense;
mod image;
mod impact;
mod lists;
mod screen;
pub use analysis::{analyze, stem, Analyzer};
mod index;
mod lexical;
mod listing;
mod memory;
#[cfg(test)]
mod multilingual_tests;
mod overlay;
mod positions;
mod postings;
mod reorder;
mod search;
mod text;
mod text_blocks;
#[cfg(not(target_arch = "wasm32"))]
mod vector_index;
pub(crate) use dense::{build_dense_index, capture_dense_index, publish_dense_index};
pub(crate) use image::{
    binding as checkpoint_binding, known_magic as checkpoint_magic_known,
    recorded_crc as checkpoint_crc, representable as checkpoint_representable, Backing,
    ServingImage, CRC_START as CHECKPOINT_CRC_START, HEADER_LEN as CHECKPOINT_HEADER_LEN,
};
pub(crate) use index::DocumentWithText;
pub(crate) use index::{canonical_staging_path, opaque_counter_start, DocumentStore};
pub(crate) use lexical::execute as execute_lexical;
pub(crate) use lexical::plain_search as plain_lexical_search;
pub(crate) use lexical::served as served_lexical;
pub use lexical::{LexicalExecution, LexicalHit, LexicalOptions, LexicalQuery, LexicalResults};
pub(crate) use listing::{
    capture_chunk, count, get_info, get_record, list_info, list_records, CapturedDocumentRead,
};
pub use listing::{DocumentChunkRecord, DocumentInfo, DocumentReadChanged, DocumentRecord};
pub use memory::{MemoryDocumentStore, MemoryStoreStats};
pub(crate) use overlay::Base as CheckpointBase;
#[cfg(test)]
pub(crate) use search::prepare_search;
pub(crate) use search::{
    capture_search, count_matches, execute_search, prepare_search_with, rank_search, read_ranked,
    validate_search, CapturedSearch, PreparedSearch,
};
pub(crate) use text::TextSegment;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use vector_index::{
    build_configured_vector_snapshot, capture_vector_rows, search_vector_snapshot_with_report,
    vector_index_status, VectorIndexSlot,
};
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use vector_index::{rank_vector_snapshot, read_vector_hits};
#[cfg(not(target_arch = "wasm32"))]
pub use vector_index::{
    GraphUnavailable, VectorCandidateIdentity, VectorIndex, VectorIndexKind, VectorIndexMode,
    VectorIndexStatus, VectorQueryOptions, VectorSearchDiagnostics, VectorSearchResults,
    VectorSearchRoute, DEFAULT_EF, DEFAULT_GRAPH_MIN_ROWS, MAX_EF, MAX_K, REBUILD_AFTER_MUTATIONS,
    SNAPSHOT_MIN_ROWS,
};

#[cfg(test)]
pub(crate) use image::encode_v1 as encode_checkpoint_v1;

/// Marsaglia xorshift64 step for deterministic test data.
#[cfg(test)]
pub(crate) fn xorshift64(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

mod chunking;
#[cfg(test)]
mod cosine_batch_tests;
mod embedding;
mod filter;
mod prepare;
mod query;
mod stored;
mod terms;
#[cfg(test)]
mod tests;
mod types;
pub use self::chunking::*;
pub(crate) use self::embedding::*;
pub(crate) use self::filter::{
    indexed_equality, matches_metadata, operators, unsupported_operator,
};
pub use self::filter::{validate_filter, UnsupportedFilterOperator, SUPPORTED_FILTER_OPERATORS};
pub(crate) use self::prepare::*;
pub(crate) use self::query::*;
pub(crate) use self::stored::*;
pub(crate) use self::terms::*;
pub use self::types::*;
