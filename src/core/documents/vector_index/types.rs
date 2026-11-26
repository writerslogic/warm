// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Which implementation answered, or may answer, a vector query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VectorIndexKind {
    /// Exact cosine over the store's `f32` rows.
    Exact,
    /// The vertex graph (`qgraph::VGraph`) with exact re-scoring of the fetched rows.
    Graph,
    /// Reserved for the graph-free scan.
    Scan,
}

/// Route of one vector query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VectorIndexMode {
    /// The graph when the snapshot has one, the exact scan otherwise.
    #[default]
    Auto,
    /// Always the exact scan (the generic path when no snapshot exists).
    Exact,
    /// The graph, or [`GraphUnavailable`] when none is built.
    Graph,
}

impl VectorIndexMode {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "auto" => Some(Self::Auto),
            "exact" => Some(Self::Exact),
            "graph" => Some(Self::Graph),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Exact => "exact",
            Self::Graph => "graph",
        }
    }
}

/// Per-query options of [`crate::WarmCore::search_vectors`].
#[derive(Clone, Debug)]
pub struct VectorQueryOptions {
    pub mode: VectorIndexMode,
    /// Beam width of a graph query (`1..=MAX_EF`); ignored by the exact scan.
    pub ef: usize,
    /// Lowest cosine returned, in `[-1, 1]`.
    pub min_score: f64,
    /// Exact metadata matches, evaluated before full-dimensional ranking.
    pub filter: Option<serde_json::Value>,
    pub source_uri: Option<String>,
    pub document_ids: Option<Vec<String>>,
}

impl Default for VectorQueryOptions {
    fn default() -> Self {
        Self {
            mode: VectorIndexMode::Auto,
            ef: DEFAULT_EF,
            min_score: -1.0,
            filter: None,
            source_uri: None,
            document_ids: None,
        }
    }
}

/// Derived candidate identity, separate from the full stored embedding space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorCandidateIdentity {
    pub embedding_space: EmbeddingSpace,
    pub dimensions: usize,
    pub normalization: String,
}

/// The route that actually produced one vector response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VectorSearchRoute {
    FullExact,
    FullGraph,
    ProjectedGraph,
    StaleExact,
    FilteredExact,
    FallbackExact,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchDiagnostics {
    pub route: VectorSearchRoute,
    pub candidate_dimensions: usize,
    pub shortlist_limit: Option<usize>,
    pub graph_degree: Option<usize>,
    pub graph_build_ef: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchResults {
    pub results: Vec<DocumentResult>,
    pub diagnostics: VectorSearchDiagnostics,
}

/// State of the snapshot relative to the live store.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexStatus {
    /// Embedded chunks the snapshot covers (0 without a snapshot).
    pub rows: usize,
    /// Store generation the snapshot was captured at.
    pub generation: u64,
    /// Whether the snapshot is the live store.
    pub current: bool,
    /// Inserts, replaces and deletes since the capture.
    pub stale_mutations: u64,
    /// Whether the snapshot carries a graph index.
    pub graph: bool,
    /// Graph construction settings, present only when a graph is built.
    pub graph_degree: Option<usize>,
    pub graph_build_ef: Option<usize>,
    /// Whether maintenance should capture a fresh snapshot.
    pub rebuild_recommended: bool,
    pub candidate_identity: Option<VectorCandidateIdentity>,
    pub rerank_candidates: Option<usize>,
    /// Logical retained graph buffers, excluding stored embeddings, allocator overhead and scratch.
    pub graph_index_bytes: Option<usize>,
    /// Residual buffers already included in graph_index_bytes.
    pub graph_residual_bytes: Option<usize>,
}

/// A `graph` query reached a store without a graph index.
#[derive(Debug, Clone, Copy)]
pub struct GraphUnavailable;

impl std::fmt::Display for GraphUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no graph index is built; run index maintenance")
    }
}

impl std::error::Error for GraphUnavailable {}

/// A nearest-neighbour index over the rows of one `VectorSnapshot`.
pub trait VectorIndex: Send + Sync {
    fn kind(&self) -> VectorIndexKind;
    fn rows(&self) -> usize;
    /// The `k` best rows by cosine, best first, as `(row, score)`; `ef` is the beam of an
    /// approximate index and ignored by an exact one. `query` is l2-normalised and has the
    /// snapshot's dimensions.
    fn search(&self, query: &[f32], k: usize, ef: usize) -> Result<Vec<(u32, f32)>>;
}
