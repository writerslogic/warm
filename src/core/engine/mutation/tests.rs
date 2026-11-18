// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::core::{documents, EmbeddingSpace, WarmConfig};
use crate::{DocumentInput, SearchOptions};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn config(encrypted: bool) -> WarmConfig {
    let mut config = WarmConfig {
        embedding_space: Some(EmbeddingSpace {
            model: "publication-fixture".into(),
            revision: "1".into(),
            dimensions: 2,
            normalization: "l2".into(),
            metric: "cosine".into(),
        }),
        ..Default::default()
    };
    config.security.encryption_enabled = encrypted;
    if encrypted {
        config.security.encryption_passphrase = Some("publication-test-fixture".into());
    }
    config
}

fn modes() -> impl Iterator<Item = bool> {
    [false, true]
        .into_iter()
        .filter(|encrypted| !encrypted || cfg!(feature = "security"))
}

fn document(id: &str, version: &str) -> DocumentInput {
    serde_json::from_value(json!({
        "id":id,"text":format!("needle source {version}"),"version":version,
        "sourceUri":format!("source:{version}"),"metadata":{"revision":version},
        "embeddings":[if version == "old" { vec![1.0,0.0] } else { vec![0.0,1.0] }],
    }))
    .unwrap()
}

/// A batch prepared against one stored version of a document and installed after another
/// writer replaced that version must diff its stale chunks against the version actually
/// stored, or the replaced version's extra chunk vectors would survive as orphans.
#[test]
fn prepared_batch_rediffs_a_document_replaced_before_installation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let core = WarmCore::new(
        1024,
        Some(directory.path().display().to_string()),
        Some(config(false)),
    )?;
    core.memorize_documents(vec![document("a", "old")])?;
    let prepare = |input: DocumentInput| -> Result<Vec<Mutation>> {
        let stored = documents::prepare(
            input,
            core.dimensions,
            core.config.embedding_space.as_ref().map(|s| s.dimensions),
            core.config.text_encoder,
        )?;
        Ok(vec![Mutation::Document(Arc::new(stored))])
    };
    let short = prepare(document("a", "new"))?;
    let batch = core.prepare_mutations(&short);
    // Another writer replaces "a" with a two-chunk version (300 words at the default 256
    // words per chunk) in between.
    let long: DocumentInput = serde_json::from_value(json!({
        "id":"a","text":"needle ".repeat(300),"version":"long",
        "sourceUri":"source:long","metadata":{"revision":"long"},
        "embeddings":[vec![0.0,1.0], vec![1.0,0.0]],
    }))?;
    core.memorize_documents(vec![long])?;
    let stored = core.documents.read().get("a").cloned().expect("stored");
    assert_eq!(
        stored.chunks.len(),
        2,
        "the long version splits into two chunks"
    );
    assert_eq!(core.shards.read().count(), 2);
    {
        let _transaction = core.mutation_gate.write();
        let shards = core.shards.write();
        core.apply_mutations(&batch, &shards)?;
    }
    let stored = core.documents.read().get("a").cloned().expect("stored");
    assert_eq!(stored.version, "new");
    assert_eq!(stored.chunks.len(), 1);
    assert_eq!(
        core.shards.read().count(),
        1,
        "the replaced version's second chunk vector was removed"
    );
    assert!(
        core.apply_mutations(&batch, &core.shards.read()).is_err(),
        "a batch installs once"
    );
    Ok(())
}

/// A delete whose text scrub fails after the transaction is journaled still has to reach
/// the journaled state: the document's chunk vectors go, the rest of the batch applies and
/// the revision moves, while the scrub failure is reported.
#[test]
fn failed_scrub_still_applies_the_journaled_batch() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let core = WarmCore::new(
        1024,
        Some(directory.path().display().to_string()),
        Some(config(false)),
    )?;
    core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
    assert_eq!(core.shards.read().count(), 2);
    {
        // Point a's text past the end of the segment so scrubbing it fails.
        let mut store = core.documents.write();
        let mut broken = StoredDocument::clone(&store.get("a").cloned().expect("stored"));
        assert!(
            broken.chunks[0].text_at.is_some(),
            "text kept in the segment"
        );
        broken.chunks[0].text_at = Some((u64::MAX / 2, 4, 0));
        store.insert("a".into(), broken);
    }
    let revision = core.revision.load(std::sync::atomic::Ordering::Acquire);
    let deleted = core.commit(&[
        Mutation::DeleteDocument { id: "a".into() },
        Mutation::DeleteDocument { id: "b".into() },
    ]);
    assert!(deleted.is_err(), "the scrub failure is reported");
    assert!(core.documents.read().get("a").is_none());
    assert!(
        core.documents.read().get("b").is_none(),
        "the batch went on"
    );
    assert_eq!(core.shards.read().count(), 0, "no orphaned chunk vectors");
    assert_eq!(
        core.revision.load(std::sync::atomic::Ordering::Acquire),
        revision + 1
    );
    drop(core);
    let reopened = WarmCore::new(
        1024,
        Some(directory.path().display().to_string()),
        Some(config(false)),
    )?;
    assert!(reopened.documents.read().get("a").is_none());
    assert!(reopened.documents.read().get("b").is_none());
    assert_eq!(
        reopened.shards.read().count(),
        0,
        "replay reaches the same state"
    );
    Ok(())
}

fn replacements(core: &WarmCore) -> Result<Vec<Mutation>> {
    ["a", "b"]
        .into_iter()
        .map(|id| {
            documents::prepare(
                document(id, "new"),
                core.dimensions,
                Some(2),
                core.config.text_encoder,
            )
            .map(|document| Mutation::Document(Arc::new(document)))
        })
        .collect()
}

#[test]
fn retained_text_failure_precedes_commit_and_preserves_retry() -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
        core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
        core.documents
            .write()
            .make_text_read_only(&directory.path().join("documents.text"))?;
        let checkpoint = core.arena.checkpoint();
        let revision = core.revision();
        let mut mutations = replacements(&core)?;
        let tail = core.encode_text("later mutation stays uncommitted");
        mutations.push(Mutation::Vector {
            id: "tail".into(),
            vector: tail.clone(),
        });
        let error = core.commit(&mutations).unwrap_err();
        assert!(
            !error.is::<crate::CommittedMutationError>(),
            "retained-text preparation failed after commit: {error:#}"
        );
        assert!(format!("{error:#}").contains("appending chunk text"));
        assert_eq!(core.arena.checkpoint(), checkpoint);
        assert_eq!(core.revision(), revision);
        for id in ["a", "b"] {
            assert_eq!(core.get_document(id)?.unwrap().version, "old");
        }
        assert!(!core.query(&tail, 10)?.iter().any(|hit| hit.id == "tail"));
        drop(core);
        let reopened = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        for id in ["a", "b"] {
            assert_eq!(reopened.get_document(id)?.unwrap().version, "old");
        }
        reopened.commit(&mutations)?;
        for id in ["a", "b"] {
            assert_eq!(reopened.get_document(id)?.unwrap().version, "new");
        }
        assert!(reopened
            .query(&tail, 10)?
            .iter()
            .any(|hit| hit.id == "tail"));
    }
    Ok(())
}

#[test]
fn failed_text_preparation_preserves_mixed_batches_and_repeated_ids() -> Result<()> {
    for (encrypted, repeated) in
        modes().flat_map(|encrypted| [false, true].map(move |repeated| (encrypted, repeated)))
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
        core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
        let text_path = directory.path().join("documents.text");
        core.documents.write().make_text_read_only(&text_path)?;
        let revision = core.revision();
        let checkpoint = core.arena.checkpoint();
        let mut mutations = replacements(&core)?;
        if repeated {
            mutations.push(Mutation::Document(Arc::new(documents::prepare(
                document("a", "last"),
                core.dimensions,
                Some(2),
                core.config.text_encoder,
            )?)));
        }
        let tail = core.encode_text("tail mutation stays uncommitted");
        mutations.push(Mutation::Vector {
            id: "tail".into(),
            vector: tail.clone(),
        });
        mutations.push(Mutation::DeleteDocument { id: "b".into() });
        let error = core.commit(&mutations).unwrap_err();
        assert!(!error.is::<crate::CommittedMutationError>(), "{error:#}");
        assert_eq!(core.revision(), revision);
        assert_eq!(core.arena.checkpoint(), checkpoint);
        assert_eq!(core.get_document("a")?.unwrap().version, "old");
        assert_eq!(core.get_document("b")?.unwrap().version, "old");
        assert!(!core.query(&tail, 10)?.iter().any(|hit| hit.id == "tail"));
        core.documents.write().restore_text_writes(&text_path)?;
        core.commit(&mutations)?;
        assert_eq!(core.revision(), revision + 1);
        drop(core);
        let reopened = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        let version = if repeated { "last" } else { "new" };
        assert_eq!(reopened.get_document("a")?.unwrap().version, version);
        assert_eq!(
            reopened.get_document("a")?.unwrap().text.as_deref(),
            Some(format!("needle source {version}").as_str())
        );
        assert!(!reopened.has_document("b"));
        assert_eq!(reopened.shards.read().count(), 2);
        assert!(reopened
            .query(&tail, 10)?
            .iter()
            .any(|hit| hit.id == "tail"));
    }
    Ok(())
}

#[test]
fn rejected_journal_append_rolls_back_prepared_text_and_preserves_error() -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
        let text_path = directory.path().join("documents.text");
        let original = std::fs::read(&text_path)?;
        let revision = core.revision();
        let checkpoint = core.arena.checkpoint();
        let mutations = replacements(&core)?;
        let error = core
            .commit_with_append(&mutations, |_| {
                assert!(std::fs::metadata(&text_path)?.len() > original.len() as u64);
                Err(std::io::Error::other("injected journal failure").into())
            })
            .unwrap_err();
        assert!(error.is::<std::io::Error>(), "{error:#}");
        assert_eq!(std::fs::read(&text_path)?, original);
        assert_eq!(
            core.documents.read().text_segment().unwrap().len(),
            original.len() as u64
        );
        assert_eq!(core.revision(), revision);
        assert_eq!(core.arena.checkpoint(), checkpoint);
        for id in ["a", "b"] {
            assert_eq!(core.get_document(id)?.unwrap().version, "old");
        }
        core.commit(&mutations)?;
        for id in ["a", "b"] {
            assert_eq!(core.get_document(id)?.unwrap().version, "new");
        }
    }
    Ok(())
}

#[test]
fn failed_final_text_preparation_leaves_new_document_absent_after_reopen() -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
        core.memorize_document(document("a", "old"))?;
        core.documents
            .write()
            .make_text_read_only(&directory.path().join("documents.text"))?;
        let revision = core.revision();
        let error = core
            .memorize_documents(vec![document("a", "new"), document("b", "new")])
            .unwrap_err();
        assert!(!error.is::<crate::CommittedMutationError>(), "{error:#}");
        assert_eq!(core.revision(), revision);
        drop(core);
        let reopened = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        assert_eq!(reopened.get_document("a")?.unwrap().version, "old");
        assert!(!reopened.has_document("b"));
        reopened.memorize_documents(vec![document("a", "new"), document("b", "new")])?;
        for id in ["a", "b"] {
            assert_eq!(
                reopened.get_document(id)?.unwrap().text.as_deref(),
                Some("needle source new")
            );
        }
        assert_eq!(reopened.shards.read().count(), 2);
    }
    Ok(())
}

#[test]
fn failed_cleanup_hides_old_text_and_flush_retries_without_repeating_mutations() -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
        let text_path = directory.path().join("documents.text");
        core.documents.write().make_text_read_only(&text_path)?;
        let segment = core.documents.read().text_segment().unwrap();
        let old = core.documents.read().get("a").cloned().unwrap();
        let span = old.chunks[0].text_at.unwrap();
        let revision = core.revision();
        let tail = core.encode_text("tail committed once");
        let error = core
            .commit(&[
                Mutation::DeleteDocument { id: "a".into() },
                Mutation::Vector {
                    id: "tail".into(),
                    vector: tail.clone(),
                },
            ])
            .unwrap_err();
        assert!(error.is::<crate::CommittedMutationError>(), "{error:#}");
        assert_eq!(core.revision(), revision + 1);
        assert!(core.get_document("a")?.is_none());
        assert!(segment.is_scrubbed(span.0, span.1));
        assert_eq!(segment.read(span.0, span.1, span.2)?, "");
        assert!(core.query(&tail, 10)?.iter().any(|hit| hit.id == "tail"));
        assert!(core.flush().is_err());
        let blocked = core.memorize_document(document("c", "new")).unwrap_err();
        assert!(!blocked.is::<crate::CommittedMutationError>());
        assert_eq!(core.revision(), revision + 1);
        core.documents.write().restore_text_writes(&text_path)?;
        core.flush()?;
        core.flush()?;
        assert_eq!(core.revision(), revision + 1);
        let bytes = std::fs::read(&text_path)?;
        assert!(bytes[span.0 as usize..span.0 as usize + span.1 as usize]
            .iter()
            .all(|byte| *byte == 0));
        assert_eq!(
            core.get_document("b")?.unwrap().text.as_deref(),
            Some("needle source old")
        );
        core.memorize_document(document("c", "new"))?;
    }
    Ok(())
}

#[test]
fn failed_replacement_scrub_publishes_owners_and_reports_committed_state() -> Result<()> {
    let mut failures = Vec::new();
    for (encrypted, chunks) in
        modes().flat_map(|encrypted| [1, 2].map(|chunks| (encrypted, chunks)))
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
        core.memorize_document(document("a", "old"))?;
        core.documents
            .write()
            .make_text_read_only(&directory.path().join("documents.text"))?;
        let mut mutations = Vec::new();
        for id in ["a", "b"] {
            let replacement = documents::prepare(
                serde_json::from_value(json!({
                    "id": id, "version": "new", "text": "replacement alpha beta gamma",
                    "storeText": false, "chunkWords": 4 / chunks, "overlapWords": 0,
                    "embeddings": vec![vec![0.0, 1.0]; chunks],
                }))?,
                core.dimensions,
                Some(2),
                core.config.text_encoder,
            )?;
            assert_eq!(replacement.chunks.len(), chunks);
            assert!(replacement
                .chunks
                .iter()
                .all(|chunk| chunk.text.is_none() && chunk.text_at.is_none()));
            mutations.push(Mutation::Document(Arc::new(replacement)));
        }
        let tail = core.encode_text("later mutation");
        mutations.push(Mutation::Vector {
            id: "tail".into(),
            vector: tail.clone(),
        });
        let revision = core.revision();
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.commit(&mutations)));
        let published = match outcome {
            Ok(Err(error)) => {
                assert!(error.is::<crate::CommittedMutationError>(), "{error:#}");
                assert!(format!("{error:#}").contains("scrubbing chunk text"));
                true
            }
            Ok(Ok(())) => {
                failures.push(format!(
                    "encrypted={encrypted}, chunks={chunks}: scrub failure was hidden"
                ));
                false
            }
            Err(_) => {
                failures.push(format!(
                    "encrypted={encrypted}, chunks={chunks}: replacement scrub panicked"
                ));
                false
            }
        };
        if core.revision() != revision + 1 {
            failures.push(format!(
                "encrypted={encrypted}, chunks={chunks}: durable revision stayed stale"
            ));
        }
        let live = if published {
            for id in ["a", "b"] {
                let record = core.get_document(id)?.unwrap();
                assert_eq!(record.version, "new");
                assert!(record.text.is_none());
            }
            assert_eq!(core.document_count(), 2);
            assert_eq!(core.shards.read().count(), (2 * chunks + 1) as u64);
            assert!(core.query(&tail, 10)?.iter().any(|hit| hit.id == "tail"));
            Some(serde_json::to_value(
                core.search_documents("replacement", &Default::default())?,
            )?)
        } else {
            None
        };
        drop(core);
        let reopened = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        for id in ["a", "b"] {
            let record = reopened.get_document(id)?.unwrap();
            assert_eq!(record.version, "new");
            assert!(record.text.is_none());
        }
        assert_eq!(reopened.document_count(), 2);
        assert_eq!(reopened.shards.read().count(), (2 * chunks + 1) as u64);
        assert!(reopened
            .query(&tail, 10)?
            .iter()
            .any(|hit| hit.id == "tail"));
        if let Some(live) = live {
            assert_eq!(
                serde_json::to_value(
                    reopened.search_documents("replacement", &Default::default())?
                )?,
                live
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn shared_documents_preserve_legacy_wire_bytes_and_owner_lifetime() -> Result<()> {
    #[derive(Serialize)]
    enum Legacy<'a> {
        Document(&'a documents::StoredDocument),
    }
    let directory = tempfile::tempdir()?;
    let core = WarmCore::new(
        1024,
        Some(directory.path().display().to_string()),
        Some(config(false)),
    )?;
    let mutations = replacements(&core)?;
    let legacy = mutations
        .iter()
        .map(|mutation| match mutation {
            Mutation::Document(document) => Legacy::Document(document),
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&legacy)?;
    assert_eq!(serde_json::to_vec(&mutations)?, bytes);
    {
        let mut omitted = serde_json::to_value(&legacy)?;
        let document = omitted[0]["Document"].as_object_mut().unwrap();
        document.remove("source_uri");
        let chunk = document["chunks"][0].as_object_mut().unwrap();
        chunk.remove("embedding");
        chunk.remove("text");
        chunk.remove("stemmed");
        let decoded: Vec<Mutation> = serde_json::from_value(omitted)?;
        let Mutation::Document(legacy_document) = &decoded[0] else {
            unreachable!()
        };
        assert!(legacy_document.source_uri.is_none());
        assert!(legacy_document.chunks[0].embedding.is_none());
        assert!(legacy_document.chunks[0].text.is_none());
        assert!(!legacy_document.chunks[0].stemmed);
    }
    let restored: Vec<Mutation> = serde_json::from_slice(&bytes)?;
    assert_eq!(serde_json::to_vec(&restored)?, bytes);
    core.commit(&restored)?;
    for mutation in &restored {
        let Mutation::Document(document) = mutation else {
            unreachable!()
        };
        // The store installs the committed document with its chunk text moved to the
        // text segment; serialized with the text resolved it is byte-identical to the
        // committed one.
        let store = core.documents.read();
        let stored = store.get(&document.id).unwrap();
        assert!(stored
            .chunks
            .iter()
            .all(|c| c.text.is_none() && c.text_at.is_some()));
        let segment = store.text_segment();
        assert_eq!(
            serde_json::to_vec(&crate::core::documents::DocumentWithText {
                document: stored,
                segment: segment.as_deref(),
            })?,
            serde_json::to_vec(document.as_ref())?
        );
    }
    drop(restored);
    let before = current(&core, "new")?;
    assert_eq!(before.as_array().unwrap().len(), 2);
    core.memorize_document(document("a", "old"))?;
    let after = core.search_documents(
        "needle",
        &SearchOptions {
            source_uri: Some("source:new".into()),
            filter: Some(json!({"revision":"new"})),
            embedding: Some(vec![0.0, 1.0]),
            ..Default::default()
        },
    )?;
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].document_id, "b");
    assert_eq!(after[0].version, "new");
    assert_eq!(after[0].source_uri.as_deref(), Some("source:new"));
    assert_eq!(after[0].text.as_deref(), Some("needle source new"));
    assert_eq!(after[0].semantic_score, Some(1.0));
    assert_eq!(serde_json::to_vec(&mutations)?, bytes);
    Ok(())
}

#[test]
fn owned_document_buffers_stay_compact_through_publication_and_recovery() -> Result<()> {
    fn reserved(value: &str) -> String {
        let mut result = String::with_capacity(64);
        result.push_str(value);
        result
    }
    fn metadata(value: &serde_json::Value) {
        match value {
            serde_json::Value::Array(values) => {
                assert_eq!(values.capacity(), values.len());
                for value in values {
                    metadata(value);
                }
            }
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    assert_eq!(key.capacity(), key.len());
                    metadata(value);
                }
            }
            serde_json::Value::String(value) => assert_eq!(value.capacity(), value.len()),
            _ => {}
        }
    }
    fn compact(document: &documents::StoredDocument) {
        assert_eq!(document.id.capacity(), document.id.len());
        assert_eq!(document.version.capacity(), document.version.len());
        if let Some(uri) = &document.source_uri {
            assert_eq!(uri.capacity(), uri.len());
        }
        metadata(&document.metadata);
        assert_eq!(document.chunks.capacity(), document.chunks.len());
        for chunk in &document.chunks {
            assert_eq!(chunk.id.capacity(), chunk.id.len());
            if let Some(text) = &chunk.text {
                assert_eq!(text.capacity(), text.len());
            }
            assert_eq!(chunk.vector.indices.capacity(), chunk.vector.indices.len());
            chunk.terms.assert_valid_storage();
        }
    }
    for encrypted in modes() {
        for (vsa, dense, count) in [
            (1024, 127, 3),
            (1024, 128, 3),
            (1024, 129, 3),
            (1536, 128, 3),
            (1024, 128, 4),
            (1024, 128, 5),
        ] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().display().to_string();
            let mut config = config(encrypted);
            config.embedding_space.as_mut().unwrap().dimensions = dense;
            let core = WarmCore::new(vsa, Some(path.clone()), Some(config.clone()))?;
            let mut tags = Vec::with_capacity(64);
            tags.extend((0..count).map(|value| json!(value)));
            let mut values = serde_json::Map::new();
            values.insert(reserved("tags"), serde_json::Value::Array(tags));
            values.insert(
                reserved("empty"),
                serde_json::Value::Array(Vec::with_capacity(64)),
            );
            values.insert(
                reserved("label"),
                serde_json::Value::String(reserved("µ😀")),
            );
            let mut embedding = vec![0.0; dense];
            embedding[0] = 1.0;
            let input = DocumentInput {
                id: reserved("buffers"),
                text: "needle source".into(),
                source_uri: Some(reserved("source:buffers")),
                version: Some(reserved("1")),
                metadata: Some(serde_json::Value::Object(values)),
                embeddings: Some(vec![embedding.clone()]),
                chunk_words: Some(4096),
                overlap_words: Some(0),
                store_text: Some(true),
                title_weight: None,
            };
            let mut replacement = input.clone();
            replacement.version = Some(reserved("2"));
            core.memorize_document(input)?;
            let held = Arc::clone(core.documents.read().get("buffers").unwrap());
            compact(&held);
            let original_wire = serde_json::to_vec(&held)?;
            core.memorize_document(replacement)?;
            assert_eq!(serde_json::to_vec(&held)?, original_wire);
            let wire = {
                let documents = core.documents.read();
                let document = documents.get("buffers").unwrap();
                compact(document);
                serde_json::to_vec(document)?
            };
            let options = SearchOptions {
                k: Some(1),
                candidate_limit: Some(1),
                source_uri: Some("source:buffers".into()),
                embedding: Some(embedding),
                ..Default::default()
            };
            let before = serde_json::to_value(core.search_documents("needle", &options)?)?;
            assert_eq!(before.as_array().unwrap().len(), 1);
            core.flush()?;
            drop(core);
            let core = WarmCore::new(vsa, Some(path.clone()), Some(config.clone()))?;
            {
                let documents = core.documents.read();
                let document = documents.get("buffers").unwrap();
                compact(document);
                assert_eq!(serde_json::to_vec(document)?, wire);
            }
            assert_eq!(
                serde_json::to_value(core.search_documents("needle", &options)?)?,
                before
            );
            core.compact()?;
            drop(core);
            let core = WarmCore::new(vsa, Some(path), Some(config))?;
            compact(core.documents.read().get("buffers").unwrap());
            assert_eq!(
                serde_json::to_value(core.search_documents("needle", &options)?)?,
                before
            );
        }
    }
    Ok(())
}

#[test]
fn journal_replay_keeps_length_sized_chunk_storage() -> Result<()> {
    for encrypted in modes() {
        for count in [3, 4, 5] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().display().to_string();
            let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
            let input: DocumentInput = serde_json::from_value(json!({
                "id":"capacity", "version":"1", "sourceUri":"source:capacity",
                "text": (0..count).map(|i| format!("needle term{i}")).collect::<Vec<_>>().join(" "),
                "metadata":{"chunks":count}, "chunkWords":2, "overlapWords":0,
                "embeddings":vec![vec![1.0,0.0]; count],
            }))?;
            core.memorize_document(input)?;
            let options = SearchOptions {
                k: Some(10),
                candidate_limit: Some(10),
                source_uri: Some("source:capacity".into()),
                filter: Some(json!({"chunks":count})),
                embedding: Some(vec![1.0, 0.0]),
                ..Default::default()
            };
            let before = serde_json::to_value(core.search_documents("needle", &options)?)?;
            assert_eq!(before.as_array().unwrap().len(), count);
            let wire = serde_json::to_vec(core.documents.read().get("capacity").unwrap())?;
            core.flush()?;
            drop(core);
            let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
            {
                let documents = core.documents.read();
                let document = documents.get("capacity").unwrap();
                assert_eq!(document.chunks.len(), count);
                assert_eq!(document.chunks.capacity(), count);
                assert_eq!(serde_json::to_vec(document)?, wire);
            }
            assert_eq!(
                serde_json::to_value(core.search_documents("needle", &options)?)?,
                before
            );
            core.compact()?;
            drop(core);
            let core = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
            assert_eq!(
                core.documents
                    .read()
                    .get("capacity")
                    .unwrap()
                    .chunks
                    .capacity(),
                count
            );
            assert_eq!(
                serde_json::to_value(core.search_documents("needle", &options)?)?,
                before
            );
        }
    }
    Ok(())
}

#[test]
fn replacement_chunk_transitions_remove_obsolete_vectors_and_replay_exactly() -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
        for chunks in [2, 1, 2, 3, 2] {
            let text = ["needle alpha", "needle beta", "needle gamma"][..chunks].join(" ");
            let input: DocumentInput = serde_json::from_value(json!({
                "id":"changing", "text": text,
                "version": chunks.to_string(), "sourceUri":format!("source:{chunks}"),
                "metadata":{"chunks":chunks}, "chunkWords":2,"overlapWords":0,
                "embeddings":vec![vec![1.0,0.0]; chunks],
            }))?;
            core.memorize_document(input)?;
            let documents = core.documents.read();
            assert_eq!(documents.get("changing").unwrap().chunks.len(), chunks);
            let shards = core.shards.read();
            for chunk in 0..3 {
                let id = format!("hms:chunk:8:changing:{chunk}");
                assert_eq!(shards.get_vector(&id).is_some(), chunk < chunks);
            }
        }
        let options = SearchOptions {
            embedding: Some(vec![1.0, 0.0]),
            k: Some(10),
            ..Default::default()
        };
        let before = serde_json::to_value(core.search_documents("needle", &options)?)?;
        core.flush()?;
        drop(core);
        let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
        assert_eq!(
            serde_json::to_value(core.search_documents("needle", &options)?)?,
            before
        );
        assert!(core
            .shards
            .read()
            .get_vector("hms:chunk:8:changing:2")
            .is_none());
        core.compact()?;
        drop(core);
        let core = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        assert_eq!(
            serde_json::to_value(core.search_documents("needle", &options)?)?,
            before
        );
        assert!(core.delete_document("changing")?);
        assert!(core.search_documents("needle", &options)?.is_empty());
    }
    Ok(())
}

fn current(core: &WarmCore, version: &str) -> Result<serde_json::Value> {
    let found = core.search_documents(
        "needle",
        &SearchOptions {
            source_uri: Some(format!("source:{version}")),
            filter: Some(json!({"revision":version})),
            embedding: Some(if version == "old" {
                vec![1.0, 0.0]
            } else {
                vec![0.0, 1.0]
            }),
            ..Default::default()
        },
    )?;
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|row| row.version == version
        && row.text.as_deref() == Some(&format!("needle source {version}"))
        && row.semantic_score == Some(1.0)));
    Ok(serde_json::to_value(found)?)
}

fn reader_progress(after_append: bool) -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = Arc::new(WarmCore::new(
            1024,
            Some(path.clone()),
            Some(config(encrypted)),
        )?);
        core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
        let anchor = core.encode_text("anchor");
        core.memorize("anchor".into(), anchor.clone())?;
        let mutations = replacements(&core)?;
        let checkpoint = core.arena.checkpoint();
        let (paused_tx, paused_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let writer_core = Arc::clone(&core);
        let writer = std::thread::spawn(move || {
            writer_core.commit_with_append(&mutations, |payload| {
                if after_append {
                    writer_core.arena_write(payload)?;
                }
                paused_tx.send(())?;
                resume_rx.recv_timeout(Duration::from_secs(15))?;
                if !after_append {
                    writer_core.arena_write(payload)?;
                }
                Ok(())
            })
        });
        paused_rx.recv_timeout(Duration::from_secs(10))?;
        if after_append {
            let (payload, _) = core.arena_read_frame(checkpoint.1)?;
            let decoded = crate::core::engine::journal::decode(&payload)?;
            assert_eq!(decoded.len(), 2);
            assert!(decoded.iter().all(|mutation| matches!(mutation,
                Mutation::Document(document) if document.version == "new")));
        } else {
            assert_eq!(core.arena.checkpoint(), checkpoint);
        }
        let (document_tx, document_rx) = mpsc::channel();
        let reader_core = Arc::clone(&core);
        let reader = std::thread::spawn(move || {
            document_tx.send(reader_core.search_documents(
                "needle",
                &SearchOptions {
                    embedding: Some(vec![1.0, 0.0]),
                    ..Default::default()
                },
            ))
        });
        let (vector_tx, vector_rx) = mpsc::channel();
        let reader_core = Arc::clone(&core);
        let query_anchor = anchor.clone();
        let vector_reader = std::thread::spawn(move || {
            vector_tx.send(reader_core.query(&query_anchor, 10).unwrap())
        });
        let (queued_tx, queued_rx) = mpsc::channel();
        let (committed_tx, committed_rx) = mpsc::channel();
        let second_core = Arc::clone(&core);
        let second = std::thread::spawn(move || -> Result<()> {
            queued_tx.send(())?;
            second_core.memorize("second".into(), anchor)?;
            committed_tx.send(())?;
            Ok(())
        });
        queued_rx.recv_timeout(Duration::from_secs(5))?;
        let (compacted_tx, compacted_rx) = mpsc::channel();
        let compact_core = Arc::clone(&core);
        let compactor = std::thread::spawn(move || -> Result<()> {
            compact_core.compact()?;
            compacted_tx.send(())?;
            Ok(())
        });
        let documents = document_rx.recv_timeout(Duration::from_secs(2));
        let vectors = vector_rx.recv_timeout(Duration::from_secs(2));
        let writer_waited = committed_rx.try_recv().is_err();
        let compactor_waited = compacted_rx.try_recv().is_err();
        resume_tx.send(())?;
        writer.join().unwrap()?;
        second.join().unwrap()?;
        compactor.join().unwrap()?;
        reader.join().unwrap().unwrap();
        vector_reader.join().unwrap().unwrap();
        assert!(
            writer_waited && compactor_waited,
            "publication exclusion was lost"
        );
        let documents = documents.expect("durable append blocked document queries")?;
        assert_eq!(documents.len(), 2);
        assert!(documents.iter().all(|row| row.version == "old"
            && row.text.as_deref() == Some("needle source old")
            && row.source_uri.as_deref() == Some("source:old")
            && row.metadata == json!({"revision":"old"})
            && row.semantic_score == Some(1.0)));
        assert!(vectors
            .expect("durable append blocked vector queries")
            .iter()
            .any(|row| row.id == "anchor"));
        let before = current(&core, "new")?;
        drop(core);
        let restored = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        assert_eq!(current(&restored, "new")?, before);
        assert!(restored.shards.read().get_vector("second").is_some());
    }
    Ok(())
}

#[test]
fn pending_append_allows_coherent_queries_and_excludes_other_writers() -> Result<()> {
    reader_progress(false)
}

#[test]
fn durable_unpublished_batch_allows_old_queries_and_blocks_compaction() -> Result<()> {
    reader_progress(true)
}

#[test]
fn append_failure_preserves_live_state_and_allows_retry() -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
        core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
        let before = current(&core, "old")?;
        let checkpoint = core.arena.checkpoint();
        let mutations = replacements(&core)?;
        let expected = core.transaction_payload(&mutations)?;
        let revision = core.revision.load(std::sync::atomic::Ordering::Acquire);
        assert!(core
            .commit_with_append(&mutations, |payload| {
                assert_eq!(payload, expected);
                anyhow::bail!("injected append failure")
            })
            .is_err());
        assert_eq!(core.arena.checkpoint(), checkpoint);
        assert_eq!(
            core.revision.load(std::sync::atomic::Ordering::Acquire),
            revision
        );
        assert_eq!(current(&core, "old")?, before);
        core.commit_with_append(&mutations, |payload| {
            assert_eq!(payload, expected);
            core.arena_write(payload).map(|_| ())
        })?;
        assert_eq!(core.arena_read_frame(checkpoint.1)?.0, expected);
        let before = current(&core, "new")?;
        drop(core);
        let restored = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
        assert_eq!(current(&restored, "new")?, before);
    }
    Ok(())
}

#[test]
fn complete_transaction_bytes_match_legacy_for_all_mutations() -> Result<()> {
    for encrypted in modes() {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let mut configuration = config(encrypted);
        configuration.meaning.enabled = true;
        let core = WarmCore::new(1024, Some(path.clone()), Some(configuration.clone()))?;
        let vector = core.encode_text("needle café \"\\\n");
        let mut input = document("文書\"\\\n", "new");
        input.metadata = Some(json!({
            "escaped\"\\\n": [null, true, false, "é\u{0001}", [], {}],
            "floats": [-0.0, f64::MIN_POSITIVE, f64::MAX, 1.2345678901234567],
            "integers": [i64::MIN, u64::MAX],
        }));
        let document = Arc::new(documents::prepare(
            input,
            core.dimensions,
            Some(2),
            core.config.text_encoder,
        )?);
        let mutations = vec![
            Mutation::Vector {
                id: "vector".into(),
                vector: vector.clone(),
            },
            Mutation::Atom {
                id: "atom".into(),
                vector: vector.clone(),
            },
            Mutation::Composite {
                id: "composite".into(),
                vector,
            },
            Mutation::StoredTriple(crate::core::triple_store::TripleRecord {
                subject_id: "s".into(),
                relation_id: "r".into(),
                object_id: "o".into(),
                composite_id: "composite".into(),
                deleted: false,
            }),
            Mutation::Document(Arc::clone(&document)),
            Mutation::Derived {
                id: "derived".into(),
                children: vec!["hms:meaning:7:derived:child".into()],
            },
            Mutation::DeleteDocument {
                id: "absent-document".into(),
            },
            Mutation::Delete {
                id: "absent-vector".into(),
            },
            Mutation::DeleteAtom {
                id: "absent-atom".into(),
            },
            Mutation::DeleteComposite {
                id: "absent-composite".into(),
            },
            Mutation::RefreshMeaningIndices {},
            Mutation::Triplet {
                id: "triple".into(),
                subject: "s".into(),
                relation: "r".into(),
                object: "o".into(),
            },
            Mutation::Relation(Relation {
                source_id: "s".into(),
                relation_type: "r".into(),
                target_id: "o".into(),
                properties: Some("\"\\\n文書".into()),
                valid_from: f64::MIN_POSITIVE,
                valid_to: f64::MAX,
            }),
            Mutation::RemoveRelation {
                source: "absent".into(),
                relation: "r".into(),
                target: "o".into(),
            },
            Mutation::RelationType(RelationType {
                name: "r".into(),
                transitive: true,
                symmetric: false,
            }),
            Mutation::Rule(CompositionRule {
                name: "rule".into(),
                input_relations: vec!["r".into(), "r".into()],
                output_relation: "rr".into(),
            }),
        ];
        let expected = core.transaction_payload(&mutations)?;
        let checkpoint = core.arena.checkpoint();
        core.commit_with_append(&mutations, |payload| {
            assert_eq!(payload, expected);
            core.arena_write(payload).map(|_| ())
        })?;
        assert_eq!(core.arena_read_frame(checkpoint.1)?.0, expected);
        let options = SearchOptions {
            embedding: Some(vec![0.0, 1.0]),
            ..Default::default()
        };
        let before = serde_json::to_value(core.search_documents("needle", &options)?)?;
        assert_eq!(before.as_array().unwrap().len(), 1);
        drop(core);
        let core = WarmCore::new(1024, Some(path.clone()), Some(configuration.clone()))?;
        assert_eq!(core.arena_read_frame(checkpoint.1)?.0, expected);
        assert_eq!(
            serde_json::to_value(core.search_documents("needle", &options)?)?,
            before
        );
        core.compact()?;
        drop(core);
        let core = WarmCore::new(1024, Some(path), Some(configuration))?;
        assert_eq!(
            serde_json::to_value(core.search_documents("needle", &options)?)?,
            before
        );
        // The payload, in either journal format, reads back as these mutations.
        assert_eq!(
            serde_json::to_vec(&mutations)?,
            &*crate::core::engine::journal::payload_json(&expected).unwrap()?
        );
    }
    Ok(())
}

#[test]
fn transaction_limits_and_validation_precede_append() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let core = WarmCore::new(1024, Some(directory.path().display().to_string()), None)?;
    let checkpoint = core.arena.checkpoint();
    let revision = core.revision.load(std::sync::atomic::Ordering::Acquire);
    for count in [0, 1, 4095, 4096, 4097] {
        let mutations = vec![
            Mutation::Delete {
                id: "absent\"\\\n文書".into()
            };
            count
        ];
        let expected = core.transaction_payload(&mutations)?;
        let called = std::cell::Cell::new(false);
        let error = core
            .commit_with_append(&mutations, |payload| {
                called.set(true);
                assert_eq!(payload, expected);
                anyhow::bail!("injected append failure")
            })
            .unwrap_err();
        assert_eq!(called.get(), (1..=4096).contains(&count));
        assert!(error.to_string().contains(if called.get() {
            "injected append failure"
        } else {
            "transaction requires"
        }));
        assert_eq!(core.arena.checkpoint(), checkpoint);
        assert_eq!(
            core.revision.load(std::sync::atomic::Ordering::Acquire),
            revision
        );
    }
    let invalid = [
        Mutation::Delete { id: "valid".into() },
        Mutation::Delete { id: String::new() },
    ];
    let error = core
        .commit_with_append(&invalid, |_| panic!("invalid transaction reached append"))
        .unwrap_err();
    assert!(error.to_string().contains("IDs must"));
    assert_eq!(core.arena.checkpoint(), checkpoint);
    Ok(())
}

#[test]
fn publication_exit_child() -> Result<()> {
    let Some(path) = crate::compatibility::var_os("WARM_MUTATION_PUBLICATION_TEST_PATH") else {
        return Ok(());
    };
    let phase: usize =
        crate::compatibility::var("WARM_MUTATION_PUBLICATION_TEST_PHASE")?.parse()?;
    ensure!(phase <= 2, "invalid publication interruption phase");
    let encrypted =
        crate::compatibility::var("WARM_MUTATION_PUBLICATION_TEST_ENCRYPTED")? == "true";
    let core = WarmCore::new(
        1024,
        Some(PathBuf::from(path).display().to_string()),
        Some(config(encrypted)),
    )?;
    core.commit_with_append(&replacements(&core)?, |payload| {
        if phase == 0 {
            std::process::exit(77);
        }
        core.arena_write(payload)?;
        if phase == 1 {
            std::process::exit(78);
        }
        Ok(())
    })?;
    std::process::exit(79);
}

#[test]
fn interrupted_append_and_publication_recover_whole_batches() -> Result<()> {
    for encrypted in modes() {
        for phase in 0..=2 {
            let directory = tempfile::tempdir()?;
            let path = directory.path().display().to_string();
            let core = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
            core.memorize_documents(vec![document("a", "old"), document("b", "old")])?;
            drop(core);
            let child = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "core::engine::mutation::tests::publication_exit_child",
                    "--nocapture",
                ])
                .env("WARM_MUTATION_PUBLICATION_TEST_PATH", &path)
                .env("WARM_MUTATION_PUBLICATION_TEST_PHASE", phase.to_string())
                .env(
                    "WARM_MUTATION_PUBLICATION_TEST_ENCRYPTED",
                    encrypted.to_string(),
                )
                .output()?;
            assert_eq!(
                child.status.code(),
                Some(77 + phase),
                "{}",
                String::from_utf8_lossy(&child.stderr)
            );
            let restored = WarmCore::new(1024, Some(path.clone()), Some(config(encrypted)))?;
            let version = if phase == 0 { "old" } else { "new" };
            let before = current(&restored, version)?;
            assert!(restored
                .search_documents(
                    "needle",
                    &SearchOptions {
                        source_uri: Some(format!(
                            "source:{}",
                            if version == "old" { "new" } else { "old" }
                        )),
                        ..Default::default()
                    }
                )?
                .is_empty());
            restored.compact()?;
            drop(restored);
            let restored = WarmCore::new(1024, Some(path), Some(config(encrypted)))?;
            assert_eq!(current(&restored, version)?, before);
        }
    }
    Ok(())
}
