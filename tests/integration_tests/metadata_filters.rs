// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Value};
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{
    DocumentInput, EmbeddingSpace, SearchOptions, UnsupportedFilterOperator, WarmCore,
    SUPPORTED_FILTER_OPERATORS,
};

fn doc(id: &str, text: &str, metadata: Value) -> DocumentInput {
    serde_json::from_value(json!({"id": id, "text": text, "metadata": metadata})).unwrap()
}

fn ids(warm: &WarmCore, filter: Value) -> Vec<String> {
    let options = SearchOptions {
        filter: Some(filter),
        k: Some(50),
        ..Default::default()
    };
    let mut ids: Vec<String> = warm
        .search_documents("memo", &options)
        .unwrap()
        .into_iter()
        .map(|hit| hit.document_id)
        .collect();
    ids.sort();
    ids
}

fn corpus(dir: &std::path::Path) -> WarmCore {
    let warm = WarmCore::new(4096, Some(dir.display().to_string()), None).unwrap();
    warm.memorize_documents(vec![
        doc(
            "a",
            "memo one",
            json!({"team": "red", "n": 1, "tags": ["x", "y"]}),
        ),
        doc(
            "b",
            "memo two",
            json!({"team": "blue", "n": 2, "tags": ["y", "z"]}),
        ),
        doc(
            "c",
            "memo three",
            json!({"team": "green", "n": 3, "tags": []}),
        ),
        doc("d", "memo four", json!({"team": "red", "n": 4})),
    ])
    .unwrap();
    warm
}

#[test]
fn an_unsupported_operator_is_an_error_naming_it_and_the_supported_set() {
    let dir = tempfile::tempdir().unwrap();
    let warm = corpus(dir.path());
    for filter in [
        json!({"n": {"$gt": 1}}),
        json!({"$or": [{"team": "red"}]}),
        json!({"team": {"$in": ["red"], "$regex": "r"}}),
    ] {
        let options = SearchOptions {
            filter: Some(filter.clone()),
            ..Default::default()
        };
        let error = warm.search_documents("memo", &options).unwrap_err();
        let typed = error.downcast_ref::<UnsupportedFilterOperator>().unwrap();
        let named = filter.to_string();
        assert!(named.contains(&typed.operator), "{named}");
        assert_eq!(typed.supported, SUPPORTED_FILTER_OPERATORS);
        let message = error.to_string();
        assert!(
            message.contains(&typed.operator) && message.contains("$in"),
            "{message}"
        );
    }
    assert!(warm
        .search_documents(
            "memo",
            &SearchOptions {
                filter: Some(json!({"team": {"$in": "red"}})),
                ..Default::default()
            }
        )
        .is_err());
}

#[test]
fn eq_and_in_select_documents_and_list_fields_match_any_element() {
    let dir = tempfile::tempdir().unwrap();
    let warm = corpus(dir.path());
    assert_eq!(ids(&warm, json!({"team": "red"})), ["a", "d"]);
    assert_eq!(ids(&warm, json!({"team": {"$eq": "red"}})), ["a", "d"]);
    assert_eq!(
        ids(&warm, json!({"team": {"$in": ["red", "green"]}})),
        ["a", "c", "d"]
    );
    assert_eq!(ids(&warm, json!({"n": {"$in": [2, 4]}})), ["b", "d"]);
    assert!(ids(&warm, json!({"team": {"$in": []}})).is_empty());
    assert!(ids(&warm, json!({"team": {"$in": ["purple"]}})).is_empty());
    // Terms are ANDed.
    assert_eq!(
        ids(&warm, json!({"team": {"$in": ["red", "blue"]}, "n": 4})),
        ["d"]
    );
    // A list field matches when any element equals the value, or for $in is in the set.
    assert_eq!(ids(&warm, json!({"tags": "y"})), ["a", "b"]);
    assert_eq!(ids(&warm, json!({"tags": {"$eq": "z"}})), ["b"]);
    assert_eq!(ids(&warm, json!({"tags": {"$in": ["x", "z"]}})), ["a", "b"]);
    assert!(ids(&warm, json!({"tags": "q"})).is_empty());
    // A list value still compares as a whole.
    assert_eq!(ids(&warm, json!({"tags": ["y", "z"]})), ["b"]);
    // Arrays are stored as arrays, and matching survives replacement, deletion and reopen.
    warm.memorize_document(doc("b", "memo two", json!({"tags": ["w"]})))
        .unwrap();
    warm.delete_document("a").unwrap();
    assert_eq!(ids(&warm, json!({"tags": "w"})), ["b"]);
    assert!(ids(&warm, json!({"tags": "y"})).is_empty());
    warm.compact().unwrap();
    drop(warm);
    let warm = WarmCore::new(4096, Some(dir.path().display().to_string()), None).unwrap();
    assert_eq!(ids(&warm, json!({"tags": {"$in": ["w", "x"]}})), ["b"]);
    let hit = &warm
        .search_documents(
            "memo",
            &SearchOptions {
                filter: Some(json!({"tags": "w"})),
                ..Default::default()
            },
        )
        .unwrap()[0];
    assert_eq!(hit.metadata["tags"], json!(["w"]));
}

fn unit(seed: u64, dims: usize) -> Vec<f64> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..dims)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f64 / (1u64 << 31) as f64) - 0.5
        })
        .collect()
}

fn cosine(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f64]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
    dot / (norm(a) * norm(b))
}

#[test]
fn every_hit_carries_the_exact_cosine_of_its_stored_vector() {
    const DIMS: usize = 16;
    let dir = tempfile::tempdir().unwrap();
    let config = WarmConfig {
        embedding_space: Some(EmbeddingSpace {
            model: "fixture".into(),
            revision: "1".into(),
            dimensions: DIMS,
            normalization: "l2".into(),
            metric: "cosine".into(),
        }),
        ..Default::default()
    };
    let path = dir.path().display().to_string();
    let warm = WarmCore::new(4096, Some(path.clone()), Some(config.clone())).unwrap();
    // More rows than the semantic screen's threshold, so screened and exact scoring both run.
    let vectors: Vec<Vec<f64>> = (0..300).map(|i| unit(i, DIMS)).collect();
    let inputs = vectors
        .iter()
        .enumerate()
        .map(|(i, vector)| {
            let mut input = doc(&format!("d{i}"), "keyword filler", json!({}));
            input.embeddings = Some(vec![vector.clone()]);
            input
        })
        .collect();
    warm.memorize_documents(inputs).unwrap();
    let query = unit(10_000, DIMS);
    let check = |warm: &WarmCore| {
        let options = SearchOptions {
            embedding: Some(query.clone()),
            k: Some(40),
            min_semantic_score: Some(-1.0),
            ..Default::default()
        };
        let hits = warm.search_documents("keyword", &options).unwrap();
        assert_eq!(hits.len(), 40);
        for hit in hits {
            let index: usize = hit.document_id[1..].parse().unwrap();
            let expected = cosine(&vectors[index], &query);
            let score = hit.semantic_score.expect("a query vector yields a score");
            assert!((score - expected).abs() < 1e-6, "{score} vs {expected}");
        }
        // Without a query vector there is nothing to score against.
        let lexical = warm
            .search_documents("keyword", &SearchOptions::default())
            .unwrap();
        assert!(lexical.iter().all(|hit| hit.semantic_score.is_none()));
    };
    check(&warm);
    warm.compact().unwrap();
    drop(warm);
    check(&WarmCore::new(4096, Some(path), Some(config)).unwrap());
}

#[test]
fn the_store_reports_the_model_its_vectors_were_built_with() {
    let dir = tempfile::tempdir().unwrap();
    let plain =
        WarmCore::new(4096, Some(dir.path().join("p").display().to_string()), None).unwrap();
    assert!(plain.embedding_model().is_none());
    let config = WarmConfig {
        embedding_space: Some(EmbeddingSpace {
            model: "fixture".into(),
            revision: "rev-1".into(),
            dimensions: 3,
            normalization: "l2".into(),
            metric: "cosine".into(),
        }),
        ..Default::default()
    };
    let warm = WarmCore::new(
        4096,
        Some(dir.path().join("q").display().to_string()),
        Some(config),
    )
    .unwrap();
    let info = warm.embedding_model().unwrap();
    assert_eq!(
        (
            info.model.as_str(),
            info.revision.as_str(),
            info.dimensions,
            info.pack_sha256
        ),
        ("fixture", "rev-1", 3, None)
    );
}
