// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#[test]
fn bounded_filtered_scan_preserves_missing_embeddings_and_minimum_scores() {
    let mut store = DocumentStore::default();
    for id in 0..257 {
        let mut document = document(id, 1);
        if id % 3 == 0 {
            document.chunks[0].embedding = None;
        }
        store.insert(document.id.clone(), document);
    }
    let trained = super::super::build_dense_index(
        super::super::capture_dense_index(&store, 3)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(&mut store, &trained));
    for fusion in ["blend", "rrf"] {
        for minimum in [-1.0, 0.0, 0.9, 1.0] {
            let options = SearchOptions {
                embedding: Some(vec![1.0, 0.0, 0.0]),
                filter: Some(json!({"group": 0})),
                lexical_weight: Some(0.0),
                semantic_weight: Some(1.0),
                min_semantic_score: Some(minimum),
                fusion: Some(fusion.into()),
                k: Some(10),
                candidate_limit: Some(100),
                ..Default::default()
            };
            let captured =
                capture_search(&store, vec![prepare_search("", &options, Some(3)).unwrap()]);
            assert!(captured.queries[0].scan.is_some());
            let expected = super::super::search_impl(&store, "", &options, Some(3), false).unwrap();
            assert_eq!(
                serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }
}

#[test]
fn bounded_filtered_pruning_merges_live_deltas_and_excludes_replaced_rows() {
    let document = |id: usize, positive: bool| {
        let vector = if positive {
            vec![1.0, 0.0]
        } else {
            vec![-1.0, (id % 5) as f64 / 100.0]
        };
        let input =
            serde_json::from_value(json!({"id": format!("d{id:04}"), "text": "current owner",
            "metadata": {"group": id % 2}, "embeddings": [vector]}))
            .unwrap();
        super::super::prepare(input, 1024, Some(2), TextEncoder::default()).unwrap()
    };
    let mut store = DocumentStore::default();
    for id in 0..8192 {
        let document = document(id, id < 256);
        store.insert(document.id.clone(), document);
    }
    let trained = super::super::build_dense_index(
        super::super::capture_dense_index(&store, 2)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(&mut store, &trained));
    let replacement = document(0, false);
    store.insert(replacement.id.clone(), replacement);
    store.remove("d0002").unwrap();
    let mut added = document(8192, true);
    added.id = "a-new".into();
    store.insert(added.id.clone(), added);
    assert!(store.dense_index.is_some());
    for fusion in ["blend", "rrf"] {
        let options = SearchOptions {
            embedding: Some(vec![1.0, 0.0]),
            filter: Some(json!({"group": 0})),
            lexical_weight: Some(0.0),
            semantic_weight: Some(1.0),
            min_semantic_score: Some(-1.0),
            fusion: Some(fusion.into()),
            k: Some(50),
            candidate_limit: Some(100),
            ..Default::default()
        };
        let captured = capture_search(&store, vec![prepare_search("", &options, Some(2)).unwrap()]);
        let query = &captured.queries[0];
        let (_, dots) = query
            .dense
            .as_ref()
            .unwrap()
            .search_where_limited(
                query.prepared.embedding.as_ref().unwrap(),
                query.prepared.limit,
                query.prepared.minimum,
                |document| {
                    !query
                        .excluded_owners
                        .contains(&(document as *const StoredDocument as usize))
                        && query.prepared.matches(document)
                },
                query.count * 3 / 16,
            )
            .unwrap();
        assert!(dots < query.count);
        let expected = super::super::search_impl(&store, "", &options, Some(2), false).unwrap();
        assert!(expected.iter().any(|hit| hit.document_id == "a-new"));
        assert!(!expected
            .iter()
            .any(|hit| matches!(hit.document_id.as_str(), "d0000" | "d0002")));
        assert_eq!(
            serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn bounded_filtered_snapshot_keeps_current_owners_across_mutation_and_store_drop() {
    let mut store = store(256);
    let trained = super::super::build_dense_index(
        super::super::capture_dense_index(&store, 3)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(&mut store, &trained));
    let options = SearchOptions {
        embedding: Some(vec![1.0, 0.0, 0.0]),
        filter: Some(json!({"group": 0})),
        lexical_weight: Some(0.0),
        semantic_weight: Some(1.0),
        min_semantic_score: Some(-1.0),
        k: Some(10),
        candidate_limit: Some(100),
        ..Default::default()
    };
    let captured = capture_search(&store, vec![prepare_search("", &options, Some(3)).unwrap()]);
    assert_eq!(captured.queries[0].scan.as_ref().unwrap().len(), 128);
    let query = &captured.queries[0];
    assert!(query
        .dense
        .as_ref()
        .unwrap()
        .search_where_limited(
            query.prepared.embedding.as_ref().unwrap(),
            query.prepared.limit,
            query.prepared.minimum,
            |document| query.prepared.matches(document),
            query.count * 4 / 16,
        )
        .is_none());
    let expected = serde_json::to_value(
        super::super::search_impl(&store, "", &options, Some(3), false).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
        expected
    );
    let mut replacement = document(0, 99);
    replacement.metadata = json!({"group": 1}).into();
    store.insert(replacement.id.clone(), replacement);
    store.remove("d002").unwrap();
    let added = document(1000, 1);
    store.insert(added.id.clone(), added);
    assert!(store.dense_index.is_some());
    for fusion in ["blend", "rrf"] {
        let options = SearchOptions {
            fusion: Some(fusion.into()),
            ..options.clone()
        };
        let fresh = capture_search(&store, vec![prepare_search("", &options, Some(3)).unwrap()]);
        let expected = serde_json::to_value(
            super::super::search_impl(&store, "", &options, Some(3), false).unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(execute_search(&fresh, 0).unwrap()).unwrap(),
            expected
        );
    }
    drop(store);
    assert_eq!(
        serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
        expected
    );
}

use super::*;
use crate::core::encoding::TextEncoder;
use crate::core::sync::RwLock;
use serde_json::json;
use std::sync::Barrier;

#[test]
fn eligible_owner_membership_preserves_wide_addresses_and_exact_boundaries() {
    let mut ranges = vec![vec![], vec![0], vec![usize::MAX], vec![0, usize::MAX]];
    for width in [u32::MAX as usize - 1, u32::MAX as usize] {
        ranges.push(vec![0, width / 2, width, width]);
        ranges.push(vec![usize::MAX - width, usize::MAX]);
    }
    if let Some(width) = (u32::MAX as usize).checked_add(1) {
        ranges.push(vec![0, width]);
        ranges.push(vec![usize::MAX - width, usize::MAX]);
    }
    for addresses in ranges {
        let owners = EligibleOwners::new(addresses.clone());
        for address in addresses.iter().copied().chain([0, 1, usize::MAX]) {
            for probe in [
                address.checked_sub(1),
                Some(address),
                address.checked_add(1),
            ]
            .into_iter()
            .flatten()
            {
                assert_eq!(owners.contains(&probe), addresses.contains(&probe));
            }
        }
    }
}

#[test]
fn planned_filtered_lexical_and_hybrid_match_oracle_under_term_skew() {
    let mut store = store(512);
    let mut long = document(7, 1);
    long.chunks[0].word_count = 300;
    store.insert(long.id.clone(), long);
    for trained in [false, true] {
        if trained {
            let index = super::super::build_dense_index(
                super::super::capture_dense_index(&store, 3)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap()
            .unwrap();
            assert!(super::super::publish_dense_index(&mut store, &index));
        }
        for group in [1, 0, 1] {
            for query in [
                "rare7 rare9 ghost absent",
                "common",
                "common rare7 rare9 ghost absent",
            ] {
                for embedding in [None, Some(vec![1.0, 0.0, 0.0])] {
                    for k1 in [0.0, 1.2] {
                        let options = SearchOptions {
                            k: Some(3),
                            candidate_limit: Some(5),
                            filter: Some(json!({"group": group})),
                            embedding: embedding.clone(),
                            bm25_k1: Some(k1),
                            min_semantic_score: Some(-1.0),
                            ..Default::default()
                        };
                        let captured = capture_search(
                            &store,
                            vec![prepare_search(query, &options, Some(3)).unwrap()],
                        );
                        if trained || embedding.is_none() {
                            assert!(captured.queries[0].chunks.len() <= 5);
                            assert_eq!(captured.queries[0].count, 512);
                        }
                        let expected =
                            super::super::search_impl(&store, query, &options, Some(3), false)
                                .unwrap();
                        assert_eq!(
                            serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
                            serde_json::to_value(expected).unwrap()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn delta_capture_bounds_owners_and_survives_later_replacement() {
    let mut store = store(512);
    let index = super::super::build_dense_index(
        super::super::capture_dense_index(&store, 3)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(&mut store, &index));
    let base_owner = Arc::clone(&store.documents["d007"]);
    store.insert("d007".into(), document(7, 2));
    let options = SearchOptions {
        k: Some(3),
        candidate_limit: Some(5),
        filter: Some(json!({"group": 1})),
        embedding: Some(vec![1.0, 0.0, 0.0]),
        min_semantic_score: Some(-1.0),
        ..Default::default()
    };
    let captured = capture_search(
        &store,
        vec![prepare_search("common rare7", &options, Some(3)).unwrap()],
    );
    assert!(captured.queries[0].dense.is_some());
    assert!(captured.queries[0].chunks.len() <= 5 + store.documents["d007"].chunks.len());
    if let Some(eligible) = &captured.queries[0].eligible {
        assert!(!eligible.contains(&(Arc::as_ptr(&base_owner) as usize)));
        assert!(eligible.contains(&(Arc::as_ptr(&store.documents["d007"]) as usize)));
        assert!(captured.queries[0].excluded_owners.is_empty());
    } else {
        assert!(captured.queries[0]
            .excluded_owners
            .contains(&(Arc::as_ptr(&base_owner) as usize)));
    }
    let expected =
        super::super::search_impl(&store, "common rare7", &options, Some(3), false).unwrap();
    store.insert("d007".into(), document(7, 3));
    assert_eq!(
        serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn audit_regression_filtered_hybrid_capture_bounds_owners_and_reuses_exact_index() {
    let mut store = store(512);
    let trained = super::super::build_dense_index(
        super::super::capture_dense_index(&store, 3)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(&mut store, &trained));
    for filter in [
        None,
        Some(json!({"group": 1})),
        Some(json!({"nested": {"flag": 0}})),
    ] {
        for fusion in ["blend", "rrf"] {
            let options = SearchOptions {
                k: Some(7),
                candidate_limit: Some(11),
                embedding: Some(vec![1.0, 0.0, 0.0]),
                filter: filter.clone(),
                fusion: Some(fusion.into()),
                min_semantic_score: Some(-1.0),
                ..Default::default()
            };
            let captured = capture_search(
                &store,
                vec![prepare_search("common rare7 missing", &options, Some(3)).unwrap()],
            );
            assert!(captured.queries[0].dense.is_some());
            assert!(captured.queries[0].chunks.len() <= 11);
            let expected =
                super::super::search_impl(&store, "common rare7 missing", &options, Some(3), false)
                    .unwrap();
            assert_eq!(
                serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }
}

#[test]
fn audit_regression_zero_bm25_saturation_ignores_absent_terms() {
    let store = store(128);
    for filter in [None, Some(json!({"group": 1}))] {
        let options = SearchOptions {
            k: Some(7),
            bm25_k1: Some(0.0),
            filter,
            source_uri: Some("source1".into()),
            ..Default::default()
        };
        let actual = super::super::search(&store, "common missing", &options, Some(3)).unwrap();
        let expected = super::super::search(&store, "common", &options, Some(3)).unwrap();
        assert!(!expected.is_empty());
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn trained_semantic_capture_pins_one_index_and_only_lexical_posting_owners() {
    let mut store = store(128);
    let trained = super::super::build_dense_index(
        super::super::capture_dense_index(&store, 3)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(&mut store, &trained));
    let options = SearchOptions {
        embedding: Some(vec![1.0, 0.0, 0.0]),
        lexical_weight: Some(0.0),
        min_semantic_score: Some(-1.0),
        ..Default::default()
    };
    let captured = capture_search(&store, vec![prepare_search("", &options, Some(3)).unwrap()]);
    assert!(captured.queries[0].chunks.is_empty());
    assert!(Arc::ptr_eq(
        captured.queries[0].dense.as_ref().unwrap(),
        store.dense_index.as_ref().unwrap()
    ));
    let expected = serde_json::to_value(
        super::super::search_impl(&store, "", &options, Some(3), false).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
        expected
    );
    let options = SearchOptions {
        lexical_weight: Some(1.0),
        ..options
    };
    let captured = capture_search(
        &store,
        vec![prepare_search("rare7 ghost", &options, Some(3)).unwrap()],
    );
    assert_eq!(captured.queries[0].chunks.len(), 1);
    assert_eq!(captured.queries[0].chunks[0].document().id, "d007");
    assert!(Arc::ptr_eq(
        captured.queries[0].chunks[0].document(),
        &store.documents["d007"]
    ));
    let expected = serde_json::to_value(
        super::super::search_impl(&store, "rare7 ghost", &options, Some(3), false).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
        expected
    );
}

fn document(id: usize, revision: usize) -> StoredDocument {
    let input = serde_json::from_value(json!({
        "id": format!("d{id:03}"), "version": revision.to_string(),
        "text": format!("running common rare{id} generation{revision} old semantic"),
        "sourceUri": format!("source{}", id % 3),
        "metadata": {"group": id % 2, "nested": {"flag": id % 2}},
        "chunkWords": 3, "overlapWords": 0,
        "embeddings": [[1.0, id as f64 / 10.0, 0.0], [0.0, 1.0, revision as f64 / 100.0]],
    }))
    .unwrap();
    let mut document = super::super::prepare(input, 1024, Some(3), TextEncoder::default()).unwrap();
    for chunk in &mut document.chunks {
        if id.is_multiple_of(2) {
            chunk.terms =
                crate::core::documents::terms(chunk.text.as_deref().unwrap(), false).into();
            chunk.stemmed = false;
        }
        chunk.terms.insert("ghost".into(), 0);
    }
    document
}

fn store(count: usize) -> DocumentStore {
    let mut store = DocumentStore::default();
    for id in (0..count).rev() {
        let document = document(id, 1);
        store.insert(document.id.clone(), document);
    }
    store
}

fn variants() -> Vec<(String, SearchOptions)> {
    let mut variants = Vec::new();
    for fusion in ["blend", "rrf"] {
        for filter in [
            None,
            Some(json!({"group": 1})),
            Some(json!({"nested": {"flag": 0}})),
            Some(json!({"group": 99})),
        ] {
            for embedding in [None, Some(vec![1.0, 0.0, 0.0])] {
                for limit in [9, 10, 11] {
                    variants.push((
                        "running semantic ghost rare3".into(),
                        SearchOptions {
                            k: Some(9),
                            candidate_limit: Some(limit),
                            fusion: Some(fusion.into()),
                            filter: filter.clone(),
                            embedding: embedding.clone(),
                            min_semantic_score: Some(-1.0),
                            ..Default::default()
                        },
                    ));
                }
            }
        }
    }
    variants.push((
        "running".into(),
        SearchOptions {
            source_uri: Some("source1".into()),
            document_ids: Some(vec!["d001".into(), "d004".into(), "d001".into()]),
            ..Default::default()
        },
    ));
    variants
}

fn capture_variants(store: &DocumentStore, variants: &[(String, SearchOptions)]) -> CapturedSearch {
    capture_search(
        store,
        variants
            .iter()
            .map(|(text, options)| prepare_search(text, options, Some(3)).unwrap())
            .collect(),
    )
}

fn scanned_variants(store: &DocumentStore, variants: &[(String, SearchOptions)]) -> Vec<Value> {
    variants
        .iter()
        .map(|(text, options)| {
            serde_json::to_value(
                super::super::search_impl(store, text, options, Some(3), false).unwrap(),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn captured_view_survives_replace_delete_slot_reuse_and_store_drop() {
    let mut store = store(12);
    let variants = variants();
    let expected = scanned_variants(&store, &variants);
    let captured = capture_variants(&store, &variants);
    let old = Arc::downgrade(&store.documents["d000"]);
    let captured_document = captured.queries[0]
        .chunks
        .iter()
        .find(|chunk| chunk.document().id == "d000")
        .unwrap();
    assert!(Arc::ptr_eq(
        captured_document.document(),
        &store.documents["d000"]
    ));
    assert_eq!(
        captured_document
            .stored()
            .unwrap()
            .embedding
            .as_ref()
            .unwrap()
            .as_ptr(),
        store.documents["d000"].chunks[captured_document.index]
            .embedding
            .as_ref()
            .unwrap()
            .as_ptr()
    );
    let replacement = document(0, 2);
    store.insert(replacement.id.clone(), replacement);
    assert!(store.remove("d002").is_some());
    let replacement = document(20, 2);
    store.insert(replacement.id.clone(), replacement);
    let ids: Vec<_> = store.documents.keys().cloned().collect();
    for id in ids {
        store.remove(&id);
    }
    assert_eq!(store.slot_count, 0);
    drop(store);
    assert!(old.upgrade().is_some());
    for (i, expected) in expected.into_iter().enumerate() {
        assert_eq!(
            serde_json::to_value(execute_search(&captured, i).unwrap()).unwrap(),
            expected
        );
    }
    drop(captured);
    assert!(old.upgrade().is_none());
}

/// A store of `count` two-chunk documents whose embeddings are pseudo-random unit-ish vectors in 8 dimensions,
/// with exact duplicates and a shared near-tie so the list's boundary falls among equal scores.
fn noisy_store(count: usize, without_embedding: usize) -> DocumentStore {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as f64 / (1u64 << 31) as f64) - 0.5
    };
    let mut store = DocumentStore::default();
    for id in 0..count {
        let vectors: Vec<Vec<f64>> = (0..2)
            .map(|chunk| {
                if (id + chunk) % 7 == 0 {
                    vec![0.25; 8]
                } else {
                    (0..8).map(|_| next()).collect()
                }
            })
            .collect();
        let input = serde_json::from_value(json!({
            "id": format!("n{id:04}"), "version": "1",
            "text": format!("running common rare{id} old semantic noisy"),
            "chunkWords": 3, "overlapWords": 0,
            "embeddings": vectors,
        }))
        .unwrap();
        let mut document =
            super::super::prepare(input, 1024, Some(8), TextEncoder::default()).unwrap();
        if id < without_embedding {
            document.chunks[0].embedding = None;
        }
        store.insert(document.id.clone(), document);
    }
    store
}

/// Screening only decides which chunks are scored exactly: the ranked output is identical to the reference scan
/// on both sides of the row count where screening starts (`SCREEN_MIN_ROWS` - 1, `SCREEN_MIN_ROWS`, +1), at
/// the list-length boundaries, with and without a floor, and with chunks lacking an embedding.
#[test]
fn screened_semantic_scan_matches_the_exact_scan() {
    let query = vec![0.3, -0.2, 0.5, 0.1, 0.4, -0.3, 0.2, 0.25];
    for (count, missing) in [
        (SCREEN_MIN_ROWS / 2 - 1, 0),
        (SCREEN_MIN_ROWS / 2, 1),
        (SCREEN_MIN_ROWS / 2, 0),
        (SCREEN_MIN_ROWS / 2 + 1, 0),
        (400, 3),
        (1100, 3),
    ] {
        let store = noisy_store(count, missing);
        for fusion in ["blend", "rrf"] {
            for (limit, text) in [
                (9u32, "running rare3"),
                (10, "common old"),
                (11, "running rare3"),
                (100, "common old"),
            ] {
                for (floor, lexical_weight) in [(-1.0, 1.0), (0.3, 1.0), (-1.0, 0.0), (0.3, 0.0)] {
                    let options = SearchOptions {
                        k: Some(9),
                        candidate_limit: Some(limit),
                        fusion: Some(fusion.into()),
                        embedding: Some(query.clone()),
                        min_semantic_score: Some(floor),
                        lexical_weight: Some(lexical_weight),
                        ..Default::default()
                    };
                    let expected = serde_json::to_value(
                        super::super::search_impl(&store, text, &options, Some(8), false).unwrap(),
                    )
                    .unwrap();
                    let actual = serde_json::to_value(
                        super::super::search_impl(&store, text, &options, Some(8), true).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        actual, expected,
                        "{count} {missing} {fusion} {limit} {floor}"
                    );
                }
            }
        }
        if count * 2 >= 2048 {
            assert!(
                store.screen_table(8).is_some(),
                "the screen table serves {count}"
            );
        }
    }
}

/// A one-chunk document of `words` with a pseudo-random embedding.
fn chunk_document(id: usize, words: &str, state: &mut u64) -> StoredDocument {
    let mut next = || {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*state >> 33) as f64 / (1u64 << 31) as f64) - 0.5
    };
    let vector: Vec<f64> = (0..8).map(|_| next()).collect();
    let input = serde_json::from_value(json!({
        "id": format!("h{id:05}"), "version": "1", "text": words,
        "chunkWords": 32, "overlapWords": 0, "embeddings": [vector],
    }))
    .unwrap();
    super::super::prepare(input, 1024, Some(8), TextEncoder::default()).unwrap()
}

/// A thread keeps a live store's decoded lists only from `LIVE_HOT_MIN_LEN` postings and within
/// `LIVE_HOT_POSTINGS`, and every ranking, cold or from kept lists, is the reference scan's, also after a
/// mutation of the store.
#[test]
fn live_hot_lists_rank_as_the_reference_scan_at_their_limits() {
    const DOCUMENTS: usize = 2100;
    let query = vec![0.3, -0.2, 0.5, 0.1, 0.4, -0.3, 0.2, 0.25];
    let options = SearchOptions {
        k: Some(9),
        candidate_limit: Some(10),
        embedding: Some(query),
        min_semantic_score: Some(-1.0),
        ..Default::default()
    };
    let text = "alpha beta common delta";
    // The budget left after beta and common is 2200 - (16 + 2100) = 84 postings.
    for (alpha, beta, delta) in [
        (LIVE_HOT_MIN_LEN - 1, LIVE_HOT_MIN_LEN, 83),
        (LIVE_HOT_MIN_LEN, LIVE_HOT_MIN_LEN + 1, 84),
        (LIVE_HOT_MIN_LEN + 1, LIVE_HOT_MIN_LEN - 1, 85),
    ] {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut store = DocumentStore::default();
        let document = |id: usize, state: &mut u64| {
            let mut words = format!("common rare{id}");
            for (name, count) in [("alpha", alpha), ("beta", beta), ("delta", delta)] {
                if id < count {
                    words.push_str(&format!(" {name}"));
                }
            }
            chunk_document(id, &words, state)
        };
        for id in 0..DOCUMENTS {
            let document = document(id, &mut state);
            store.insert(document.id.clone(), document);
        }
        let kept = || LEXICAL_SCRATCH.with(|scratch| scratch.borrow().hot.postings);
        let check = |store: &DocumentStore, label: &str| {
            let expected = serde_json::to_value(
                super::super::search_impl(store, text, &options, Some(8), false).unwrap(),
            )
            .unwrap();
            for pass in 0..3 {
                let actual = serde_json::to_value(
                    super::super::search_impl(store, text, &options, Some(8), true).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    actual, expected,
                    "{label} {alpha} {beta} {delta} pass {pass}"
                );
            }
        };
        LEXICAL_SCRATCH.with(|scratch| scratch.take());
        check(&store, "built");
        let admitted = |count: usize| if count >= LIVE_HOT_MIN_LEN { count } else { 0 };
        let first = admitted(alpha)
            + admitted(beta)
            + DOCUMENTS
            + if delta <= LIVE_HOT_POSTINGS - (admitted(alpha) + admitted(beta) + DOCUMENTS) {
                delta
            } else {
                0
            };
        assert_eq!(kept(), first, "{alpha} {beta} {delta}");
        let document = document(DOCUMENTS, &mut state);
        store.insert(document.id.clone(), document);
        check(&store, "grown");
    }
}

/// The screen table serves from `SCREEN_TABLE_MIN_CHUNKS` chunks (the second query of a generation builds it) and
/// ranks identically to the exact scan on both sides of that size.
#[test]
fn screen_table_switches_on_at_the_chunk_threshold() {
    let single = |id: usize| {
        let input = serde_json::from_value(json!({
            "id": format!("s{id:04}"), "version": "1", "text": "running common",
            "chunkWords": 3, "overlapWords": 0,
            "embeddings": [[0.1 * id as f64 + 1.0, 0.5, -0.2, 0.3, 0.1, 0.9, -0.4, 0.2]],
        }))
        .unwrap();
        super::super::prepare(input, 1024, Some(8), TextEncoder::default()).unwrap()
    };
    let options = SearchOptions {
        k: Some(9),
        candidate_limit: Some(10),
        embedding: Some(vec![0.3, -0.2, 0.5, 0.1, 0.4, -0.3, 0.2, 0.25]),
        min_semantic_score: Some(-1.0),
        ..Default::default()
    };
    for (extra, served) in [(1, false), (2, true), (3, true)] {
        let mut store = noisy_store(1023, 0);
        for id in 0..extra {
            let document = single(id);
            store.insert(document.id.clone(), document);
        }
        assert_eq!(store.chunk_count, 2046 + extra, "{extra}");
        let built: Vec<bool> = (0..3).map(|_| store.screen_table(8).is_some()).collect();
        assert_eq!(built, [false, served, served], "{extra}");
        let expected =
            super::super::search_impl(&store, "common", &options, Some(8), false).unwrap();
        let actual = super::super::search_impl(&store, "common", &options, Some(8), true).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{extra}"
        );
    }
}

/// The screen prunes: on random vectors far fewer chunks than the store holds are scored exactly, every score
/// returned is the exact cosine, and the exact top `limit` is among them.
#[test]
fn screening_scores_only_chunks_that_can_rank() {
    let store = noisy_store(600, 0);
    let query = normalize_embedding(&[0.3, -0.2, 0.5, 0.1, 0.4, -0.3, 0.2, 0.25], 8).unwrap();
    let options = SearchOptions {
        embedding: Some(query.clone()),
        ..Default::default()
    };
    let captured = capture_search(
        &store,
        vec![prepare_search("running", &options, Some(8)).unwrap()],
    );
    let chunks = &captured.queries[0].chunks;
    assert!(chunks.len() >= 1200);
    let limit = 50;
    let scores = semantic_scores(
        chunks,
        Some(&query),
        &SemanticNeed {
            reported: &[],
            list: Some((limit, -1.0)),
            survivors: false,
        },
    );
    let scored = scores.iter().flatten().count();
    assert!(scored >= limit && scored < chunks.len() / 4, "{scored}");
    let mut exact: Vec<(f64, usize)> = chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            (
                chunk
                    .stored()
                    .unwrap()
                    .embedding
                    .as_ref()
                    .unwrap()
                    .cosine(&query),
                i,
            )
        })
        .collect();
    for &(score, i) in &exact {
        if let Some(got) = scores[i] {
            assert_eq!(got.to_bits(), score.to_bits());
        }
    }
    exact.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(chunks[a.1].order().cmp(&chunks[b.1].order()))
    });
    assert!(exact[..limit].iter().all(|&(_, i)| scores[i].is_some()));
}

#[test]
fn lexical_capture_holds_only_ranked_candidates_and_shares_embeddings() {
    let store = store(120);
    for (text, limit, held) in [("rare7", 100, 1), ("common", 100, 100), ("common", 7, 7)] {
        let options = SearchOptions {
            k: Some(limit),
            candidate_limit: Some(limit),
            ..Default::default()
        };
        let captured = capture_search(
            &store,
            vec![prepare_search(text, &options, Some(3)).unwrap()],
        );
        let query = &captured.queries[0];
        assert_eq!(query.count, 240);
        assert_eq!(query.chunks.len(), held);
        assert_eq!(query.lexical_scores.as_ref().map(Vec::len), Some(held));
        assert!(query.postings.iter().all(BTreeMap::is_empty));
        for chunk in &query.chunks {
            assert!(Arc::ptr_eq(
                chunk.document(),
                &store.documents[&chunk.document().id]
            ));
        }
        assert_eq!(
            serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
            serde_json::to_value(
                super::super::search_impl(&store, text, &options, Some(3), false).unwrap()
            )
            .unwrap()
        );
    }
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A document of 1 to 12 words from a small vocabulary (so that scores tie), chunked 1 to
/// 4 words apiece; a third keep unstemmed terms and a quarter carry a zero-frequency term.
fn random_document(id: usize, state: &mut u64) -> StoredDocument {
    const WORDS: [&str; 9] = [
        "alpha", "beta", "gamma", "delta", "running", "runs", "ran", "shared", "Shared",
    ];
    let template = id.is_multiple_of(5);
    let mut seed = if template { 7 } else { xorshift(state) };
    let count = 1 + (xorshift(&mut seed) % 12) as usize;
    let text: Vec<&str> = (0..count)
        .map(|_| WORDS[(xorshift(&mut seed) % WORDS.len() as u64) as usize])
        .collect();
    let input = serde_json::from_value(json!({
        "id": format!("d{id:04}"), "text": text.join(" "),
        "chunkWords": 1 + xorshift(state) % 4, "overlapWords": 0,
    }))
    .unwrap();
    let mut document = super::super::prepare(input, 1024, None, TextEncoder::default()).unwrap();
    for chunk in &mut document.chunks {
        if id.is_multiple_of(3) {
            chunk.terms =
                crate::core::documents::terms(chunk.text.as_deref().unwrap(), false).into();
            chunk.stemmed = false;
        }
        if id % 4 == 1 {
            chunk.terms.insert("ghost".into(), 0);
        }
    }
    document
}

/// Lexical hits of the ranked capture and of the exhaustive posting capture, as JSON.
fn ranked_and_exhaustive(
    store: &DocumentStore,
    text: &str,
    options: &SearchOptions,
) -> (Value, Value) {
    let ranked = capture_search(store, vec![prepare_search(text, options, None).unwrap()]);
    assert!(ranked.queries[0].lexical_scores.is_some());
    let exhaustive = CapturedSearch {
        queries: vec![capture_postings(
            store,
            prepare_search(text, options, None).unwrap(),
        )],
    };
    (
        serde_json::to_value(execute_search(&ranked, 0).unwrap()).unwrap(),
        serde_json::to_value(execute_search(&exhaustive, 0).unwrap()).unwrap(),
    )
}

fn assert_ranked_matches_exhaustive(store: &DocumentStore) {
    let bm25 = [
        (None, None),
        (Some(0.0), Some(0.0)),
        (Some(0.0), Some(1.0)),
        (Some(1.0), Some(1.0)),
        (Some(2.5), Some(0.3)),
    ];
    for text in [
        "gamma",
        "alpha beta",
        "ran shared ghost",
        "ghost",
        "running runs alpha alpha delta",
        "missing",
        "",
    ] {
        for (k1, b) in bm25 {
            let all = SearchOptions {
                k: Some(1000),
                candidate_limit: Some(4096),
                bm25_k1: k1,
                bm25_b: b,
                ..Default::default()
            };
            let (ranked, exhaustive) = ranked_and_exhaustive(store, text, &all);
            assert_eq!(ranked, exhaustive, "{text:?} k1 {k1:?} b {b:?}");
            let matched = exhaustive.as_array().unwrap().len() as u32;
            for limit in [1, 2, 5, matched.saturating_sub(1), matched, matched + 1] {
                let limit = limit.max(1);
                for (k, fusion) in [(limit.min(1000), "blend"), (limit.min(3), "rrf")] {
                    let options = SearchOptions {
                        k: Some(k),
                        candidate_limit: Some(limit),
                        fusion: Some(fusion.into()),
                        ..all.clone()
                    };
                    let (ranked, exhaustive) = ranked_and_exhaustive(store, text, &options);
                    assert_eq!(ranked, exhaustive, "{text:?} limit {limit} {fusion}");
                }
            }
        }
    }
}

/// The ranked lexical capture returns exactly what the exhaustive posting capture returns
/// (hits, order, bit-identical scores) through ties, zero-frequency terms, mixed
/// tokenization, multi-chunk documents, deletes, replacements with slot reuse, a registry
/// rebuild, stores of equal generation, BM25 constants at their bounds, limits around the
/// match count, and the scratch's recovery and epoch wrap.
#[test]
fn ranked_lexical_capture_matches_the_exhaustive_capture() {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut store = DocumentStore::default();
    for id in 0..60 {
        let document = random_document(id, &mut state);
        store.insert(document.id.clone(), document);
    }
    assert_ranked_matches_exhaustive(&store);
    for id in (0..60).step_by(3) {
        assert!(store.remove(&format!("d{id:04}")).is_some());
    }
    assert_ranked_matches_exhaustive(&store);
    // Replacements: same chunk count (in place) and a different one (fresh slots), then
    // new documents in the freed slots.
    for id in [1, 2, 4, 5, 7] {
        let mut document = random_document(id + 100, &mut state);
        document.id = format!("d{id:04}");
        for (index, chunk) in document.chunks.iter_mut().enumerate() {
            chunk.id = format!("{}{}:d{id:04}:{index}", super::super::CHUNK_PREFIX, 5);
        }
        store.insert(document.id.clone(), document);
    }
    for id in 60..80 {
        let document = random_document(id, &mut state);
        store.insert(document.id.clone(), document);
    }
    assert_ranked_matches_exhaustive(&store);

    // A query that panicked midway leaves the scratch dirty; the next query resets it.
    LEXICAL_SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        scratch.scores.fill(3.5);
        scratch.dirty = true;
    });
    assert_ranked_matches_exhaustive(&store);
    // The norm epoch wraps (every change of store state or BM25 constants advances it):
    // norms of earlier states must not survive the wrap.
    LEXICAL_SCRATCH.with(|scratch| scratch.borrow_mut().epoch = u32::MAX - 1);
    let document = random_document(80, &mut state);
    store.insert(document.id.clone(), document);
    assert_ranked_matches_exhaustive(&store);
    assert!(LEXICAL_SCRATCH.with(|scratch| scratch.borrow().epoch) < 1000);
    assert!(store.remove("d0080").is_some());
    assert_ranked_matches_exhaustive(&store);

    // Two stores at the same generation, queried alternately, keep their own norms.
    let mut other = DocumentStore::default();
    let mut long_state = 0x9E37_79B9_7F4A_7C15u64;
    while other.generation < store.generation {
        let id = other.generation as usize + 1000;
        let mut document = random_document(id, &mut long_state);
        for chunk in &mut document.chunks {
            chunk.word_count += 7;
        }
        other.insert(document.id.clone(), document);
    }
    assert_eq!(other.generation, store.generation);
    for _ in 0..2 {
        assert_ranked_matches_exhaustive(&other);
        assert_ranked_matches_exhaustive(&store);
    }

    // Deleting most of a large store rebuilds its registry (new slots, new identity).
    let mut large = DocumentStore::default();
    for id in 0..1100 {
        let mut document = random_document(id, &mut state);
        document.chunks.truncate(1);
        large.insert(document.id.clone(), document);
    }
    assert_ranked_matches_exhaustive(&large);
    let identity = large.identity;
    for id in 0..700 {
        large.remove(&format!("d{id:04}"));
    }
    assert_ne!(large.identity, identity, "registry rebuilt");
    assert_ranked_matches_exhaustive(&large);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(32))]
    /// The ranked capture returns what the exhaustive capture returns, bit for bit, and
    /// counts agree with the chunks, at limits far below the posting count, before and
    /// after deletes, replacements and slot reuse.
    #[test]
    fn ranking_and_counts_match_exhaustive_under_churn(
        seed in proptest::prelude::any::<u64>(),
        documents in 20usize..90,
        limit in 1u32..9,
        churn in 2usize..9,
    ) {
        let mut state = seed | 1;
        let mut store = DocumentStore::default();
        for id in 0..documents {
            let document = random_document(id, &mut state);
            store.insert(document.id.clone(), document);
        }
        let check = |store: &DocumentStore| {
            for text in ["gamma", "alpha beta", "ran shared ghost", "running runs alpha delta"] {
                let options = SearchOptions {
                    k: Some(limit.min(3)),
                    candidate_limit: Some(limit),
                    ..Default::default()
                };
                let (ranked, exhaustive) = ranked_and_exhaustive(store, text, &options);
                assert_eq!(ranked, exhaustive, "{text:?} limit {limit}");
                let matched = matching_documents(store, text);
                assert_eq!(count_matches(store, text, Analyzer::Legacy).unwrap(), matched, "{text:?}");
            }
        };
        check(&store);
        for id in (0..documents).step_by(churn) {
            store.remove(&format!("d{id:04}"));
        }
        check(&store);
        for id in (1..documents).step_by(churn + 1) {
            let mut document = random_document(id + documents, &mut state);
            document.id = format!("d{id:04}");
            for (index, chunk) in document.chunks.iter_mut().enumerate() {
                chunk.id = format!("{}{}:d{id:04}:{index}", super::super::CHUNK_PREFIX, 5);
            }
            store.insert(document.id.clone(), document);
        }
        check(&store);
    }
}

/// A checkpoint of a store with zero-frequency terms, free slots and both posting forms
/// ranks from its mapping, and rebuilds into a store whose block bounds are a fresh
/// build's, exactly as the store that wrote it ranks.
#[test]
fn checkpoint_ranks_and_rebuilds_as_the_store_that_wrote_it() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut store = DocumentStore::default();
    for id in 0..300 {
        let mut document = random_document(id, &mut state);
        for chunk in &mut document.chunks {
            chunk.vector_state = super::super::VectorState::Lexical;
            chunk.vector = crate::core::entangled::EntangledHVec::from_indices(Vec::new(), 1024);
        }
        store.insert(document.id.clone(), document);
    }
    for id in (0..300).step_by(7) {
        store.remove(&format!("d{id:04}"));
    }
    let bytes = super::super::image::encode(&store, 1, 1024)
        .unwrap()
        .unwrap();
    let image = ServingImage::new(super::super::Backing::Owned(bytes)).unwrap();
    let rebuilt = image.hydrate(None).unwrap();
    let words: Vec<u32> = (0..rebuilt.slot_count)
        .map(|slot| rebuilt.slot_word_count(slot))
        .collect();
    for postings in &rebuilt.postings {
        postings.assert_valid();
    }
    for text in [
        "gamma",
        "alpha beta",
        "ran shared ghost",
        "running runs alpha delta",
    ] {
        for (form, terms) in [
            crate::core::documents::terms(text, false),
            crate::core::documents::terms(text, true),
        ]
        .iter()
        .enumerate()
        {
            for word in terms.keys() {
                if let Some(list) = rebuilt.postings[form].get(word) {
                    assert!(list.bounds_are_tight(&words), "{word} bounds");
                }
            }
        }
        let options = SearchOptions {
            k: Some(10),
            ..Default::default()
        };
        let expected = ranked_and_exhaustive(&store, text, &options).0;
        assert_eq!(ranked_and_exhaustive(&rebuilt, text, &options).0, expected);
        let mapped = CapturedSearch {
            queries: vec![
                capture_image(&image, prepare_search(text, &options, None).unwrap()).unwrap(),
            ],
        };
        assert_eq!(
            serde_json::to_value(execute_search(&mapped, 0).unwrap()).unwrap(),
            expected,
            "{text:?}"
        );
    }
}

/// The lines of the repository changelog as one-chunk lexical documents, `every`th line
/// from `first`: real prose, whose common words have the long posting lists a thread keeps
/// decoded.
fn prose_store(first: usize, every: usize) -> DocumentStore {
    let mut store = DocumentStore::default();
    let lines = include_str!("../../../../CHANGELOG.md")
        .lines()
        .filter(|line| line.len() >= 20);
    for (id, line) in lines.enumerate().skip(first).step_by(every) {
        let input = serde_json::from_value(json!({
            "id": format!("p{id:04}"), "text": line, "chunkWords": 4096, "overlapWords": 0,
        }))
        .unwrap();
        let mut document =
            super::super::prepare(input, 1024, None, TextEncoder::default()).unwrap();
        for chunk in &mut document.chunks {
            chunk.vector_state = super::super::VectorState::Lexical;
            chunk.vector = crate::core::entangled::EntangledHVec::from_indices(Vec::new(), 1024);
        }
        store.insert(document.id.clone(), document);
    }
    store
}

/// Long posting lists decoded and weighted once and reused across queries rank real prose
/// exactly as the exhaustive posting capture does, in the queries that build them, in the
/// ones that reuse them and after the checkpoint under the thread changes.
#[test]
fn decoded_hot_lists_rank_prose_as_the_exhaustive_capture() {
    let queries: Vec<String> = include_str!("../../../../CHANGELOG.md")
        .lines()
        .filter(|line| line.len() >= 20)
        .step_by(5)
        .take(40)
        .map(str::to_owned)
        .chain(["the", "the of and to in", "fix the cache and the index"].map(str::to_owned))
        .collect();
    let options = [
        SearchOptions {
            k: Some(10),
            ..Default::default()
        },
        SearchOptions {
            k: Some(100),
            candidate_limit: Some(100),
            bm25_k1: Some(1.6),
            bm25_b: Some(0.4),
            ..Default::default()
        },
    ];
    // Version 1 (flat postings) is still read: it serves, and rebuilds the store, as the
    // version written does.
    let versions = [
        super::super::image::encode,
        super::super::image::encode_v3,
        super::super::image::encode_v2,
        super::super::image::encode_v1,
    ];
    for ((first, every), encode) in [(0, 1), (1, 2)]
        .into_iter()
        .flat_map(|layout| versions.map(|encode| (layout, encode)))
    {
        let store = prose_store(first, every);
        let written = super::super::image::encode(&store, 1, 1024)
            .unwrap()
            .unwrap();
        let bytes = encode(&store, 1, 1024).unwrap().unwrap();
        let image = ServingImage::new(super::super::Backing::Owned(bytes)).unwrap();
        let rebuilt = image.hydrate(None).unwrap();
        assert_eq!(
            super::super::image::encode(&rebuilt, 1, 1024)
                .unwrap()
                .unwrap(),
            written
        );
        for round in 0..3 {
            for options in &options {
                for text in &queries {
                    let expected = ranked_and_exhaustive(&store, text, options).1;
                    let mapped = CapturedSearch {
                        queries: vec![capture_image(
                            &image,
                            prepare_search(text, options, None).unwrap(),
                        )
                        .unwrap()],
                    };
                    assert_eq!(
                        serde_json::to_value(execute_search(&mapped, 0).unwrap()).unwrap(),
                        expected,
                        "round {round} {text:?}"
                    );
                }
            }
            // A version 3 checkpoint is pruned, not decoded into hot lists.
            if image.has_impacts() {
                continue;
            }
            let kept = LEXICAL_SCRATCH.with(|scratch| {
                let scratch = scratch.borrow();
                (
                    scratch.hot.postings,
                    scratch.hot.lists.iter().map(|l| l.len()).sum::<usize>(),
                )
            });
            assert!(
                kept.0 > 0 && kept.0 <= HOT_POSTINGS && kept.1 > 0,
                "{kept:?}"
            );
        }
    }
}

/// A store of `documents` one-chunk documents whose words follow a heavy-tailed
/// distribution over `vocabulary` terms, with lengths from 3 to 60 words, so some posting
/// lists run to many blocks and chunk lengths differ.
fn heavy_tailed_store(documents: usize, vocabulary: u64, state: &mut u64) -> DocumentStore {
    let mut store = DocumentStore::default();
    for id in 0..documents {
        let length = 3 + xorshift(state) % 58;
        let text: Vec<String> = (0..length)
            .map(|_| {
                let u = (xorshift(state) % 1_000_000) as f64 / 1_000_000.0;
                format!("w{}", ((vocabulary as f64).powf(u) as u64).min(vocabulary))
            })
            .collect();
        let input = serde_json::from_value(json!({
            "id": format!("h{id:06}"), "text": text.join(" "), "chunkWords": 4096, "overlapWords": 0,
        }))
        .unwrap();
        let mut document =
            super::super::prepare(input, 1024, None, TextEncoder::default()).unwrap();
        for chunk in &mut document.chunks {
            chunk.vector_state = super::super::VectorState::Lexical;
            chunk.vector = crate::core::entangled::EntangledHVec::from_indices(Vec::new(), 1024);
        }
        store.insert(document.id.clone(), document);
    }
    store
}

/// Block-max pruned ranking from a version 3 checkpoint returns the chunks, the scores to
/// the bit and the order the exhaustive ranking of a version 2 one does, for queries of one
/// to thirty terms at several depths and BM25 settings, and decodes fewer blocks than the
/// lists hold.
#[test]
fn pruned_ranking_matches_exhaustive_ranking_to_the_bit() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let stores = [
        prose_store(0, 1),
        heavy_tailed_store(6000, 400, &mut state),
        heavy_tailed_store(40, 6, &mut state),
    ];
    let options = [
        SearchOptions {
            k: Some(10),
            ..Default::default()
        },
        SearchOptions {
            k: Some(100),
            candidate_limit: Some(100),
            ..Default::default()
        },
        SearchOptions {
            k: Some(3),
            candidate_limit: Some(5),
            bm25_k1: Some(1.6),
            bm25_b: Some(0.4),
            ..Default::default()
        },
        SearchOptions {
            k: Some(50),
            candidate_limit: Some(50),
            bm25_k1: Some(0.0),
            bm25_b: Some(1.0),
            ..Default::default()
        },
    ];
    let (mut listed, mut decoded, mut scored_total, mut queries_run) = (0, 0, 0, 0);
    for (index, store) in stores.iter().enumerate() {
        type Encoder = fn(&DocumentStore, u32, usize) -> Result<Option<Vec<u8>>>;
        let build = |encode: Encoder| {
            ServingImage::new(super::super::Backing::Owned(
                encode(store, 1, 1024).unwrap().unwrap(),
            ))
            .unwrap()
        };
        let (pruned, plain) = (
            build(super::super::image::encode),
            build(super::super::image::encode_v2),
        );
        assert!(pruned.has_impacts() && !plain.has_impacts());
        let texts: Vec<String> = match index {
            0 => include_str!("../../../../CHANGELOG.md")
                .lines()
                .filter(|line| line.len() >= 20)
                .step_by(4)
                .map(str::to_owned)
                .collect(),
            1 => (0..300)
                .map(|_| {
                    let terms = 1 + xorshift(&mut state) % 30;
                    (0..terms)
                        .map(|_| {
                            let u = (xorshift(&mut state) % 1_000_000) as f64 / 1_000_000.0;
                            format!("w{}", (400f64.powf(u) as u64).min(400))
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect(),
            _ => (0..40)
                .map(|i| format!("w1 w2 w{} w{} nonsense", i % 7, (i * 3) % 7))
                .collect(),
        };
        for text in &texts {
            for options in &options {
                let prepared = prepare_search(text, options, None).unwrap();
                let (fast, exact, stats) = LEXICAL_SCRATCH.with(|scratch| {
                    let mut scratch = scratch.borrow_mut();
                    let fast = scratch.rank(&pruned, &prepared, true);
                    let stats = scratch.pruned.stats;
                    let exact = scratch.rank(&plain, &prepared, true);
                    (fast, exact, stats)
                });
                let bits = |ranked: &[(f64, u32)]| -> Vec<(u64, u32)> {
                    ranked
                        .iter()
                        .map(|&(s, slot)| (s.to_bits(), slot))
                        .collect()
                };
                assert_eq!(bits(&fast), bits(&exact), "{text:?}");
                listed += stats.blocks_listed;
                decoded += stats.blocks_decoded;
                scored_total += stats.scored;
                queries_run += 1;
            }
        }
    }
    assert!(queries_run > 1000 && scored_total > 0);
    assert!(
        decoded < listed,
        "pruning decoded {decoded} of {listed} blocks"
    );
}

/// The windowed ranker's block decoder and frequency tables at the block boundary: a list
/// of 127, 128 or 129 postings (and the same plus one block) with frequencies running past
/// the table's escape value ranks to the bit as the exhaustive ranking does.
#[test]
fn pruned_ranking_matches_exhaustive_at_block_boundaries() {
    let options = [
        SearchOptions {
            k: Some(10),
            ..Default::default()
        },
        SearchOptions {
            k: Some(100),
            candidate_limit: Some(100),
            ..Default::default()
        },
    ];
    for documents in [127usize, 128, 129, 255, 256, 257] {
        let mut store = DocumentStore::default();
        for id in 0..documents {
            let repeats = 1 + id % 20;
            let filler = 1 + (id * 7) % 11;
            let mut words = vec!["edge"; repeats];
            words.extend(std::iter::repeat_n("pad", filler));
            if id % 3 == 0 {
                words.push("common");
            }
            let input = serde_json::from_value(json!({
                "id": format!("b{id:04}"), "text": words.join(" "), "chunkWords": 4096, "overlapWords": 0,
            }))
            .unwrap();
            let mut document =
                super::super::prepare(input, 1024, None, TextEncoder::default()).unwrap();
            for chunk in &mut document.chunks {
                chunk.vector_state = super::super::VectorState::Lexical;
                chunk.vector =
                    crate::core::entangled::EntangledHVec::from_indices(Vec::new(), 1024);
            }
            store.insert(document.id.clone(), document);
        }
        type Encoder = fn(&DocumentStore, u32, usize) -> Result<Option<Vec<u8>>>;
        let build = |encode: Encoder| {
            ServingImage::new(super::super::Backing::Owned(
                encode(&store, 1, 1024).unwrap().unwrap(),
            ))
            .unwrap()
        };
        let (pruned, plain) = (
            build(super::super::image::encode),
            build(super::super::image::encode_v2),
        );
        for text in [
            "edge",
            "edge pad",
            "edge common",
            "common pad edge edge",
            "pad",
        ] {
            for options in &options {
                let prepared = prepare_search(text, options, None).unwrap();
                let (fast, exact) = LEXICAL_SCRATCH.with(|scratch| {
                    let mut scratch = scratch.borrow_mut();
                    let fast = scratch.rank(&pruned, &prepared, true);
                    (fast, scratch.rank(&plain, &prepared, true))
                });
                let bits = |ranked: &[(f64, u32)]| -> Vec<(u64, u32)> {
                    ranked
                        .iter()
                        .map(|&(s, slot)| (s.to_bits(), slot))
                        .collect()
                };
                assert_eq!(bits(&fast), bits(&exact), "{documents} documents, {text:?}");
            }
        }
    }
}
/// The cost model switches to the pruned ranker exactly above the serving floor, and every
/// store too large for a dense slot table is above it.
#[test]
fn pruning_switches_on_above_the_serving_floor() {
    let floor = PRUNE_SLOT_FLOOR_SERVING;
    assert!(!prune_above(floor - 1, floor));
    assert!(!prune_above(floor, floor));
    assert!(prune_above(floor + 1, floor));
    assert!(DENSE_SLOT_LIMIT > floor);
}
/// Reports how much of a checkpoint's posting lists block-max pruning reads per query:
/// `WARM_PRUNE_CKPT` the `documents.ckpt`, `WARM_PRUNE_QUERIES` the harness query file.
#[test]
#[ignore = "measurement: needs WARM_PRUNE_CKPT and WARM_PRUNE_QUERIES"]
fn pruning_work_report() {
    let (Ok(ckpt), Ok(queries)) = (
        std::env::var("WARM_PRUNE_CKPT"),
        std::env::var("WARM_PRUNE_QUERIES"),
    ) else {
        return;
    };
    let image =
        ServingImage::new(super::super::Backing::Owned(std::fs::read(ckpt).unwrap())).unwrap();
    let queries: Vec<Value> = serde_json::from_slice(&std::fs::read(queries).unwrap()).unwrap();
    let options = SearchOptions {
        k: Some(100),
        candidate_limit: Some(100),
        ..Default::default()
    };
    let mut total = [0usize; 10];
    for query in &queries {
        let text = query["text"].as_str().unwrap();
        let prepared = prepare_search(text, &options, None).unwrap();
        let stats = LEXICAL_SCRATCH.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            scratch.rank(&image, &prepared, true);
            scratch.pruned.stats
        });
        total[0] += stats.listed;
        total[1] += stats.blocks_listed;
        total[2] += stats.blocks_decoded;
        total[3] += stats.scored;
        total[4] += stats.windows;
        total[6] += stats.accumulated;
        total[7] += stats.survivors;
        total[8] += stats.table_lookups;
        total[9] += stats.cursor_lookups;
    }
    let n = queries.len() as f64;
    eprintln!(
        "queries {} per query: postings listed {:.0} blocks listed {:.0} blocks decoded {:.0} chunks scored {:.0} windows {:.1} accumulated {:.0} survivors {:.0} table lookups {:.0} cursor lookups {:.0}",
        queries.len(),
        total[0] as f64 / n,
        total[1] as f64 / n,
        total[2] as f64 / n,
        total[3] as f64 / n,
        total[4] as f64 / n,
        total[6] as f64 / n,
        total[7] as f64 / n,
        total[8] as f64 / n,
        total[9] as f64 / n
    );
}

/// Rebuilding a checkpoint without the postings of its dead documents and then removing
/// them leaves the store byte for byte as rebuilding everything and removing them does,
/// before and after more writes, below and above the slot count at which a removal re-slots
/// the whole store.
#[test]
fn hydrating_without_dead_postings_matches_removing_them() {
    for (documents, dead_every, keep_every) in [(300, 7, 1), (1500, 1, 3)] {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut store = DocumentStore::default();
        for id in 0..documents {
            let mut document = random_document(id, &mut state);
            for chunk in &mut document.chunks {
                chunk.vector_state = super::super::VectorState::Lexical;
                chunk.vector =
                    crate::core::entangled::EntangledHVec::from_indices(Vec::new(), 1024);
            }
            store.insert(document.id.clone(), document);
        }
        let bytes = super::super::image::encode(&store, 1, 1024)
            .unwrap()
            .unwrap();
        let image = ServingImage::new(super::super::Backing::Owned(bytes)).unwrap();
        // Every `dead_every`-th document, sparing every `keep_every`-th of those, in id order
        // as the overlay lists them.
        let dead: Vec<String> = (0..documents)
            .step_by(dead_every)
            .enumerate()
            .filter(|(i, _)| keep_every == 1 || i % keep_every != 0)
            .map(|(_, id)| format!("d{id:04}"))
            .collect();
        let mut removed = image.hydrate(None).unwrap();
        for id in &dead {
            removed.remove_scrubbed(id).unwrap();
        }
        let mut excluded = image.hydrate_excluding(None, &dead).unwrap();
        for id in &dead {
            excluded.remove_unindexed_scrubbed(id).unwrap();
        }
        let encode = |store: &DocumentStore| {
            super::super::image::encode(store, 1, 1024)
                .unwrap()
                .expect("lexical-only store")
        };
        assert_eq!(encode(&excluded), encode(&removed));
        for id in documents..documents + 40 {
            let document = random_document(id, &mut state);
            for store in [&mut removed, &mut excluded] {
                let mut document = document.clone();
                for chunk in &mut document.chunks {
                    chunk.vector_state = super::super::VectorState::Lexical;
                    chunk.vector =
                        crate::core::entangled::EntangledHVec::from_indices(Vec::new(), 1024);
                }
                store.insert(document.id.clone(), document);
            }
        }
        for postings in &excluded.postings {
            postings.assert_valid();
        }
        assert_eq!(encode(&excluded), encode(&removed));
    }
}

/// Documents with a chunk holding a term of `text` at a positive frequency, read from the
/// chunks themselves.
fn matching_documents(store: &DocumentStore, text: &str) -> usize {
    let terms = [
        crate::core::documents::terms(text, false),
        crate::core::documents::terms(text, true),
    ];
    store
        .documents
        .values()
        .filter(|document| {
            document.chunks.iter().any(|chunk| {
                terms[usize::from(chunk.stemmed)]
                    .keys()
                    .any(|word| chunk.terms.get(word).is_some_and(|&f| f > 0))
            })
        })
        .count()
}

#[test]
fn captured_view_survives_registry_rebuild_without_payload_copy() {
    for count in [1023, 1024, 1025] {
        let mut store = DocumentStore::default();
        for id in 0..count {
            let mut document = document(id, 1);
            document.chunks.truncate(1);
            store.insert(document.id.clone(), document);
        }
        let text = format!("rare{}", count - 1);
        let options = SearchOptions::default();
        let captured = capture_search(
            &store,
            vec![prepare_search(&text, &options, Some(3)).unwrap()],
        );
        let expected = serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap();
        for id in 0..count - 1 {
            store.remove(&format!("d{id:03}"));
        }
        assert!(store.slot_count <= 1024);
        assert!(Arc::ptr_eq(
            captured.queries[0].chunks[0].document(),
            &store.documents[&format!("d{:03}", count - 1)]
        ));
        assert_eq!(
            serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn captured_variants_remain_coherent_during_concurrent_publication() {
    for (count, indexed) in [(16, false), (128, true)] {
        let mut store = store(count);
        if indexed {
            let trained = super::super::build_dense_index(
                super::super::capture_dense_index(&store, 3)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap()
            .unwrap();
            assert!(super::super::publish_dense_index(&mut store, &trained));
        }
        let store = RwLock::new(store);
        let variants = variants();
        let expected = scanned_variants(&store.read(), &variants);
        let captured = capture_variants(&store.read(), &variants);
        let barrier = Barrier::new(2);
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                barrier.wait();
                for revision in 2..=9 {
                    for id in 0..count {
                        let document = document(id, revision);
                        let mut store = store.write();
                        store.remove(&document.id);
                        store.insert(document.id.clone(), document);
                    }
                }
            });
            barrier.wait();
            for _ in 0..8 {
                for (i, expected) in expected.iter().enumerate() {
                    assert_eq!(
                        serde_json::to_value(execute_search(&captured, i).unwrap()).unwrap(),
                        *expected
                    );
                }
            }
            writer.join().unwrap();
        });
        let current = capture_search(
            &store.read(),
            vec![prepare_search("generation9", &SearchOptions::default(), Some(3)).unwrap()],
        );
        assert!(execute_search(&current, 0)
            .unwrap()
            .iter()
            .all(|result| result.version == "9"));
        assert!(execute_search(&captured, 0)
            .unwrap()
            .iter()
            .all(|result| result.version == "1"));
        assert_eq!(
            captured.queries.iter().any(|query| query.dense.is_some()),
            indexed
        );
    }
}

#[test]
fn combined_weight_boundary_is_finite_and_overflow_is_rejected() {
    let half = f64::MAX / 2.0;
    let below = f64::from_bits(half.to_bits() - 1);
    let above = f64::from_bits(half.to_bits() + 1);
    let store = store(12);
    for lexical_weight in [below, half] {
        let options = SearchOptions {
            lexical_weight: Some(lexical_weight),
            semantic_weight: Some(half),
            embedding: Some(vec![1.0, 0.0, 0.0]),
            ..Default::default()
        };
        let captured = capture_search(
            &store,
            vec![prepare_search("running", &options, Some(3)).unwrap()],
        );
        assert!(execute_search(&captured, 0)
            .unwrap()
            .iter()
            .all(|result| result.score.is_finite()));
    }
    for (lexical_weight, semantic_weight) in [(above, half), (f64::MAX, f64::MAX)] {
        let options = SearchOptions {
            lexical_weight: Some(lexical_weight),
            semantic_weight: Some(semantic_weight),
            ..Default::default()
        };
        assert!(prepare_search("running", &options, Some(3)).is_err());
    }
    assert!(capture_search(&store, Vec::new()).queries.is_empty());
}
