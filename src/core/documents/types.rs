// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Documents, and chunks, one `memorize_documents_with` call accepts.
pub const MAX_BATCH_ITEMS: usize = 4096;

/// A document batch over one of the engine's batch limits (items, chunks, source or metadata
/// bytes, embedding values, transaction bytes). Nothing was written; a smaller batch can succeed.
#[derive(Debug, Clone, Copy)]
pub struct BatchTooLarge(pub &'static str);

impl std::fmt::Display for BatchTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for BatchTooLarge {}
pub(crate) const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_CHUNK_BYTES: usize = 64 * 1024;
pub(crate) const CHUNK_PREFIX: &str = "hms:chunk:";

#[cfg_attr(feature = "node-api", napi_derive::napi(object))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInput {
    pub id: String,
    pub text: String,
    pub source_uri: Option<String>,
    pub version: Option<String>,
    pub metadata: Option<Value>,
    pub chunk_words: Option<u32>,
    pub overlap_words: Option<u32>,
    pub store_text: Option<bool>,
    pub embeddings: Option<Vec<Vec<f64>>>,
    /// Counts the terms of the text's first line this many times in the first chunk's term
    /// frequencies and word count, as if the title were repeated; 1 or absent leaves the
    /// document exactly as before this option existed.
    pub title_weight: Option<u32>,
}

/// The embedding model a store was built with ([`crate::core::WarmCore::embedding_model`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelInfo {
    pub model: String,
    pub revision: String,
    pub dimensions: usize,
    pub normalization: String,
    pub metric: String,
    /// sha256 of the model pack; `None` while no embedder is installed.
    pub pack_sha256: Option<String>,
}

/// When a document's chunk hypervectors are computed. Terms, postings and text are stored at
/// ingest in every mode, so lexical search sees the document at once; only the vector side
/// (`WarmCore::query` over chunk vectors) differs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncodeMode {
    /// Encode at ingest: the behavior before this option existed.
    #[default]
    Eager,
    /// Commit the document lexically searchable now; its vectors are encoded from the stored
    /// text on the first vector query, by `maintain_indices`, `train_document_index` or
    /// `encode_pending_documents`, and persisted then. Needs a text encoder and the chunk text
    /// (`storeText` not false).
    Deferred,
    /// Never encode: the document stays outside vector search until memorized again.
    Lexical,
}

impl EncodeMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Eager => "eager",
            Self::Deferred => "deferred",
            Self::Lexical => "lexical",
        }
    }
}

impl std::str::FromStr for EncodeMode {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        match name {
            "eager" => Ok(Self::Eager),
            "deferred" => Ok(Self::Deferred),
            "lexical" => Ok(Self::Lexical),
            _ => bail!("encode mode must be \"eager\", \"deferred\" or \"lexical\", not {name:?}"),
        }
    }
}

/// Options of one `memorize_documents_with` call; the default is exactly `memorize_documents`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IngestOptions {
    pub encode: EncodeMode,
    /// Index each word lowercased but not stemmed, so a document matches only the query words
    /// it contains as written. The default stems every word with the English Snowball stemmer.
    /// Query terms are matched in both forms, so one store may hold documents of either kind.
    pub unstemmed: bool,
    /// Retain token positions for exact, ordered phrase queries. Off by default.
    pub positions: bool,
    /// Index one whole article as one lexical chunk (up to 8 MiB). Requires lexical
    /// encoding and positions; chunkWords/overlapWords do not split the article.
    pub whole_document: bool,
}

/// Persisted state of a chunk's hypervector. Written only when not `Encoded`, so a store
/// without deferred or lexical documents serializes byte for byte as before the flag existed,
/// and a chunk without the field reads as `Encoded`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VectorState {
    #[default]
    Encoded,
    /// Memorized with [`EncodeMode::Deferred`]; `vector` is empty and the chunk is not in the
    /// vector index until `WarmCore::encode_pending_documents` installs its encoding.
    Pending,
    /// Memorized with [`EncodeMode::Lexical`]; `vector` is empty for good.
    Lexical,
}

impl VectorState {
    pub(crate) fn is_encoded(&self) -> bool {
        *self == Self::Encoded
    }
}

#[cfg_attr(feature = "node-api", napi_derive::napi(object))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentChunk {
    pub id: String,
    pub text: String,
    pub start_byte: u32,
    pub end_byte: u32,
}

#[cfg_attr(feature = "node-api", napi_derive::napi(object))]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchOptions {
    pub k: Option<u32>,
    pub candidate_limit: Option<u32>,
    pub filter: Option<Value>,
    pub source_uri: Option<String>,
    pub document_ids: Option<Vec<String>>,
    pub embedding: Option<Vec<f64>>,
    pub lexical_weight: Option<f64>,
    pub semantic_weight: Option<f64>,
    pub min_semantic_score: Option<f64>,
    /// How the lexical and semantic lists combine: `"blend"` (default) adds each list's
    /// min-max normalized scores over its top `candidateLimit`, weighted by `lexicalWeight`
    /// and `semanticWeight`; `"rrf"` is reciprocal rank fusion with k = 60.
    pub fusion: Option<String>,
    /// BM25 term-frequency saturation `k1`; the default scores exactly as before this option
    /// existed (1.2).
    pub bm25_k1: Option<f64>,
    /// BM25 length normalization `b` in `[0, 1]`; the default scores exactly as before (0.75).
    pub bm25_b: Option<f64>,
}

/// BM25 constants as the scorer multiplies them: `k1 + 1`, `k1`, `1 - b` and `b`. The default
/// keeps the literal constants the scorer always used, so default scores stay bit-identical.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bm25 {
    pub(crate) k1_plus_one: f64,
    pub(crate) k1: f64,
    pub(crate) one_minus_b: f64,
    pub(crate) b: f64,
}

impl Bm25 {
    pub(crate) const DEFAULT: Self = Self {
        k1_plus_one: 2.2,
        k1: 1.2,
        one_minus_b: 0.25,
        b: 0.75,
    };

    pub(crate) fn from_options(options: &SearchOptions) -> Result<Self> {
        let (Some(k1), Some(b)) = (
            options.bm25_k1.or(Some(Self::DEFAULT.k1)),
            options.bm25_b.or(Some(Self::DEFAULT.b)),
        ) else {
            unreachable!("defaults fill both parameters");
        };
        if options.bm25_k1.is_none() && options.bm25_b.is_none() {
            return Ok(Self::DEFAULT);
        }
        ensure!(
            k1.is_finite() && k1 >= 0.0 && (0.0..=1.0).contains(&b),
            "bm25K1 must be finite and nonnegative and bm25B in [0, 1]"
        );
        Ok(Self {
            k1_plus_one: k1 + 1.0,
            k1,
            one_minus_b: 1.0 - b,
            b,
        })
    }

    /// The BM25 length factor of a chunk of `word_count` words against `average`.
    #[inline]
    pub(crate) fn norm(self, word_count: f64, average: f64) -> f64 {
        self.one_minus_b + self.b * word_count / average
    }

    /// The inverse document frequency of a term in `df` of `n` chunks.
    #[inline]
    pub(crate) fn idf(n: f64, df: f64) -> f64 {
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    /// The weight of a term with inverse document frequency `idf` seen `tf` times in a chunk
    /// whose length normalization is `norm`.
    #[inline]
    pub(crate) fn weight(self, idf: f64, tf: f64, norm: f64) -> f64 {
        if tf == 0.0 {
            return 0.0;
        }
        if self.k1 <= 1.0 || self == Self::DEFAULT {
            return idf * tf * self.k1_plus_one / (tf + self.k1 * norm);
        }
        let saturation = tf * (1.0 + self.k1.recip()) / (tf / self.k1 + norm);
        idf * saturation
    }
}

#[cfg_attr(feature = "node-api", napi_derive::napi(object))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentResult {
    pub id: String,
    pub document_id: String,
    pub source_uri: Option<String>,
    pub version: String,
    pub metadata: Value,
    pub text: Option<String>,
    pub start_byte: u32,
    pub end_byte: u32,
    pub score: f64,
    pub lexical_score: f64,
    pub semantic_score: Option<f64>,
}
