// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use super::config::WarmConfig;
#[cfg(not(target_arch = "wasm32"))]
use super::documents::Analyzer;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingSpace {
    pub model: String,
    pub revision: String,
    pub dimensions: usize,
    pub normalization: String,
    pub metric: String,
}

#[cfg(any(feature = "local-embed", feature = "local-models"))]
impl EmbeddingSpace {
    pub(crate) fn effective(
        model: String,
        dimensions: usize,
        normalization: &str,
        descriptor: &impl Serialize,
    ) -> Result<Self> {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(serde_json::to_vec(descriptor)?);
        Ok(Self {
            model,
            revision: format!(
                "hms-embedding-v1:{}",
                digest
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ),
            dimensions,
            normalization: normalization.into(),
            metric: "cosine".into(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg(not(target_arch = "wasm32"))]
struct StoreSchema {
    version: u32,
    dimensions: usize,
    text_encoder: String,
    vector_encoder: String,
    metric: String,
    encrypted: bool,
    meaning_enabled: bool,
    embedding_space: Option<EmbeddingSpace>,
    /// Absent for the legacy analyzer, so default stores keep their bytes and older releases
    /// open them; any other analyzer also marks each chunk it wrote (binary flag bit 7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lexical_analyzer: Option<String>,
}

/// Checks the store's `store.json` against `config` (writing it for a new store) and returns
/// its schema version, which fixes the journal format the store is written in, and its lexical
/// analyzer: the one recorded, else `config.analyzer` for a new store, else the legacy one.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn validate_store(
    path: &Path,
    dimensions: usize,
    config: &WarmConfig,
) -> Result<(u32, Analyzer)> {
    let mut expected = StoreSchema {
        version: if config.vector_records {
            super::format::VECTOR_RECORDS_SCHEMA_VERSION
        } else {
            super::format::STORE_SCHEMA_DEFAULT_VERSION
        },
        dimensions,
        text_encoder: config.text_encoder.name().into(),
        vector_encoder: "signed-projection-v2".into(),
        metric: "jaccard".into(),
        encrypted: config.security.encryption_enabled,
        meaning_enabled: config.meaning.enabled,
        embedding_space: config.embedding_space.clone(),
        lexical_analyzer: None,
    };
    let schema_path = path.join("store.json");
    if schema_path.exists() {
        let bytes = std::fs::read(schema_path)?;
        if let Some(version) = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v["version"].as_u64())
            .filter(|&v| v > u64::from(super::format::STORE_SCHEMA_VERSION))
        {
            bail!(
                "store schema version {version} is newer than the supported version {}; open it with a release that supports it",
                super::format::STORE_SCHEMA_VERSION
            );
        }
        let actual: StoreSchema = serde_json::from_slice(&bytes)?;
        let recorded = match &actual.lexical_analyzer {
            Some(name) => name.parse::<Analyzer>()?,
            None => Analyzer::Legacy,
        };
        if let Some(wanted) = config.analyzer.filter(|&wanted| wanted != recorded) {
            bail!("store was created with the {recorded} lexical analyzer, not {wanted}; reopen without an analyzer or with {recorded}");
        }
        expected.lexical_analyzer = actual.lexical_analyzer.clone();
        if config.vector_records && actual.version < super::format::VECTOR_RECORDS_SCHEMA_VERSION {
            bail!("vector records require store schema 7; use warm-format upgrade to create an upgraded copy before opening with vectorRecords");
        }
        // Every schema from the oldest supported one opens; only the version differs between
        // them (it selects the journal format).
        if (super::format::STORE_SCHEMA_OLDEST..=super::format::STORE_SCHEMA_VERSION)
            .contains(&actual.version)
        {
            expected.version = actual.version;
        }
        if actual != expected {
            bail!("incompatible store schema: dimensions, encoder, embedding model, encryption, or meaning configuration changed; reopen with the original configuration or re-encode source documents into a new store");
        }
        return Ok((expected.version, recorded));
    } else {
        if path.join("vectors_data.bin").exists() {
            bail!("legacy store has no embedding schema; preserve it and use warm-admin reencode with original source documents to create a new store");
        }
        super::audit_effects::AuditEffectStream::open(path)?;
        super::audit_effects::crash_point("initialized");
        let analyzer = config.analyzer.unwrap_or_default();
        expected.lexical_analyzer = (!analyzer.is_legacy()).then(|| analyzer.name().to_owned());
        super::durable_file::atomic_write(&schema_path, &serde_json::to_vec_pretty(&expected)?)?;
        Ok((expected.version, analyzer))
    }
}
