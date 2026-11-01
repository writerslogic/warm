// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Read and delete access for the memory API (`core::memory_api`). Memory facts are ordinary
//! documents, so nothing here adds a persisted format; it only exposes what the API needs
//! from the document store without widening its visibility.

use anyhow::Result;
use serde_json::Value;

use super::{mutation::Mutation, WarmCore};
use crate::core::documents;

/// Most deletions one journal transaction carries.
pub(crate) const DELETE_BATCH: usize = 4096;

/// Everything needed to store a document again unchanged: its first chunk's text and
/// embedding, its metadata, source and version.
pub(crate) struct DocumentParts {
    pub metadata: Value,
    pub text: Option<String>,
    pub source_uri: Option<String>,
    pub version: String,
    pub embedding: Option<Vec<f64>>,
}

/// Metadata of a document as the memory API sees it.
pub(crate) struct DocumentView<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub metadata: &'a Value,
}

impl WarmCore {
    pub(crate) fn validate_document_search(
        &self,
        text: &str,
        options: &crate::SearchOptions,
    ) -> Result<()> {
        crate::core::documents::validate_search(
            text,
            options,
            self.config
                .embedding_space
                .as_ref()
                .map(|space| space.dimensions),
        )
        .map(drop)
    }

    /// Advances by one per write transaction this process commits or replays, so equal values
    /// bracket an unchanged store and the difference of two readings counts the transactions
    /// between them. It starts at a random value per open, so one reading does not reveal how
    /// many transactions (inserts, deletes) the store has seen.
    pub fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn try_scan_documents(
        &self,
        ids: Option<&std::collections::BTreeSet<String>>,
        filter: Option<&Value>,
        mut visit: impl FnMut(DocumentView<'_>) -> Result<()>,
    ) -> Result<u64> {
        let _transaction = self.mutation_gate.read();
        let revision = self.revision();
        let store = self.documents.read();
        let mut visit_document = |id: &str, version: &str, metadata: &Value| {
            if ids.is_some_and(|ids| !ids.contains(id))
                || !documents::matches_metadata(metadata, filter)
            {
                return Ok(());
            }
            visit(DocumentView {
                id,
                version,
                metadata,
            })
        };
        if store.has_base() {
            if let Some(ids) = ids {
                for id in ids {
                    if let Some(info) = documents::get_info(&store, id)? {
                        visit_document(&info.id, &info.version, &info.metadata)?;
                    }
                }
            } else {
                let mut from = None;
                loop {
                    let page = documents::list_info(&store, from.as_deref(), 256)?;
                    for info in &page {
                        visit_document(&info.id, &info.version, &info.metadata)?;
                    }
                    match page.last() {
                        Some(last) if page.len() == 256 => from = Some(format!("{}\0", last.id)),
                        _ => break,
                    }
                }
            }
            return Ok(revision);
        }
        let candidates = match (ids, store.metadata_candidates(filter)) {
            (Some(ids), Some(metadata)) if metadata.len() < ids.len() => Some(metadata),
            (Some(ids), _) => Some(ids),
            (None, metadata) => metadata,
        };
        if let Some(ids) = candidates {
            for id in ids {
                if let Some(document) = store.get(id) {
                    visit_document(&document.id, &document.version, &document.metadata)?;
                }
            }
        } else {
            for document in store.shared_values() {
                visit_document(&document.id, &document.version, &document.metadata)?;
            }
        }
        Ok(revision)
    }

    pub(crate) fn document_parts(&self, id: &str) -> Result<Option<DocumentParts>> {
        self.document_parts_with(id, || {})
    }

    fn document_parts_with(
        &self,
        id: &str,
        mut after_capture: impl FnMut(),
    ) -> Result<Option<DocumentParts>> {
        for _ in 0..3 {
            let (captured, embedding) = {
                let _transaction = self.mutation_gate.read();
                let store = self.documents.read();
                let Some(captured) = documents::get_record(&store, id)? else {
                    return Ok(None);
                };
                let embedding = store
                    .get(id)
                    .and_then(|document| document.chunks.first())
                    .and_then(|chunk| chunk.embedding.as_ref().map(|embedding| embedding.to_f64()));
                (captured, embedding)
            };
            after_capture();
            let (identity, record) = captured.read();
            let _transaction = self.mutation_gate.read();
            if identity.is_current(&self.documents.read()) {
                return record.map(|record| {
                    Some(DocumentParts {
                        metadata: record.metadata,
                        text: record.text,
                        source_uri: record.source_uri,
                        version: record.version,
                        embedding,
                    })
                });
            }
        }
        Err(documents::DocumentReadChanged.into())
    }

    pub(crate) fn memory_revision_matches(&self, revision: u64) -> bool {
        let _writer = self.mutation_gate.maintenance();
        self.revision() == revision
    }

    pub(crate) fn change_memory_documents(
        &self,
        inputs: Vec<crate::DocumentInput>,
        deleted: &[String],
        revision: u64,
    ) -> Result<bool> {
        anyhow::ensure!(
            inputs
                .len()
                .checked_add(deleted.len())
                .is_some_and(|n| n <= DELETE_BATCH),
            "memory transaction requires at most 4096 document changes"
        );
        let mut mutations = if inputs.is_empty() {
            Vec::new()
        } else {
            self.prepare_batch(inputs, &Default::default())?.0
        };
        mutations.extend(
            deleted
                .iter()
                .cloned()
                .map(|id| Mutation::DeleteDocument { id }),
        );
        if mutations.is_empty() {
            return Ok(self.memory_revision_matches(revision));
        }
        let payload = super::document_batch::serialize_transaction(self, &mutations)?;
        self.commit_serialized_if_revision(&mutations, &payload, revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_parts_retry_replacement_and_keep_embedding_with_its_text() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = crate::core::WarmConfig {
            embedding_space: Some(crate::EmbeddingSpace {
                model: "fixture".into(),
                revision: "1".into(),
                dimensions: 2,
                normalization: "l2".into(),
                metric: "cosine".into(),
            }),
            ..Default::default()
        };
        let core = WarmCore::new(
            256,
            Some(directory.path().display().to_string()),
            Some(config),
        )?;
        let input = |version: &str, embedding: [f64; 2]| {
            serde_json::from_value(serde_json::json!({
                "id": "doc", "version": version, "text": version, "embeddings": [embedding],
                "sourceUri": version, "metadata": {"version": version}
            }))
            .unwrap()
        };
        core.memorize_document(input("old evidence", [1.0, 0.0]))?;
        let mut calls = 0;
        let parts = core
            .document_parts_with("doc", || {
                calls += 1;
                assert!(core.documents.write().contains_key("doc"));
                if calls == 1 {
                    core.memorize_document(input("new evidence", [0.0, 1.0]))
                        .unwrap();
                }
            })?
            .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(parts.version, "new evidence");
        assert_eq!(parts.text.as_deref(), Some("new evidence"));
        assert_eq!(parts.source_uri.as_deref(), Some("new evidence"));
        assert_eq!(parts.metadata["version"], "new evidence");
        assert_eq!(parts.embedding, Some(vec![0.0, 1.0]));
        let deleted = core.document_parts_with("doc", || {
            core.delete_document("doc").unwrap();
        })?;
        assert!(deleted.is_none());
        core.memorize_document(input("initial evidence", [1.0, 0.0]))?;
        let mut calls = 0;
        let result = core.document_parts_with("doc", || {
            calls += 1;
            core.memorize_document(input(&format!("change {calls}"), [1.0, 0.0]))
                .unwrap();
        });
        assert_eq!(calls, 3);
        let error = result.err().expect("repeated replacement must fail");
        assert!(error.is::<documents::DocumentReadChanged>());
        Ok(())
    }

    #[test]
    fn metadata_scan_intersects_scope_and_checks_unindexed_values() -> Result<()> {
        use serde_json::json;
        use std::collections::BTreeSet;

        let directory = tempfile::tempdir()?;
        let core = WarmCore::new(256, Some(directory.path().display().to_string()), None)?;
        for (id, group, selected) in [
            ("a", 1, false),
            ("b", 1, true),
            ("c", 2, true),
            ("d", 2, false),
        ] {
            core.memorize_document(serde_json::from_value(json!({
                "id": id, "text": "evidence", "metadata": {"group": group, "nested": {"selected": selected}}
            }))?)?;
        }
        for (scope, filter, expected) in [
            (
                Some(BTreeSet::from(["a".into(), "b".into(), "c".into()])),
                json!({"group": 2}),
                vec!["c"],
            ),
            (
                Some(BTreeSet::from(["b".into()])),
                json!({"group": 1}),
                vec!["b"],
            ),
            (
                None,
                json!({"group": 2, "nested": {"selected": true}}),
                vec!["c"],
            ),
            (None, json!({"nested": {"selected": true}}), vec!["b", "c"]),
            (None, json!({"group": 3}), vec![]),
        ] {
            let mut visited = Vec::new();
            let revision = core.try_scan_documents(scope.as_ref(), Some(&filter), |document| {
                visited.push(document.id.to_owned());
                Ok(())
            })?;
            assert_eq!(revision, core.revision());
            assert_eq!(visited, expected);
        }
        Ok(())
    }

    #[test]
    fn fallible_scoped_scan_stops_at_the_first_error() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let core = WarmCore::new(256, Some(directory.path().display().to_string()), None)?;
        for id in ["a", "b", "c"] {
            core.memorize_document(serde_json::from_value(
                serde_json::json!({"id": id, "text": "evidence"}),
            )?)?;
        }
        let scope = std::collections::BTreeSet::from(["b".into(), "c".into()]);
        let mut visited = Vec::new();
        let result = core.try_scan_documents(Some(&scope), None, |document| {
            visited.push(document.id.to_owned());
            anyhow::bail!("budget exhausted")
        });
        assert!(result.is_err());
        assert_eq!(visited, ["b"]);
        Ok(())
    }

    #[test]
    fn metadata_scan_merges_checkpoint_and_overlay_without_hydration() -> Result<()> {
        use serde_json::json;
        use std::collections::BTreeSet;
        let directory = tempfile::tempdir()?;
        let mut config = crate::core::WarmConfig::default();
        config.document_checkpoint.min_journal_bytes = 0;
        let options = documents::IngestOptions {
            encode: documents::EncodeMode::Lexical,
            ..Default::default()
        };
        {
            let core = WarmCore::new(
                256,
                Some(directory.path().display().to_string()),
                Some(config.clone()),
            )?;
            core.memorize_documents_with(
                (0..257)
                    .map(|i| {
                        serde_json::from_value(json!({
                "id": format!("r/{i:03}"), "text": "evidence", "metadata": {"group": i % 2}
            })).unwrap()
                    })
                    .collect(),
                &options,
            )?;
        }
        let core = WarmCore::new(
            256,
            Some(directory.path().display().to_string()),
            Some(config),
        )?;
        assert!(core.serves_from_checkpoint());
        assert!(core.delete_document("r/000")?);
        for id in ["r/001", "z"] {
            core.memorize_document_with(
                serde_json::from_value(json!({
                    "id": id, "text": "new evidence", "metadata": {"group": 0}
                }))?,
                &options,
            )?;
        }
        let mut all = Vec::new();
        assert_eq!(
            core.try_scan_documents(None, None, |document| {
                all.push(document.id.to_owned());
                Ok(())
            })?,
            core.revision()
        );
        let expected: Vec<_> = (1..257)
            .map(|i| format!("r/{i:03}"))
            .chain(["z".into()])
            .collect();
        assert_eq!(all, expected);
        let filter = json!({"group": 0});
        for scope in [
            None,
            Some(BTreeSet::from(["r/001".into(), "r/003".into(), "z".into()])),
        ] {
            let mut ids = Vec::new();
            core.try_scan_documents(scope.as_ref(), Some(&filter), |document| {
                ids.push(document.id.to_owned());
                Ok(())
            })?;
            let expected: Vec<String> = if scope.is_some() {
                vec!["r/001".into(), "z".into()]
            } else {
                ["r/001".into()]
                    .into_iter()
                    .chain((2..257).step_by(2).map(|i| format!("r/{i:03}")))
                    .chain(["z".into()])
                    .collect()
            };
            assert_eq!(ids, expected);
        }
        assert!(core.serves_from_checkpoint());
        Ok(())
    }
}
