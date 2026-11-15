// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Opt-in learned-sparse ranking for document search: callers supply a sparse vector
//! (term id to weight) per document at ingest and a sparse query at search; WARM runs no
//! encoder. Vectors live in `sparse-docs.wsp` (see [`crate::core::index::sparse_store`]),
//! written by the first vector supplied, so a store whose callers never supply one has no
//! such file and ranks exactly as before.
//!
//! Ordering at ingest: the id's old row is removed, the document commits, then the new row
//! is written. A crash between the last two leaves a document with no sparse row, which is
//! simply absent from sparse ranking. A row whose document is gone (a crash between a delete
//! and its row removal) is dropped at search time by a liveness check.
//!
//! The file is plaintext, so an encrypted store refuses sparse vectors.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, ensure, Result};

use super::WarmCore;
use crate::core::documents::{DocumentInput, DocumentResult, IngestOptions, SearchOptions};
use crate::core::index::sparse_store::SparseVector;

/// Reciprocal rank fusion constant, as the document search uses for its `rrf` fusion.
const RRF_K: f64 = 60.0;
/// Candidates drawn from each ranking before fusion when `candidateLimit` is unset.
const FUSION_DEPTH: usize = 100;

/// Which ranking answers a document search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RankingMode {
    /// The existing BM25 and optional dense ranking; sparse input is ignored.
    #[default]
    Lexical,
    /// Inner product with the document sparse vectors; text is not consulted.
    Sparse,
    /// Reciprocal rank fusion of the lexical ranking and the sparse ranking over documents.
    Fused,
}

impl std::str::FromStr for RankingMode {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        match name {
            "lexical" => Ok(Self::Lexical),
            "sparse" => Ok(Self::Sparse),
            "fused" => Ok(Self::Fused),
            _ => bail!("ranking must be \"lexical\", \"sparse\" or \"fused\", not {name:?}"),
        }
    }
}

/// A ranking mode with the caller's sparse query.
#[derive(Clone, Debug, Default)]
pub struct SparseSearch {
    pub mode: RankingMode,
    pub query: SparseVector,
}

impl WarmCore {
    pub(super) fn forget_sparse(&self, id: &str) {
        if self.sparse.is_active() {
            if let Err(error) = self.sparse.remove(id) {
                tracing::warn!("sparse vector removal for {id} failed: {error:#}");
            }
        }
    }

    /// Rows in the sparse store (documents with a sparse vector).
    pub fn sparse_document_count(&self) -> usize {
        self.sparse.len()
    }

    /// [`Self::memorize_documents_with`] with an optional sparse vector per document. A
    /// document without one has no sparse row afterwards, even if it replaces a document that
    /// had one. Vectors are validated before anything is written.
    pub fn memorize_documents_sparse(
        &self,
        inputs: Vec<(DocumentInput, Option<SparseVector>)>,
        options: &IngestOptions,
    ) -> Result<Vec<u32>> {
        let supplied = inputs.iter().any(|(_, sparse)| sparse.is_some());
        ensure!(
            !supplied || !self.config.security.encryption_enabled,
            "sparse vectors are stored in plaintext and are not supported on an encrypted store"
        );
        for (_, sparse) in &inputs {
            if let Some(vector) = sparse {
                self.sparse.validate_document(vector)?;
            }
        }
        let mut rows = Vec::new();
        let mut documents = Vec::with_capacity(inputs.len());
        let ids: Vec<String> = inputs.iter().map(|(input, _)| input.id.clone()).collect();
        self.sparse.remove_many(&ids)?;
        for (input, sparse) in inputs {
            if let Some(vector) = sparse {
                rows.push((input.id.clone(), vector));
            }
            documents.push(input);
        }
        let counts = self.memorize_documents_with(documents, options)?;
        self.sparse.put_many(rows)?;
        Ok(counts)
    }

    pub fn memorize_document_sparse(
        &self,
        input: DocumentInput,
        sparse: Option<SparseVector>,
        options: &IngestOptions,
    ) -> Result<u32> {
        Ok(self
            .memorize_documents_sparse(vec![(input, sparse)], options)?
            .pop()
            .expect("one document"))
    }

    /// [`Self::search_documents`] under a ranking mode. `Lexical` is exactly
    /// `search_documents`. `Sparse` and `Fused` honour `k`, `candidateLimit` (fusion depth)
    /// and `documentIds`; they reject `filter` and `sourceUri`, which the sparse ranking
    /// cannot apply.
    pub fn search_documents_ranked(
        &self,
        text: &str,
        options: &SearchOptions,
        ranking: &SparseSearch,
    ) -> Result<Vec<DocumentResult>> {
        if ranking.mode == RankingMode::Lexical {
            return self.search_documents(text, options);
        }
        ensure!(
            options.filter.is_none() && options.source_uri.is_none(),
            "sparse ranking does not support filter or sourceUri"
        );
        let k = options.k.unwrap_or(10) as usize;
        ensure!(k > 0, "k must be positive");
        let allowed: Option<HashSet<&str>> = options
            .document_ids
            .as_ref()
            .map(|ids| ids.iter().map(String::as_str).collect());
        let mut live =
            |id: &str| allowed.as_ref().is_none_or(|set| set.contains(id)) && self.has_document(id);
        if ranking.mode == RankingMode::Sparse {
            let ranked = self.sparse.search(&ranking.query, k, &mut live)?;
            return ranked
                .into_iter()
                .filter_map(|(id, score)| self.sparse_result(&id, score, 0.0).transpose())
                .collect();
        }
        let depth = options
            .candidate_limit
            .map_or(FUSION_DEPTH, |n| n as usize)
            .max(k);
        let mut fused: HashMap<String, f64> = HashMap::new();
        let mut lexical_hits: HashMap<String, DocumentResult> = HashMap::new();
        if !text.trim().is_empty() {
            let lexical = self.search_documents(
                text,
                &SearchOptions {
                    k: Some(depth as u32),
                    ..options.clone()
                },
            )?;
            let mut rank = 0usize;
            for hit in lexical {
                if lexical_hits.contains_key(&hit.document_id) {
                    continue;
                }
                *fused.entry(hit.document_id.clone()).or_default() +=
                    1.0 / (RRF_K + rank as f64 + 1.0);
                lexical_hits.insert(hit.document_id.clone(), hit);
                rank += 1;
            }
        }
        for (rank, (id, _)) in self
            .sparse
            .search(
                &ranking.query,
                depth.min(crate::core::index::sparse_store::MAX_K),
                &mut live,
            )?
            .into_iter()
            .enumerate()
        {
            *fused.entry(id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
        let mut order: Vec<(String, f64)> = fused.into_iter().collect();
        order.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        order.truncate(k);
        order
            .into_iter()
            .filter_map(|(id, score)| match lexical_hits.remove(&id) {
                Some(hit) => Some(Ok(DocumentResult { score, ..hit })),
                None => self.sparse_result(&id, score, 0.0).transpose(),
            })
            .collect()
    }

    /// The first chunk of `id` as a result, or `None` when the document vanished meanwhile.
    fn sparse_result(&self, id: &str, score: f64, lexical: f64) -> Result<Option<DocumentResult>> {
        let Some(chunk) = self.read_document_chunk(id, 0)? else {
            return Ok(None);
        };
        Ok(Some(DocumentResult {
            id: chunk.id,
            document_id: id.to_owned(),
            source_uri: chunk.document.source_uri,
            version: chunk.document.version,
            metadata: chunk.document.metadata,
            text: chunk.text,
            start_byte: chunk.start_byte,
            end_byte: chunk.end_byte,
            score,
            lexical_score: lexical,
            semantic_score: None,
        }))
    }
}
