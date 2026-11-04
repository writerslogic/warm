// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! A store written by a090f971 (before store analyzers existed), committed under
//! `tests/fixtures/legacy-analyzer-store` with the rankings that release returned
//! (`expected.json`), reopens with the legacy analyzer and answers exactly as it did: same
//! documents, scores and counts, before and after compaction. It refuses a non-legacy analyzer.

use std::path::{Path, PathBuf};

use writerslogic_warm::core::documents::Analyzer;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{SearchOptions, WarmCore};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy-analyzer-store")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn open(path: &Path, analyzer: Option<Analyzer>) -> anyhow::Result<WarmCore> {
    WarmCore::new(
        1024,
        Some(path.display().to_string()),
        Some(WarmConfig {
            analyzer,
            ..Default::default()
        }),
    )
}

fn assert_answers_as_written(core: &WarmCore, stage: &str) {
    let expected: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(fixture().join("expected.json")).unwrap()).unwrap();
    let search = SearchOptions {
        k: Some(20),
        candidate_limit: Some(20),
        lexical_weight: Some(1.0),
        semantic_weight: Some(0.0),
        ..Default::default()
    };
    assert!(expected.len() >= 10);
    for case in expected {
        let query = case["query"].as_str().unwrap();
        let hits: Vec<serde_json::Value> = core
            .search_documents(query, &search)
            .unwrap()
            .into_iter()
            .map(|r| serde_json::json!([r.document_id, r.score, r.lexical_score]))
            .collect();
        assert_eq!(
            hits,
            case["hits"].as_array().unwrap().clone(),
            "{stage}: {query:?}"
        );
        assert_eq!(
            core.count_documents(query).unwrap() as u64,
            case["count"].as_u64().unwrap(),
            "{stage}: {query:?}"
        );
    }
}

#[test]
fn store_written_before_analyzers_answers_exactly_as_it_did() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    copy_dir(&fixture().join("store"), &store);
    {
        let core = open(&store, None).unwrap();
        assert_eq!(core.analyzer(), Analyzer::Legacy);
        assert_answers_as_written(&core, "reopened");
        core.compact().unwrap();
        assert_answers_as_written(&core, "compacted");
    }
    let core = open(&store, Some(Analyzer::Legacy)).unwrap();
    assert_answers_as_written(&core, "reopened after compaction");
    drop(core);
    let error = open(&store, Some(Analyzer::English))
        .err()
        .expect("opened as english");
    assert!(format!("{error:#}").contains("legacy"), "{error:#}");
    let schema = std::fs::read_to_string(store.join("store.json")).unwrap();
    assert!(!schema.contains("lexical_analyzer"), "{schema}");
}
