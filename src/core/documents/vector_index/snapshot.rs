// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(super) struct RowRef {
    pub(super) document: Arc<StoredDocument>,
    pub(super) chunk: u32,
}

/// The embedded chunks of one store generation with the indexes over them.
pub(crate) struct VectorSnapshot {
    pub(super) identity: StoreIdentity,
    pub(super) generation: u64,
    pub(super) dim: usize,
    pub(super) rows: Vec<RowRef>,
    pub(super) exact: ExactScan,
    pub(super) graph: Option<GraphIndex>,
    pub(super) candidate_identity: Option<VectorCandidateIdentity>,
    pub(super) rerank_candidates: Option<usize>,
    pub(super) zero_prefix: bool,
    pub(super) graph_pending: bool,
}

impl VectorSnapshot {
    pub(crate) fn rows(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn has_graph(&self) -> bool {
        self.graph.is_some()
    }

    pub(crate) fn defer_graph(&mut self) {
        self.graph_pending = !self.has_graph() && self.rows() >= DEFAULT_GRAPH_MIN_ROWS;
    }

    pub(crate) fn graph_pending(&self) -> bool {
        self.graph_pending
    }

    pub(crate) fn matches_embedding_space(&self, space: &EmbeddingSpace) -> bool {
        self.candidate_identity
            .as_ref()
            .is_none_or(|identity| &identity.embedding_space == space)
    }

    pub(super) fn index(&self, mode: VectorIndexMode) -> Result<&dyn VectorIndex> {
        match mode {
            VectorIndexMode::Exact => Ok(&self.exact),
            VectorIndexMode::Graph if self.zero_prefix => Ok(&self.exact),
            VectorIndexMode::Graph => self
                .graph
                .as_ref()
                .map(|graph| graph as &dyn VectorIndex)
                .ok_or_else(|| anyhow::Error::new(GraphUnavailable)),
            VectorIndexMode::Auto => Ok(self
                .graph
                .as_ref()
                .map_or(&self.exact as &dyn VectorIndex, |graph| {
                    graph as &dyn VectorIndex
                })),
        }
    }
}

/// The rows captured under the store's read lock; [`build_configured_vector_snapshot`] turns it into a
/// snapshot without any lock. `embeddings[i]` is the handle of `rows[i]`'s chunk embedding,
/// shared with the store (a reference count, not a copy).
pub(crate) struct VectorCapture {
    pub(super) identity: StoreIdentity,
    pub(super) generation: u64,
    pub(super) dim: usize,
    pub(super) rows: Vec<RowRef>,
    pub(super) embeddings: Vec<Embedding>,
}

/// Captures every embedded chunk's embedding handle. `None` when the store has fewer than
/// [`SNAPSHOT_MIN_ROWS`] embedded chunks.
pub(crate) fn capture_vector_rows(
    store: &DocumentStore,
    dim: usize,
) -> Result<Option<VectorCapture>> {
    let count = store.embedded_count();
    if dim == 0 || count < SNAPSHOT_MIN_ROWS {
        return Ok(None);
    }
    ensure!(
        count <= MAX_ROWS,
        "vector snapshot exceeds {MAX_ROWS} embedded chunks"
    );
    ensure!(
        count
            .checked_mul(dim)
            .is_some_and(|values| values <= MAX_MATRIX_VALUES),
        "vector snapshot exceeds {MAX_MATRIX_VALUES} embedding values"
    );
    let mut rows = Vec::new();
    rows.try_reserve_exact(count)
        .context("cannot allocate vector snapshot rows")?;
    let mut embeddings = Vec::new();
    embeddings
        .try_reserve_exact(count)
        .context("cannot allocate vector snapshot embedding handles")?;
    for id in &store.embedded_documents {
        let Some(document) = store.documents.get(id) else {
            bail!("embedded document {id:?} is missing from the store");
        };
        for (chunk, stored) in document.chunks.iter().enumerate() {
            let Some(embedding) = &stored.embedding else {
                continue;
            };
            ensure!(
                embedding.len() == dim,
                "stored embedding has {} dimensions, the embedding space {dim}",
                embedding.len()
            );
            ensure!(
                rows.len() < count,
                "store counts {count} embedded chunks but holds more"
            );
            embeddings.push(embedding.clone());
            rows.push(RowRef {
                document: Arc::clone(document),
                chunk: u32::try_from(chunk).context("chunk index over u32")?,
            });
        }
    }
    ensure!(
        rows.len() == count,
        "store counts {count} embedded chunks but holds {}",
        rows.len()
    );
    store.vector_changes.write().capture(store.generation);
    Ok(Some(VectorCapture {
        identity: store.identity,
        generation: store.generation,
        dim,
        rows,
        embeddings,
    }))
}

/// Builds the default exact scan and, with enough rows, the vertex graph.
#[cfg(test)]
pub(super) fn build_vector_snapshot(
    capture: VectorCapture,
    graph_min_rows: usize,
) -> Result<VectorSnapshot> {
    build_vector_snapshot_inner(capture, graph_min_rows, None, false)
}

pub(crate) fn build_configured_vector_snapshot(
    capture: VectorCapture,
    graph_min_rows: usize,
    space: &EmbeddingSpace,
    candidates: Option<&VectorCandidateConfig>,
    high_recall: bool,
) -> Result<VectorSnapshot> {
    if let Some(candidates) = candidates {
        candidates.validate(space)?;
    }
    ensure!(
        capture.dim == space.dimensions,
        "candidate and full embedding spaces differ"
    );
    build_vector_snapshot_inner(
        capture,
        graph_min_rows,
        candidates.map(|candidates| (space, candidates)),
        high_recall,
    )
}

pub(super) fn build_vector_snapshot_inner(
    capture: VectorCapture,
    graph_min_rows: usize,
    candidates: Option<(&EmbeddingSpace, &VectorCandidateConfig)>,
    high_recall: bool,
) -> Result<VectorSnapshot> {
    let VectorCapture {
        identity,
        generation,
        dim,
        rows,
        embeddings,
    } = capture;
    let embeddings: Arc<[Embedding]> = embeddings.into();
    let exact = ExactScan {
        dim,
        rows: Arc::clone(&embeddings),
    };
    let mut zero_prefix = false;
    let graph = if rows.len() >= graph_min_rows.max(SNAPSHOT_MIN_ROWS) {
        let candidate_dimensions = candidates.map_or(dim, |(_, config)| config.dimensions);
        let params = graph_build_params(high_recall, rows.len())?;
        let mut matrix = Vec::new();
        matrix
            .try_reserve_exact(
                rows.len()
                    .checked_mul(candidate_dimensions)
                    .context("candidate matrix size overflow")?,
            )
            .context("cannot allocate the graph build matrix")?;
        for embedding in embeddings.iter() {
            if candidates.is_some() {
                if !append_normalized_prefix(embedding, candidate_dimensions, &mut matrix)? {
                    zero_prefix = true;
                    break;
                }
            } else {
                matrix.extend_from_slice(embedding);
            }
        }
        if zero_prefix {
            None
        } else {
            let built = Graph::build_owned(matrix, candidate_dimensions, &params)
                .map_err(|e| anyhow::anyhow!("graph build: {e}"))?;
            let encoded = VGraph::from_graph(&built, true)
                .map_err(|e| anyhow::anyhow!("graph encode: {e}"))?;
            drop(built);
            // The builder's candidate matrix and per-vertex neighbour lists
            // are freed now; hand the allocator's free pages back so they leave the resident set.
            crate::core::os_memory::release_free_heap();
            ensure!(
                encoded.len() == rows.len(),
                "graph index has {} vertices for {} rows",
                encoded.len(),
                rows.len()
            );
            Some(GraphIndex {
                graph: encoded,
                build_degree: params.degree,
                build_ef: params.build_ef,
                high_recall,
                candidate_dimensions: candidates.map(|(_, config)| config.dimensions),
                exact: ExactScan {
                    dim,
                    rows: Arc::clone(&embeddings),
                },
            })
        }
    } else {
        None
    };
    Ok(VectorSnapshot {
        identity,
        generation,
        dim,
        rows,
        exact,
        graph,
        candidate_identity: candidates.map(|(space, config)| VectorCandidateIdentity {
            embedding_space: space.clone(),
            dimensions: config.dimensions,
            normalization: "prefix-l2-v1".into(),
        }),
        rerank_candidates: candidates.map(|(_, config)| config.rerank_candidates),
        zero_prefix,
        graph_pending: false,
    })
}

pub(super) fn graph_build_params(high_recall: bool, rows: usize) -> Result<BuildParams> {
    let params = if high_recall {
        BuildParams {
            degree: 64,
            build_ef: 500,
            ..Default::default()
        }
    } else {
        BuildParams {
            degree: 64,
            build_ef: 256,
            ..Default::default()
        }
    };
    let max_edges = MAX_ROWS * if high_recall { 32 } else { 64 };
    ensure!(
        rows.checked_mul(params.degree)
            .is_some_and(|edges| edges <= max_edges),
        "vector graph exceeds the adjacency limit"
    );
    Ok(params)
}

pub(super) fn staleness(store_generation: u64, snapshot_generation: u64) -> u64 {
    if store_generation == snapshot_generation {
        0
    } else {
        store_generation.abs_diff(snapshot_generation).max(1)
    }
}

pub(crate) fn vector_index_status(
    snapshot: Option<&VectorSnapshot>,
    store: &DocumentStore,
) -> VectorIndexStatus {
    match snapshot {
        None => VectorIndexStatus {
            rows: 0,
            generation: 0,
            current: false,
            stale_mutations: 0,
            graph: false,
            graph_degree: None,
            graph_build_ef: None,
            candidate_identity: None,
            rerank_candidates: None,
            graph_index_bytes: None,
            graph_residual_bytes: None,
            rebuild_recommended: store.embedded_count() >= SNAPSHOT_MIN_ROWS,
        },
        Some(snapshot) => {
            let same_store = snapshot.identity == store.identity;
            let stale =
                staleness(store.generation, snapshot.generation).max(u64::from(!same_store));
            VectorIndexStatus {
                rows: snapshot.rows(),
                generation: snapshot.generation,
                current: stale == 0,
                stale_mutations: stale,
                graph: snapshot.has_graph(),
                graph_degree: snapshot.graph.as_ref().map(|graph| graph.build_degree),
                graph_build_ef: snapshot.graph.as_ref().map(|graph| graph.build_ef),
                candidate_identity: snapshot.candidate_identity.clone(),
                rerank_candidates: snapshot.rerank_candidates,
                graph_index_bytes: snapshot
                    .graph
                    .as_ref()
                    .map(|graph| graph.graph.index_bytes()),
                graph_residual_bytes: snapshot
                    .graph
                    .as_ref()
                    .map(|graph| graph.graph.residual_bytes()),
                rebuild_recommended: !same_store
                    || stale >= REBUILD_AFTER_MUTATIONS
                    || snapshot.graph_pending,
            }
        }
    }
}
