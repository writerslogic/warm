use std::sync::Arc;

use super::index::Index;
use super::*;
use crate::core::WarmCore;

fn fact(id: &str, text: &str) -> RememberInput {
    RememberInput {
        id: Some(id.into()),
        text: text.into(),
        ..RememberInput::default()
    }
}

#[test]
fn fallible_memory_reads_report_damaged_retained_text_and_recover() {
    type MemoryRead = fn(&Memory) -> Result<()>;
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let memory = Memory::new(core);
    let text = "retained memory evidence";
    memory.remember(fact("doc", text)).unwrap();
    let path = dir.path().join("documents.text");
    let original = std::fs::read(&path).unwrap();
    let at = original
        .windows(text.len())
        .position(|bytes| bytes == text.as_bytes())
        .unwrap();
    let operations: [(&str, MemoryRead); 4] = [
        ("get", |memory| memory.get("doc").map(drop)),
        ("history", |memory| memory.history("doc").map(drop)),
        ("explain", |memory| {
            memory
                .explain("retained", "doc", &RecallOptions::default())
                .map(drop)
        }),
        ("summaries", |memory| memory.try_summaries(true).map(drop)),
    ];
    let mut failures = Vec::new();
    for truncate in [false, true] {
        let mut damaged = original.clone();
        if truncate {
            damaged.pop();
        } else {
            damaged[at] ^= 1;
        }
        std::fs::write(&path, &damaged).unwrap();
        for (name, operation) in operations {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&memory))) {
                Ok(Err(error)) => {
                    let message = format!("{error:#}");
                    assert!(
                        message.contains(if truncate {
                            "reading chunk text"
                        } else {
                            "checksum mismatch"
                        }),
                        "{message}"
                    );
                }
                Ok(Ok(())) => failures.push(format!(
                    "{name}, truncate={truncate}: succeeded with damaged text"
                )),
                Err(_) => failures.push(format!("{name}, truncate={truncate}: panicked")),
            }
        }
        std::fs::write(&path, &original).unwrap();
        assert_eq!(
            memory.get("doc").unwrap().unwrap().text.as_deref(),
            Some(text)
        );
        assert!(memory.get("absent").unwrap().is_none());
        for (_, operation) in operations {
            operation(&memory).unwrap();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn damaged_text_does_not_block_metadata_only_memory_operations() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let memory = Memory::new(core.clone());
    let text = "metadata operations survive text damage";
    memory.remember(fact("doc", text)).unwrap();
    let path = dir.path().join("documents.text");
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes
        .windows(text.len())
        .position(|bytes| bytes == text.as_bytes())
        .unwrap();
    bytes[at] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(memory.get("doc").is_err());
    let revision = core.revision();
    let summaries = memory.try_summaries(false).unwrap();
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].text.is_none());
    assert!(!memory.remember(fact("doc", text)).unwrap().created);
    let scope = ForgetScope::Ids {
        ids: vec!["doc".into()],
    };
    assert_eq!(
        memory
            .forget(
                &scope,
                &ForgetOptions {
                    dry_run: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .forgotten,
        ["doc"]
    );
    assert_eq!(core.revision(), revision);
    assert_eq!(
        memory
            .forget(
                &scope,
                &ForgetOptions {
                    purge: false,
                    ..ForgetOptions::default()
                }
            )
            .unwrap()
            .forgotten,
        ["doc"]
    );
    assert!(memory.get("doc").unwrap().is_none());
    let scrubbed = std::fs::read(&path).unwrap();
    assert!(scrubbed[at..at + text.len()].iter().all(|byte| *byte == 0));
}

#[test]
fn failed_survivor_read_keeps_forgetting_atomic_and_repair_allows_retry() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let memory = Memory::new(core.clone());
    memory.remember(fact("a", "first evidence")).unwrap();
    memory
        .remember(RememberInput {
            supersedes: Some(vec!["a".into()]),
            ..fact("b", "middle evidence")
        })
        .unwrap();
    let survivor_text = "surviving evidence";
    memory
        .remember(RememberInput {
            supersedes: Some(vec!["b".into()]),
            ..fact("c", survivor_text)
        })
        .unwrap();
    let path = dir.path().join("documents.text");
    let original = std::fs::read(&path).unwrap();
    let at = original
        .windows(survivor_text.len())
        .position(|bytes| bytes == survivor_text.as_bytes())
        .unwrap();
    let mut damaged = original.clone();
    damaged[at] ^= 1;
    std::fs::write(&path, damaged).unwrap();
    let revision = core.revision();
    let scope = ForgetScope::Ids {
        ids: vec!["b".into()],
    };
    let result = memory.forget(&scope, &ForgetOptions::default());
    std::fs::write(&path, original).unwrap();
    assert!(format!("{:#}", result.unwrap_err()).contains("checksum mismatch"));
    assert_eq!(core.revision(), revision);
    assert_eq!(core.document_count(), 3);
    assert_eq!(memory.get("c").unwrap().unwrap().supersedes, ["b"]);
    assert_eq!(
        memory
            .forget(&scope, &ForgetOptions::default())
            .unwrap()
            .forgotten,
        ["b"]
    );
    let survivor = memory.get("c").unwrap().unwrap();
    assert_eq!(survivor.text.as_deref(), Some(survivor_text));
    assert_eq!(survivor.supersedes, ["a"]);
    assert!(memory.get("b").unwrap().is_none());
    assert_eq!(
        memory.get("a").unwrap().unwrap().status,
        FactStatus::Superseded
    );
}

#[test]
fn concurrent_wrappers_cannot_both_create_different_facts_with_one_id() {
    struct Synchronized(std::sync::Barrier);
    impl SupersessionPolicy for Synchronized {
        fn resolve(&self, _: &FactView, _: &[FactView]) -> Resolution {
            self.0.wait();
            Resolution::default()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let policy = Arc::new(Synchronized(std::sync::Barrier::new(2)));
    let workers: Vec<_> = ["first fact", "second fact"]
        .into_iter()
        .map(|text| {
            let memory = Memory::new(core.clone()).with_supersession(policy.clone());
            std::thread::spawn(move || {
                memory.remember(RememberInput {
                    subject: Some("slot".into()),
                    ..fact("shared", text)
                })
            })
        })
        .collect();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_ref().is_err_and(|e| matches!(
                e.downcast_ref::<MemoryError>(),
                Some(MemoryError::Conflict(_))
            )))
            .count(),
        1
    );
    assert_eq!(core.document_count(), 1);
}

#[test]
fn concurrent_subject_updates_replan_against_the_committed_predecessor() {
    struct Synchronized {
        barrier: std::sync::Barrier,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl SupersessionPolicy for Synchronized {
        fn resolve(&self, incoming: &FactView, group: &[FactView]) -> Resolution {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                self.barrier.wait();
            }
            LatestBySubject.resolve(incoming, group)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let policy = Arc::new(Synchronized {
        barrier: std::sync::Barrier::new(2),
        calls: Default::default(),
    });
    let workers: Vec<_> = [10, 20]
        .into_iter()
        .map(|time| {
            let memory = Memory::new(core.clone()).with_supersession(policy.clone());
            std::thread::spawn(move || {
                memory.remember(RememberInput {
                    subject: Some("slot".into()),
                    event_time: Some(time),
                    ..fact(&time.to_string(), "shared subject")
                })
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    let memory = Memory::new(core);
    let old = memory.get("10").unwrap().unwrap();
    let new = memory.get("20").unwrap().unwrap();
    assert_eq!(old.status, FactStatus::Superseded);
    assert_eq!(new.status, FactStatus::Current);
    assert_eq!(new.supersedes, vec!["10"]);
}

#[test]
fn conditional_changes_never_overwrite_a_raw_core_write() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let memory = Memory::new(core.clone());
    memory.remember(fact("shared", "original")).unwrap();
    let revision = core.revision();
    let replacement: crate::DocumentInput = serde_json::from_value(serde_json::json!({
        "id":"shared", "text":"raw replacement"
    }))
    .unwrap();
    core.memorize_document(replacement.clone()).unwrap();
    assert!(!core
        .change_memory_documents(vec![replacement], &["shared".into()], revision)
        .unwrap());
    assert_eq!(
        core.get_document("shared")
            .unwrap()
            .unwrap()
            .text
            .as_deref(),
        Some("raw replacement")
    );
}

#[test]
fn forgetting_policy_replans_instead_of_deleting_a_replaced_source() {
    struct ReplaceDuringSelection(Arc<WarmCore>, std::sync::atomic::AtomicBool);
    impl ForgettingPolicy for ReplaceDuringSelection {
        fn select(&self, _: &[FactSummary], _: Millis) -> Vec<String> {
            if !self.1.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.0
                    .memorize_document(
                        serde_json::from_value(serde_json::json!({
                            "id":"source", "text":"foreign replacement"
                        }))
                        .unwrap(),
                    )
                    .unwrap();
            }
            vec!["source".into()]
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let memory = Memory::new(core.clone());
    memory.remember(fact("source", "outdated fact")).unwrap();
    let report = memory
        .apply_forgetting_with(
            &ReplaceDuringSelection(core.clone(), false.into()),
            &ForgetOptions::default(),
        )
        .unwrap();
    assert!(report.forgotten.is_empty());
    assert_eq!(
        core.get_document("source")
            .unwrap()
            .unwrap()
            .text
            .as_deref(),
        Some("foreign replacement")
    );
}

#[test]
fn continuous_foreign_writes_return_contention_without_a_stale_commit() {
    struct AlwaysWrite(Arc<WarmCore>);
    impl SupersessionPolicy for AlwaysWrite {
        fn resolve(&self, _: &FactView, _: &[FactView]) -> Resolution {
            self.0
                .memorize_document(
                    serde_json::from_value(serde_json::json!({
                        "id":"foreign", "text":"writer advances revision"
                    }))
                    .unwrap(),
                )
                .unwrap();
            Resolution::default()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let memory = Memory::new(core.clone()).with_supersession(Arc::new(AlwaysWrite(core.clone())));
    let error = memory
        .remember(RememberInput {
            subject: Some("subject".into()),
            ..fact("planned", "must not be committed")
        })
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<MemoryError>(),
        Some(MemoryError::Concurrent(_))
    ));
    assert!(!core.has_document("planned"));
    assert!(core.has_document("foreign"));
}

#[test]
fn consolidation_replans_when_its_source_changes_during_the_policy_callback() {
    struct ReplaceDuringPlan(Arc<WarmCore>, std::sync::atomic::AtomicBool);
    impl ConsolidationPolicy for ReplaceDuringPlan {
        fn plan(&self, facts: &[FactSummary], _: Millis) -> Vec<ConsolidationAction> {
            if !facts.iter().any(|fact| fact.id == "source") {
                return vec![];
            }
            if !self.1.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.0
                    .memorize_document(
                        serde_json::from_value(serde_json::json!({
                            "id":"source", "text":"changed foreign source"
                        }))
                        .unwrap(),
                    )
                    .unwrap();
            }
            vec![ConsolidationAction {
                sources: vec!["source".into()],
                summary: fact("summary", "stale summary"),
                forget_sources: true,
            }]
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
    let memory = Memory::new(core.clone());
    memory.remember(fact("source", "original fact")).unwrap();
    let result = memory
        .consolidate_with(
            &ReplaceDuringPlan(core.clone(), false.into()),
            &ConsolidateOptions {
                allow_destructive: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(result.created.is_empty());
    assert!(!core.has_document("summary"));
    assert_eq!(
        core.get_document("source")
            .unwrap()
            .unwrap()
            .text
            .as_deref(),
        Some("changed foreign source")
    );
}

#[test]
fn reentrant_policy_callbacks_do_not_hold_the_memory_index_lock() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock, Weak,
    };
    struct Reenter {
        memory: OnceLock<Weak<Memory>>,
        wrote: AtomicBool,
    }
    impl Reenter {
        fn write_once(&self) {
            if !self.wrote.swap(true, Ordering::SeqCst) {
                self.memory
                    .get()
                    .unwrap()
                    .upgrade()
                    .unwrap()
                    .remember(fact("callback", "callback durable fact"))
                    .unwrap();
            }
        }
    }
    impl SupersessionPolicy for Reenter {
        fn resolve(&self, _: &FactView, _: &[FactView]) -> Resolution {
            self.write_once();
            Resolution::default()
        }
    }
    impl RecencyPolicy for Reenter {
        fn factor(&self, _: &RecencyContext) -> f64 {
            self.write_once();
            1.0
        }
    }
    for recall in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let core =
            Arc::new(WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap());
        let policy = Arc::new(Reenter {
            memory: OnceLock::new(),
            wrote: false.into(),
        });
        let memory = if recall {
            Memory::new(core.clone()).with_recency(policy.clone())
        } else {
            Memory::new(core.clone()).with_supersession(policy.clone())
        };
        let memory = Arc::new(memory);
        policy.memory.set(Arc::downgrade(&memory)).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = memory.clone();
        std::thread::spawn(move || {
            let result = worker
                .remember(RememberInput {
                    subject: Some("subject".into()),
                    ..fact("outer", "outer durable fact")
                })
                .and_then(|_| {
                    if recall {
                        worker
                            .recall("durable fact", &Default::default())
                            .map(|hits| {
                                assert_eq!(hits.len(), 2);
                            })
                    } else {
                        Ok(())
                    }
                });
            send.send(result).unwrap();
        });
        receive
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("policy callback deadlocked on its own memory index")
            .unwrap();
        assert!(core.has_document("callback"));
        assert!(core.has_document("outer"));
    }
}

#[test]
fn replacing_a_fact_leaves_the_index_equal_to_a_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(WarmCore::new(64, Some(dir.path().display().to_string()), None).unwrap());
    let config = MemoryConfig {
        on_id_conflict: IdConflict::Replace,
        ..MemoryConfig::default()
    };
    let memory = Memory::with_config(core.clone(), config).unwrap();
    memory
        .remember(RememberInput {
            subject: Some("s".into()),
            entities: Some(vec!["e1".into(), "e2".into()]),
            valid_from: Some(10),
            valid_to: Some(20),
            ..fact("a", "alpha one")
        })
        .unwrap();
    memory.remember(fact("b", "beta one")).unwrap();
    memory
        .remember(RememberInput {
            supersedes: Some(vec!["b".into()]),
            ..fact("c", "gamma one")
        })
        .unwrap();
    memory.ensure_fresh().unwrap();
    let tracked = |memory: &Memory| {
        let live = memory.index.read();
        let tracked = live.revision == Some(core.revision());
        if tracked {
            assert_eq!(live.canonical(), Index::rebuild(&core).unwrap().canonical());
        }
        tracked
    };

    // Unlinked: updated in place, the old subject, entities and window gone.
    memory
        .remember(RememberInput {
            subject: Some("t".into()),
            entities: Some(vec!["e2".into(), "e3".into()]),
            ..fact("a", "alpha two")
        })
        .unwrap();
    assert!(tracked(&memory));
    // Unlinked before, linked by the new stamp: still in place.
    memory
        .remember(RememberInput {
            supersedes: Some(vec!["c".into()]),
            ..fact("a", "alpha three")
        })
        .unwrap();
    assert!(tracked(&memory));
    // Linked (c supersedes b): only a rebuild reproduces other facts' links to it.
    memory.remember(fact("b", "beta two")).unwrap();
    assert!(!tracked(&memory));
    memory.ensure_fresh().unwrap();
    assert!(tracked(&memory));
}

mod hybrid {
    use super::*;
    use crate::core::config::WarmConfig;
    use crate::core::schema::EmbeddingSpace;

    fn space(dimensions: usize) -> EmbeddingSpace {
        EmbeddingSpace {
            model: "test".into(),
            revision: "1".into(),
            dimensions,
            normalization: "l2".into(),
            metric: "cosine".into(),
        }
    }

    fn store(dir: &std::path::Path, dimensions: Option<usize>) -> Memory {
        let config = WarmConfig {
            embedding_space: dimensions.map(space),
            ..WarmConfig::default()
        };
        let core = WarmCore::new(256, Some(dir.display().to_string()), Some(config)).unwrap();
        Memory::new(Arc::new(core))
    }

    fn with_vector(id: &str, text: &str, embedding: [f64; 3]) -> RememberInput {
        RememberInput {
            embedding: Some(embedding.to_vec()),
            ..fact(id, text)
        }
    }

    fn ids(results: Vec<Recalled>) -> Vec<String> {
        results.into_iter().map(|r| r.id).collect()
    }

    fn fixture(dir: &std::path::Path) -> Memory {
        let memory = store(dir, Some(3));
        memory
            .remember_many(vec![
                with_vector("car", "the car broke down on the highway", [1.0, 0.0, 0.0]),
                with_vector("auto", "my automobile stopped working", [0.9, 0.1, 0.0]),
                with_vector("order", "order zx81q7 shipped on monday", [0.0, 1.0, 0.0]),
                with_vector(
                    "garden",
                    "tomatoes need watering twice a week",
                    [0.0, 0.0, 1.0],
                ),
            ])
            .unwrap();
        memory
    }

    fn options(embedding: [f64; 3], k: u32) -> RecallOptions {
        RecallOptions {
            k: Some(k),
            embedding: Some(embedding.to_vec()),
            ..RecallOptions::default()
        }
    }

    #[test]
    fn dense_finds_a_paraphrase_lexical_misses() {
        let dir = tempfile::tempdir().unwrap();
        let memory = fixture(dir.path());
        let query = "the vehicle failed";
        let lexical = RecallOptions {
            semantic_weight: Some(0.0),
            ..options([1.0, 0.0, 0.0], 2)
        };
        let lexical = ids(memory.recall(query, &lexical).unwrap());
        assert!(!lexical.contains(&"auto".to_string()), "{lexical:?}");
        let hybrid = ids(memory.recall(query, &options([1.0, 0.0, 0.0], 2)).unwrap());
        assert_eq!(hybrid.len(), 2);
        assert!(hybrid.contains(&"auto".to_string()), "{hybrid:?}");
        assert!(hybrid.contains(&"car".to_string()), "{hybrid:?}");
    }

    #[test]
    fn lexical_finds_an_exact_term_dense_misses_and_fusion_keeps_both() {
        let dir = tempfile::tempdir().unwrap();
        let memory = fixture(dir.path());
        // The query vector points at the gardening fact; the exact term is in the order fact.
        let query_vector = [0.0, 0.2, 1.0];
        let dense = RecallOptions {
            lexical_weight: Some(0.0),
            ..options(query_vector, 1)
        };
        assert_eq!(ids(memory.recall("zx81q7", &dense).unwrap()), ["garden"]);
        let lexical = RecallOptions {
            semantic_weight: Some(0.0),
            ..options(query_vector, 1)
        };
        assert_eq!(ids(memory.recall("zx81q7", &lexical).unwrap()), ["order"]);
        let fused = RecallOptions {
            fusion: Some("rrf".into()),
            ..options(query_vector, 2)
        };
        let fused = ids(memory.recall("zx81q7", &fused).unwrap());
        assert_eq!(fused, ["order", "garden"]);
    }

    #[test]
    fn caller_embeddings_are_validated_with_clear_errors() {
        let dir = tempfile::tempdir().unwrap();
        let memory = store(dir.path(), Some(3));
        let invalid = |result: Result<_>| -> String {
            let error = result.expect_err("rejected");
            match error.downcast_ref::<MemoryError>() {
                Some(MemoryError::Invalid(message)) => message.clone(),
                other => panic!("not an invalid-request error: {other:?} {error:#}"),
            }
        };
        for (embedding, expected) in [
            (
                vec![1.0, 0.0],
                "2 values; the store's embedding space has 3 dimensions",
            ),
            (vec![1.0, 0.0, 0.0, 0.0], "4 values"),
            (vec![1.0, f64::NAN, 0.0], "finite"),
            (vec![f64::INFINITY, 0.0, 0.0], "finite"),
            (vec![0.0, 0.0, 0.0], "all zeros"),
        ] {
            let write = RememberInput {
                embedding: Some(embedding.clone()),
                ..fact("x", "some text")
            };
            let message = invalid(memory.remember(write).map(drop));
            assert!(message.contains(expected), "{message}");
            let read = RecallOptions {
                embedding: Some(embedding),
                ..RecallOptions::default()
            };
            let message = invalid(memory.recall("some", &read).map(drop));
            assert!(message.contains(expected), "{message}");
        }
        assert!(memory.get("x").unwrap().is_none());
        let plain_dir = tempfile::tempdir().unwrap();
        let plain = store(plain_dir.path(), None);
        let message = invalid(
            plain
                .remember(with_vector("x", "some text", [1.0, 0.0, 0.0]))
                .map(drop),
        );
        assert!(message.contains("embedding space"), "{message}");
        let message = invalid(plain.recall("some", &options([1.0, 0.0, 0.0], 1)).map(drop));
        assert!(message.contains("embedding space"), "{message}");
    }

    #[test]
    fn a_remember_call_caps_its_embedding_values() {
        const DIMENSIONS: usize = 1024;
        let dir = tempfile::tempdir().unwrap();
        let memory = store(dir.path(), Some(DIMENSIONS));
        let batch = |count: usize| -> Vec<RememberInput> {
            (0..count)
                .map(|i| RememberInput {
                    embedding: Some(vec![1.0; DIMENSIONS]),
                    ..fact(&format!("f{i}"), &format!("fact number {i}"))
                })
                .collect()
        };
        let limit = super::super::write::MAX_BATCH_EMBEDDING_VALUES / DIMENSIONS;
        let error = memory.remember_many(batch(limit + 1)).unwrap_err();
        assert!(
            format!("{error:#}").contains("embedding values"),
            "{error:#}"
        );
        assert!(memory.get("f0").unwrap().is_none());
        assert_eq!(
            memory.remember_many(batch(limit - 1)).unwrap().len(),
            limit - 1
        );
        assert_eq!(memory.remember_many(batch(limit)).unwrap().len(), limit);
    }

    #[test]
    fn stores_without_vectors_recall_exactly_as_document_search() {
        let texts = [
            "alpha beta gamma",
            "beta gamma delta delta",
            "gamma epsilon zeta",
            "the quick brown fox",
            "brown fox jumps over beta",
        ];
        let open = |dimensions| {
            let dir = tempfile::tempdir().unwrap();
            let memory = store(dir.path(), dimensions);
            for (i, text) in texts.iter().enumerate() {
                memory.remember(fact(&format!("f{i}"), text)).unwrap();
            }
            (dir, memory)
        };
        let (_plain_dir, plain) = open(None);
        // An embedding space with no embedder and no vectors must not change any ranking.
        let (_space_dir, with_space) = open(Some(3));
        for query in ["beta", "gamma delta", "brown fox", "unseen words only"] {
            let expected = plain
                .core()
                .search_documents(
                    query,
                    &crate::core::documents::SearchOptions {
                        k: Some(10),
                        ..Default::default()
                    },
                )
                .unwrap();
            for memory in [&plain, &with_space] {
                let got = memory.recall(query, &RecallOptions::default()).unwrap();
                assert_eq!(got.len(), expected.len(), "{query}");
                for (got, expected) in got.iter().zip(&expected) {
                    assert_eq!(got.id, expected.document_id, "{query}");
                    assert_eq!(got.score.to_bits(), expected.score.to_bits(), "{query}");
                }
            }
        }
    }

    #[cfg(feature = "local-embed")]
    #[test]
    fn an_installed_embedder_makes_recall_hybrid_without_caller_vectors() {
        use crate::core::embed::testing::write_tiny;
        use crate::core::embed::{EmbedOptions, Embedder, Pooling, Quantization};
        let models = tempfile::tempdir().unwrap();
        let (pack, sha) = write_tiny(models.path(), Quantization::F32, Pooling::Mean);
        let embedder =
            Arc::new(Embedder::from_pack(&pack, Some(&sha), &EmbedOptions::default()).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let config = WarmConfig {
            embedding_space: Some(embedder.space()),
            ..WarmConfig::default()
        };
        let core = Arc::new(
            WarmCore::new(256, Some(dir.path().display().to_string()), Some(config)).unwrap(),
        );
        core.set_text_embedder(Some(embedder.clone())).unwrap();
        let memory = Memory::new(core);
        memory
            .remember(fact("cat", "the cat sat on the mat"))
            .unwrap();
        memory
            .remember(fact("dog", "the dog ran in the park"))
            .unwrap();
        let explained = RecallOptions {
            explain: true,
            ..RecallOptions::default()
        };
        let hits = memory.recall("cat mat", &explained).unwrap();
        assert_eq!(hits[0].id, "cat");
        let semantic = hits[0].explanation.as_ref().unwrap().semantic_score;
        assert!(semantic.is_some_and(f64::is_finite), "{semantic:?}");
        let lexical = RecallOptions {
            semantic_weight: Some(0.0),
            ..explained
        };
        let hits = memory.recall("cat mat", &lexical).unwrap();
        assert!(hits[0]
            .explanation
            .as_ref()
            .unwrap()
            .semantic_score
            .is_none());
    }
}
