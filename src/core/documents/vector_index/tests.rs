// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::core::documents::{prepare, DocumentInput};
use crate::core::encoding::TextEncoder;
use serde_json::json;
use std::cell::RefCell;
use std::sync::{atomic::AtomicBool, atomic::Ordering as AtomicOrdering, mpsc};
use std::time::Duration;

thread_local! {
    static CANONICAL_HOOK: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static STALE_GRAPH_HOOK: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

pub(super) fn before_canonical_scoring() {
    if let Some(hook) = CANONICAL_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

pub(super) fn before_stale_graph_capture() {
    if let Some(hook) = STALE_GRAPH_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

fn matrix(rows: &[[f32; 4]]) -> ExactScan {
    let rows: Vec<Embedding> = rows
        .iter()
        .map(|row| {
            let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
            Embedding::from_f32(row.iter().map(|v| v / norm).collect())
        })
        .collect();
    ExactScan {
        dim: 4,
        rows: rows.into(),
    }
}

#[test]
fn graph_candidates_must_belong_to_snapshot() {
    assert!(validate_graph_candidates(&mut [0, 1], 2).is_ok());
    assert!(validate_graph_candidates(&mut [1, 2], 2).is_err());
    assert!(validate_graph_candidates(&mut [0], 0).is_err());
    assert!(validate_graph_candidates(&mut [], 0).is_ok());
    assert!(validate_graph_candidates(&mut [1, 0, 1], 2).is_err());
}

#[test]
fn high_recall_graph_adjacency_limit_is_checked_at_boundaries() {
    let limit = MAX_ROWS * 32 / 64;
    for rows in [limit - 1, limit, limit + 1] {
        let result = graph_build_params(true, rows);
        assert_eq!(result.is_ok(), rows <= limit);
        if let Ok(params) = result {
            assert_eq!((params.degree, params.build_ef), (64, 500));
        }
    }
    assert_eq!(
        (
            graph_build_params(false, MAX_ROWS).unwrap().degree,
            graph_build_params(false, MAX_ROWS).unwrap().build_ef
        ),
        (64, 256)
    );
    for rows in [MAX_ROWS - 1, MAX_ROWS, MAX_ROWS + 1] {
        assert_eq!(graph_build_params(false, rows).is_ok(), rows <= MAX_ROWS);
    }
    assert!(graph_build_params(true, usize::MAX).is_err());
}

#[test]
fn high_recall_graph_settings_apply_to_projected_candidates() {
    let mut store = DocumentStore::default();
    for row in 0..16 {
        put_vector(
            &mut store,
            &format!("d{row:03}"),
            "base",
            vec![vec![1.0, (row + 1) as f64 / 20.0]],
        );
    }
    let space = EmbeddingSpace {
        model: "fixture".into(),
        revision: "1".into(),
        dimensions: 2,
        normalization: "l2".into(),
        metric: "cosine".into(),
    };
    let candidates = VectorCandidateConfig {
        dimensions: 1,
        rerank_candidates: 8,
    };
    let capture = capture_vector_rows(&store, 2).unwrap().unwrap();
    let snapshot =
        build_configured_vector_snapshot(capture, 1, &space, Some(&candidates), true).unwrap();
    let status = vector_index_status(Some(&snapshot), &store);
    assert_eq!(
        (status.graph_degree, status.graph_build_ef),
        (Some(64), Some(500))
    );
    let store = RwLock::new(store);
    let result = search_vector_snapshot_with_report(
        &snapshot,
        &store,
        &[1.0, 0.0],
        &[1.0, 0.0],
        1,
        &VectorQueryOptions {
            mode: VectorIndexMode::Graph,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(result.diagnostics.route, VectorSearchRoute::ProjectedGraph);
    assert_eq!(
        (
            result.diagnostics.graph_degree,
            result.diagnostics.graph_build_ef
        ),
        (Some(64), Some(500))
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cached_graph_search_matches_fresh_scratch_after_graph_replacement() {
    for dimensions in [2, 4, 2] {
        let mut store = DocumentStore::default();
        for i in 0..32 {
            let mut values = vec![0.0; dimensions];
            values[0] = 1.0;
            values[1] = (i + 1) as f64 / 20.0;
            let input: DocumentInput = serde_json::from_value(json!({
                "id": format!("d{i:03}"), "text": "base", "embeddings": [values],
            }))
            .unwrap();
            let document = prepare(input, 1024, Some(dimensions), TextEncoder::default()).unwrap();
            store.insert(document.id.clone(), document);
        }
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, dimensions).unwrap().unwrap(), 1)
                .unwrap();
        let graph = &snapshot.graph.as_ref().unwrap().graph;
        for ef in [8, 16, 128, 1023, 1024, 1025, 8] {
            let mut query = vec![0.0f32; dimensions];
            query[0] = 1.0;
            query[1] = 0.5;
            let params = VSearchParams {
                ef,
                rerank: ef,
                ..Default::default()
            };
            let mut expected = Vec::new();
            let mut actual = Vec::new();
            graph
                .searcher()
                .search(&query, 5, params, &mut expected)
                .unwrap();
            graph.search_cached(&query, 5, params, &mut actual).unwrap();
            assert_eq!(actual, expected);
            assert!(graph
                .search_cached(&vec![0.0; dimensions], 5, params, &mut actual)
                .is_err());
            let mut invalid = query.clone();
            invalid[0] = f32::NAN;
            assert!(graph
                .search_cached(&invalid, 5, params, &mut actual)
                .is_err());
        }
    }
}

#[test]
fn exact_scan_orders_by_score_then_row_and_bounds_k() {
    let scan = matrix(&[
        [0.0, 1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [0.5, 0.5, 0.0, 0.0],
    ]);
    let hits = scan.search(&[1.0, 0.0, 0.0, 0.0], 3, 0).unwrap();
    assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(
        (hits[0].1 - 1.0).abs() < 1e-6
            && (hits[2].1 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3
    );
    assert_eq!(scan.search(&[1.0, 0.0, 0.0, 0.0], 10, 0).unwrap().len(), 4);
    assert!(scan.search(&[1.0, 0.0, 0.0], 1, 0).is_err());
}

#[test]
fn audit_regression_exact_snapshot_retains_all_canonical_rounding_winners() {
    for distractors in [8, 9, 10, 100] {
        let mut store = DocumentStore::default();
        for i in 0..=distractors {
            let input: DocumentInput = serde_json::from_value(json!({
                "id": if i == distractors { "z".to_owned() } else { format!("a{i:03}") },
                "text": "rounding regression",
                "embeddings": [[1.0, if i == distractors { 0.0001 } else { 0.0 }]],
            }))
            .unwrap();
            let document = prepare(input, 1024, Some(2), TextEncoder::default()).unwrap();
            store.insert(document.id.clone(), document);
        }
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
                .unwrap();
        let query = super::super::normalize_embedding(&[1.0, 0.0001], 2).unwrap();
        let rounded: Vec<f32> = query.iter().map(|&x| x as f32).collect();
        let store = RwLock::new(store);
        for minimum in [-1.0, 0.999_999_999] {
            let hits = search_vector_snapshot(
                &snapshot,
                &store,
                &rounded,
                &query,
                1,
                &VectorQueryOptions {
                    mode: VectorIndexMode::Exact,
                    min_score: minimum,
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                hits.len(),
                1,
                "{distractors} distractors, threshold {minimum}"
            );
            assert_eq!(hits[0].document_id, "z", "{distractors} distractors");
            assert_eq!(hits[0].semantic_score, Some(1.0));
        }
    }
}

fn vector_input(id: &str, text: &str, embeddings: Vec<Vec<f64>>) -> DocumentInput {
    serde_json::from_value(json!({
        "id": id,
        "text": text,
        "chunkWords": 1,
        "overlapWords": 0,
        "embeddings": embeddings,
    }))
    .unwrap()
}

fn put_vector(store: &mut DocumentStore, id: &str, text: &str, embeddings: Vec<Vec<f64>>) {
    let document = prepare(
        vector_input(id, text, embeddings),
        1024,
        Some(2),
        TextEncoder::default(),
    )
    .unwrap();
    store.insert(document.id.clone(), document);
}

fn delete_vector(store: &mut DocumentStore, id: &str) {
    assert!(store.remove(id).is_some());
}

fn snapshot_results(
    snapshot: &VectorSnapshot,
    store: &RwLock<DocumentStore>,
    query: &[f64],
    k: usize,
    min_score: f64,
) -> Vec<(String, String, f64)> {
    let normalized = super::super::normalize_embedding(query, 2).unwrap();
    let rounded: Vec<f32> = normalized.iter().map(|&value| value as f32).collect();
    search_vector_snapshot(
        snapshot,
        store,
        &rounded,
        &normalized,
        k,
        &VectorQueryOptions {
            mode: VectorIndexMode::Exact,
            min_score,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap()
    .into_iter()
    .map(|hit| (hit.document_id, hit.id, hit.semantic_score.unwrap()))
    .collect()
}

fn live_scan_results(
    store: &DocumentStore,
    query: &[f64],
    k: usize,
    min_score: f64,
) -> Vec<(String, String, f64)> {
    let normalized = super::super::normalize_embedding(query, 2).unwrap();
    let mut hits = Vec::new();
    for document in store.documents.values() {
        for (chunk, stored) in document.chunks.iter().enumerate() {
            let Some(embedding) = &stored.embedding else {
                continue;
            };
            if embedding.len() == 2 {
                let score = embedding.cosine(&normalized);
                if score >= min_score {
                    hits.push((document.id.clone(), chunk as u32, score));
                }
            }
        }
    }
    hits.sort_by(|a, b| {
        b.2.total_cmp(&a.2)
            .then_with(|| a.0.cmp(&b.0))
            .then(a.1.cmp(&b.1))
    });
    hits.truncate(k);
    hits.into_iter()
        .map(|(id, chunk, score)| {
            let row = &store.documents[&id].chunks[chunk as usize];
            (id, row.id.clone(), score.clamp(-1.0, 1.0))
        })
        .collect()
}

fn graph_snapshot_results(
    snapshot: &VectorSnapshot,
    store: &RwLock<DocumentStore>,
    k: usize,
) -> Vec<(String, String, f64)> {
    search_vector_snapshot(
        snapshot,
        store,
        &[1.0, 0.0],
        &[1.0, 0.0],
        k,
        &VectorQueryOptions {
            mode: VectorIndexMode::Graph,
            ef: 64,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap()
    .into_iter()
    .map(|hit| (hit.document_id, hit.id, hit.semantic_score.unwrap()))
    .collect()
}

#[test]
fn stale_graph_uses_tracked_changes_or_requests_exact_fallback() {
    for changed in [
        MAX_DELTA_DOCUMENTS - 1,
        MAX_DELTA_DOCUMENTS,
        MAX_DELTA_DOCUMENTS + 1,
    ] {
        let mut store = DocumentStore::default();
        for i in 0..8 {
            put_vector(
                &mut store,
                &format!("base-{i:02}"),
                "base",
                vec![vec![0.6, 0.8]],
            );
        }
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), 1).unwrap();
        for i in 0..changed {
            put_vector(
                &mut store,
                &format!("delta-{i:03}"),
                "delta",
                vec![vec![1.0, i as f64 / 1000.0]],
            );
        }
        assert_eq!(
            store
                .vector_changes
                .read()
                .since(snapshot.generation, store.generation)
                .is_some(),
            changed <= MAX_DELTA_DOCUMENTS
        );
        let expected = live_scan_results(&store, &[1.0, 0.0], 8, -1.0);
        let store = RwLock::new(store);
        let ranked = search_vector_snapshot(
            &snapshot,
            &store,
            &[1.0, 0.0],
            &[1.0, 0.0],
            8,
            &VectorQueryOptions {
                mode: VectorIndexMode::Graph,
                ef: 64,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ranked.is_some(), changed <= MAX_DELTA_DOCUMENTS);
        let results = ranked.unwrap_or_else(|| {
            super::super::search(
                &store.read(),
                "",
                &super::super::SearchOptions {
                    k: Some(8),
                    embedding: Some(vec![1.0, 0.0]),
                    lexical_weight: Some(0.0),
                    semantic_weight: Some(1.0),
                    min_semantic_score: Some(-1.0),
                    ..Default::default()
                },
                Some(2),
            )
            .unwrap()
        });
        let actual: Vec<_> = results
            .into_iter()
            .map(|hit| (hit.document_id, hit.id, hit.semantic_score.unwrap()))
            .collect();
        assert_eq!(actual, expected, "changed={changed}");
    }
}

#[test]
fn stale_graph_bounds_delta_chunks_without_truncating() {
    for chunks in [MAX_DELTA_CHUNKS - 1, MAX_DELTA_CHUNKS, MAX_DELTA_CHUNKS + 1] {
        let mut store = DocumentStore::default();
        put_vector(&mut store, "base", "base", vec![vec![0.0, 1.0]]);
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), 1).unwrap();
        put_vector(
            &mut store,
            "delta",
            &vec!["delta"; chunks].join(" "),
            vec![vec![1.0, 0.0]; chunks],
        );
        let expected = live_scan_results(&store, &[1.0, 0.0], 10, -1.0);
        let store = RwLock::new(store);
        let ranked = search_vector_snapshot(
            &snapshot,
            &store,
            &[1.0, 0.0],
            &[1.0, 0.0],
            10,
            &VectorQueryOptions {
                mode: VectorIndexMode::Graph,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ranked.is_some(), chunks <= MAX_DELTA_CHUNKS);
        let results = ranked.unwrap_or_else(|| {
            super::super::search(
                &store.read(),
                "",
                &super::super::SearchOptions {
                    k: Some(10),
                    embedding: Some(vec![1.0, 0.0]),
                    lexical_weight: Some(0.0),
                    semantic_weight: Some(1.0),
                    min_semantic_score: Some(-1.0),
                    ..Default::default()
                },
                Some(2),
            )
            .unwrap()
        });
        let actual: Vec<_> = results
            .into_iter()
            .map(|hit| (hit.document_id, hit.id, hit.semantic_score.unwrap()))
            .collect();
        assert_eq!(actual, expected, "chunks={chunks}");
    }
}

#[test]
fn stale_graph_tracks_replacements_deletions_and_reinsertions() {
    let mut store = DocumentStore::default();
    for i in 0..8 {
        put_vector(
            &mut store,
            &format!("base-{i:02}"),
            "base",
            vec![vec![0.6, 0.8]],
        );
    }
    let snapshot =
        build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), 1).unwrap();
    let store = RwLock::new(store);
    for stage in 0..5 {
        {
            let mut live = store.write();
            match stage {
                0 => {
                    let original = Arc::clone(live.get("base-00").unwrap());
                    live.insert("base-00".into(), original);
                }
                1 => put_vector(&mut live, "base-00", "replaced", vec![vec![1.0, 0.0]]),
                2 => delete_vector(&mut live, "base-01"),
                3 => put_vector(&mut live, "base-01", "reinserted", vec![vec![0.9, 0.1]]),
                _ => put_vector(&mut live, "new", "inserted", vec![vec![0.8, 0.2]]),
            }
        }
        assert_eq!(
            graph_snapshot_results(&snapshot, &store, 4),
            live_scan_results(&store.read(), &[1.0, 0.0], 4, -1.0),
            "stage={stage}"
        );
    }
}

#[test]
fn stale_graph_rejects_store_replacement_during_search() {
    let mut original = DocumentStore::default();
    put_vector(&mut original, "old", "old", vec![vec![1.0, 0.0]]);
    let snapshot =
        build_vector_snapshot(capture_vector_rows(&original, 2).unwrap().unwrap(), 1).unwrap();
    put_vector(&mut original, "changed", "changed", vec![vec![0.8, 0.6]]);
    let store = Arc::new(RwLock::new(original));
    let mut replacement = DocumentStore::default();
    put_vector(&mut replacement, "new", "new", vec![vec![1.0, 0.0]]);
    let changed = Arc::clone(&store);
    STALE_GRAPH_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || *changed.write() = replacement));
    });
    assert!(rank_vector_snapshot(
        &snapshot,
        &store,
        &[1.0, 0.0],
        &[1.0, 0.0],
        1,
        &VectorQueryOptions {
            mode: VectorIndexMode::Graph,
            ef: 64,
            ..Default::default()
        },
    )
    .unwrap()
    .is_none());
}

#[test]
fn stale_exact_snapshot_matches_complete_live_scan_after_mutations() {
    for obsolete_chunks in [9, 10, 11, 12, 13, 14] {
        let mut store = DocumentStore::default();
        put_vector(
            &mut store,
            "a",
            &vec!["same"; obsolete_chunks].join(" "),
            vec![vec![1.0, 0.0]; obsolete_chunks],
        );
        put_vector(&mut store, "b", "candidate", vec![vec![0.8, 0.6]]);
        put_vector(&mut store, "tie-a", "tie", vec![vec![0.0, 1.0]]);
        put_vector(&mut store, "tie-b", "tie", vec![vec![0.0, 1.0]]);
        for stable in 0..12 {
            put_vector(
                &mut store,
                &format!("stable-{stable:02}"),
                "stable",
                vec![vec![0.6, 0.8]],
            );
        }
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
                .unwrap();

        let store = RwLock::new(store);
        for stage in 0..4 {
            {
                let mut live = store.write();
                match stage {
                    0 => put_vector(&mut live, "a", "changed", vec![vec![0.0, 1.0]]),
                    1 => put_vector(&mut live, "fresh", "fresh", vec![vec![0.7, 0.7]]),
                    2 => delete_vector(&mut live, "tie-b"),
                    _ => put_vector(&mut live, "inserted", "inserted", vec![vec![0.75, 0.25]]),
                }
            }
            for k in [1, 10] {
                for min_score in [-1.0, 0.8] {
                    let expected = live_scan_results(&store.read(), &[1.0, 0.0], k, min_score);
                    let actual = snapshot_results(&snapshot, &store, &[1.0, 0.0], k, min_score);
                    assert_eq!(
                        actual, expected,
                        "obsolete chunks {obsolete_chunks}, stage {stage}, k {k}, min {min_score}"
                    );
                }
            }
        }
    }
}

#[test]
fn stale_exact_scoring_allows_writes_and_retains_a_coherent_ordered_capture() {
    let mut initial = DocumentStore::default();
    for row in 0..65 {
        put_vector(
            &mut initial,
            &format!("d{row:03}"),
            "base",
            vec![vec![1.0, row as f64 / 100.0]],
        );
    }
    let snapshot = build_vector_snapshot(
        capture_vector_rows(&initial, 2).unwrap().unwrap(),
        usize::MAX,
    )
    .unwrap();
    put_vector(&mut initial, "d000", "before", vec![vec![0.8, 0.6]]);
    delete_vector(&mut initial, "d064");
    put_vector(&mut initial, "inserted", "before", vec![vec![1.0, 0.0]]);
    let expected = live_scan_results(&initial, &[1.0, 0.0], 65, -1.0);
    let store = Arc::new(RwLock::new(initial));
    let (score_tx, score_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let writer_store = Arc::clone(&store);
    let writer = std::thread::spawn(move || {
        score_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut live = writer_store.write();
        put_vector(&mut live, "d000", "after", vec![vec![0.0, 1.0]]);
        delete_vector(&mut live, "d001");
        put_vector(&mut live, "inserted", "after", vec![vec![0.0, 1.0]]);
        put_vector(&mut live, "new-winner", "after", vec![vec![1.0, 0.0]]);
        drop(live);
        done_tx.send(()).unwrap();
    });
    let progressed = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&progressed);
    CANONICAL_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            score_tx.send(()).unwrap();
            observed.store(
                done_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
                AtomicOrdering::SeqCst,
            );
        }));
    });
    let actual = snapshot_results(&snapshot, &store, &[1.0, 0.0], 65, -1.0);
    writer.join().unwrap();
    assert!(
        progressed.load(AtomicOrdering::SeqCst),
        "writer blocked during scoring"
    );
    assert_eq!(actual, expected);
    assert_ne!(
        actual,
        live_scan_results(&store.read(), &[1.0, 0.0], 65, -1.0)
    );
}

#[test]
fn stale_graph_merges_changed_ids_without_returning_deleted_owners() {
    let mut store = DocumentStore::default();
    for row in 0..40 {
        put_vector(
            &mut store,
            &format!("d{row:03}"),
            "base",
            vec![vec![1.0, (row + 1) as f64 / 20.0]],
        );
    }
    let snapshot =
        build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), 1).unwrap();
    put_vector(&mut store, "d000", "replacement", vec![vec![0.0, 1.0]]);
    delete_vector(&mut store, "d001");
    put_vector(&mut store, "inserted", "new", vec![vec![1.0, 0.0]]);
    let expected = live_scan_results(&store, &[1.0, 0.0], 5, -1.0);
    let store = RwLock::new(store);
    let query = [1.0f32, 0.0];
    let actual = search_vector_snapshot(
        &snapshot,
        &store,
        &query,
        &[1.0, 0.0],
        5,
        &VectorQueryOptions {
            mode: VectorIndexMode::Graph,
            ef: 128,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    let ids: Vec<_> = actual.iter().map(|hit| hit.document_id.as_str()).collect();
    assert_eq!(
        ids,
        expected
            .iter()
            .map(|hit| hit.0.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(ids[0], "inserted");
    assert!(!ids.contains(&"d000") && !ids.contains(&"d001"));
}

#[test]
fn stale_exact_capture_preserves_live_rows_across_bitmap_boundaries() {
    for rows in [63, 64, 65, 127, 128, 129] {
        let mut store = DocumentStore::default();
        for row in 0..rows {
            put_vector(
                &mut store,
                &format!("d{row:03}"),
                "tie",
                vec![vec![1.0, 0.0]],
            );
        }
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
                .unwrap();
        put_vector(&mut store, "d000", "replacement", vec![vec![0.0, 1.0]]);
        delete_vector(&mut store, &format!("d{:03}", rows - 1));
        put_vector(&mut store, "new", "new", vec![vec![1.0, 0.0]]);
        let store = RwLock::new(store);
        for k in [1, rows] {
            for minimum in [-1.0, 0.5] {
                assert_eq!(
                    snapshot_results(&snapshot, &store, &[1.0, 0.0], k, minimum),
                    live_scan_results(&store.read(), &[1.0, 0.0], k, minimum),
                    "rows={rows} k={k} min={minimum}"
                );
            }
        }
    }
}

#[test]
fn stale_exact_capture_bounds_delta_documents_and_chunks_without_truncating() {
    for (documents, chunks, accepted) in [
        (MAX_DELTA_DOCUMENTS - 1, 1, true),
        (MAX_DELTA_DOCUMENTS, 1, true),
        (MAX_DELTA_DOCUMENTS + 1, 1, false),
        (1, MAX_DELTA_CHUNKS - 1, true),
        (1, MAX_DELTA_CHUNKS, true),
        (1, MAX_DELTA_CHUNKS + 1, false),
    ] {
        let mut store = DocumentStore::default();
        put_vector(&mut store, "base", "base", vec![vec![0.0, 1.0]]);
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
                .unwrap();
        for document in 0..documents {
            put_vector(
                &mut store,
                &format!("delta{document:03}"),
                &vec!["delta"; chunks].join(" "),
                vec![vec![1.0, 0.0]; chunks],
            );
        }
        let expected = live_scan_results(&store, &[1.0, 0.0], 10, -1.0);
        let store = RwLock::new(store);
        let capture = capture_stale_exact(&snapshot, &store.read()).unwrap();
        assert_eq!(
            capture.is_some(),
            accepted,
            "documents={documents} chunks={chunks}"
        );
        let ranked = search_vector_snapshot(
            &snapshot,
            &store,
            &[1.0, 0.0],
            &[1.0, 0.0],
            10,
            &VectorQueryOptions {
                mode: VectorIndexMode::Exact,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ranked.is_some(), accepted);
        let results = ranked.unwrap_or_else(|| {
            super::super::search(
                &store.read(),
                "",
                &super::super::SearchOptions {
                    k: Some(10),
                    embedding: Some(vec![1.0, 0.0]),
                    lexical_weight: Some(0.0),
                    semantic_weight: Some(1.0),
                    min_semantic_score: Some(-1.0),
                    ..Default::default()
                },
                Some(2),
            )
            .unwrap()
        });
        let actual: Vec<_> = results
            .into_iter()
            .map(|hit| (hit.document_id, hit.id, hit.semantic_score.unwrap()))
            .collect();
        assert_eq!(actual, expected, "documents={documents} chunks={chunks}");
    }
}

#[test]
fn tracked_vector_changes_deduplicate_updates_and_delete_reinsert() {
    let mut store = DocumentStore::default();
    put_vector(&mut store, "base", "base", vec![vec![1.0, 0.0]]);
    put_vector(&mut store, "stable", "stable", vec![vec![0.8, 0.6]]);
    let snapshot =
        build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
            .unwrap();
    let store = RwLock::new(store);
    for revision in 0..MAX_DELTA_DOCUMENTS + 1 {
        {
            let mut live = store.write();
            delete_vector(&mut live, "base");
            put_vector(
                &mut live,
                "base",
                "new",
                vec![vec![1.0, revision as f64 / 100.0]],
            );
        }
        assert_eq!(
            snapshot_results(&snapshot, &store, &[1.0, 0.0], 1, -1.0),
            live_scan_results(&store.read(), &[1.0, 0.0], 1, -1.0),
        );
    }
    let mut live = store.write();
    put_vector(&mut live, "temporary", "sensitive", vec![vec![1.0, 0.0]]);
    let removed = Arc::downgrade(&live.documents["temporary"]);
    delete_vector(&mut live, "temporary");
    assert!(
        removed.upgrade().is_none(),
        "ledger retained deleted payload"
    );
}

#[test]
fn tracked_vector_changes_bound_id_bytes_and_recover_after_a_new_capture() {
    use super::super::dense::MAX_DELTA_ID_BYTES;
    for bytes in [
        MAX_DELTA_ID_BYTES - 1,
        MAX_DELTA_ID_BYTES,
        MAX_DELTA_ID_BYTES + 1,
    ] {
        let mut store = DocumentStore::default();
        put_vector(&mut store, "base", "base", vec![vec![0.0, 1.0]]);
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
                .unwrap();
        let mut ids = Vec::new();
        for item in 0..17 {
            let size = match item {
                0..15 => 4096,
                15 => 2048,
                _ => bytes - 15 * 4096 - 2048,
            };
            let prefix = format!("delta{item:02}");
            let id = prefix.clone() + &"x".repeat(size - prefix.len());
            put_vector(&mut store, &id, "delta", vec![vec![1.0, 0.0]]);
            ids.push(id);
        }
        let store = RwLock::new(store);
        assert_eq!(
            capture_stale_exact(&snapshot, &store.read())
                .unwrap()
                .is_some(),
            bytes <= MAX_DELTA_ID_BYTES,
            "id bytes {bytes}"
        );
        if bytes <= MAX_DELTA_ID_BYTES {
            assert_eq!(
                snapshot_results(&snapshot, &store, &[1.0, 0.0], 1, -1.0),
                live_scan_results(&store.read(), &[1.0, 0.0], 1, -1.0),
            );
        }
        let rebuilt = build_vector_snapshot(
            capture_vector_rows(&store.read(), 2).unwrap().unwrap(),
            usize::MAX,
        )
        .unwrap();
        delete_vector(&mut store.write(), &ids[0]);
        assert_eq!(
            snapshot_results(&rebuilt, &store, &[1.0, 0.0], 1, -1.0),
            live_scan_results(&store.read(), &[1.0, 0.0], 1, -1.0),
        );
    }
}

#[test]
fn tracked_vector_changes_cover_independent_snapshots_and_dense_lifecycles() {
    use super::super::dense::{build_dense_index, capture_dense_index, publish_dense_index};
    let mut store = DocumentStore::default();
    for id in 0..=MAX_DELTA_DOCUMENTS {
        put_vector(
            &mut store,
            &format!("d{id:03}"),
            "base",
            vec![vec![1.0, 0.0]],
        );
    }
    let first = build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
        .unwrap();
    put_vector(&mut store, "d000", "changed", vec![vec![0.0, 1.0]]);
    let capture = capture_vector_rows(&store, 2).unwrap().unwrap();
    put_vector(&mut store, "d000", "changed again", vec![vec![0.8, 0.6]; 2]);
    let second = build_vector_snapshot(capture, usize::MAX).unwrap();
    let trained = build_dense_index(capture_dense_index(&store, 2).unwrap().unwrap())
        .unwrap()
        .unwrap();
    assert!(publish_dense_index(&mut store, &trained));
    put_vector(&mut store, "d001", "changed", vec![vec![0.0, 1.0]]);
    store.discard_stale_dense_index();
    assert!(store.dense_index.is_none());
    let store = RwLock::new(store);
    for snapshot in [&first, &second] {
        assert_eq!(
            snapshot_results(snapshot, &store, &[1.0, 0.0], 10, -1.0),
            live_scan_results(&store.read(), &[1.0, 0.0], 10, -1.0),
        );
    }
    {
        let mut live = store.write();
        for id in 2..MAX_DELTA_DOCUMENTS {
            put_vector(
                &mut live,
                &format!("d{id:03}"),
                "changed",
                vec![vec![0.0, 1.0]],
            );
        }
    }
    let newest = build_vector_snapshot(
        capture_vector_rows(&store.read(), 2).unwrap().unwrap(),
        usize::MAX,
    )
    .unwrap();
    put_vector(&mut store.write(), "d128", "latest", vec![vec![1.0, 0.0]]);
    assert!(capture_stale_exact(&first, &store.read())
        .unwrap()
        .is_none());
    assert!(capture_stale_exact(&second, &store.read())
        .unwrap()
        .is_none());
    assert_eq!(
        snapshot_results(&newest, &store, &[1.0, 0.0], 10, -1.0),
        live_scan_results(&store.read(), &[1.0, 0.0], 10, -1.0),
    );
}

#[test]
fn tracked_vector_changes_reject_identity_aliases_and_generation_gaps() {
    let mut original = DocumentStore::default();
    put_vector(&mut original, "old", "old", vec![vec![1.0, 0.0]]);
    let snapshot = build_vector_snapshot(
        capture_vector_rows(&original, 2).unwrap().unwrap(),
        usize::MAX,
    )
    .unwrap();
    let mut other = DocumentStore::default();
    put_vector(&mut other, "new", "new", vec![vec![0.0, 1.0]]);
    assert_eq!(original.generation, other.generation);
    let status = vector_index_status(Some(&snapshot), &other);
    assert!(!status.current && status.rebuild_recommended);
    assert!(rank_vector_snapshot(
        &snapshot,
        &RwLock::new(other),
        &[1.0, 0.0],
        &[1.0, 0.0],
        1,
        &VectorQueryOptions::default(),
    )
    .unwrap()
    .is_none());
    original.generation += 1;
    assert!(capture_stale_exact(&snapshot, &original).unwrap().is_none());
    put_vector(&mut original, "old", "after gap", vec![vec![0.0, 1.0]; 2]);
    assert!(capture_stale_exact(&snapshot, &original).unwrap().is_none());
    let rebuilt = build_vector_snapshot(
        capture_vector_rows(&original, 2).unwrap().unwrap(),
        usize::MAX,
    )
    .unwrap();
    put_vector(&mut original, "new", "new", vec![vec![1.0, 0.0]]);
    let store = RwLock::new(original);
    assert_eq!(
        snapshot_results(&rebuilt, &store, &[1.0, 0.0], 1, -1.0),
        live_scan_results(&store.read(), &[1.0, 0.0], 1, -1.0),
    );
}

#[test]
fn tracked_vector_changes_cover_batched_publication_and_replacements() {
    for count in [
        MAX_DELTA_DOCUMENTS - 1,
        MAX_DELTA_DOCUMENTS,
        MAX_DELTA_DOCUMENTS + 1,
    ] {
        let mut store = DocumentStore::default();
        put_vector(&mut store, "base", "base", vec![vec![0.0, 1.0]]);
        let snapshot =
            build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
                .unwrap();
        let batch = (0..count)
            .map(|id| {
                Arc::new(
                    prepare(
                        vector_input(&format!("d{id:03}"), "batch", vec![vec![1.0, 0.0]]),
                        1024,
                        Some(2),
                        TextEncoder::default(),
                    )
                    .unwrap(),
                )
            })
            .collect();
        store.insert_batch(batch).unwrap();
        let store = RwLock::new(store);
        assert_eq!(
            capture_stale_exact(&snapshot, &store.read())
                .unwrap()
                .is_some(),
            count <= MAX_DELTA_DOCUMENTS,
        );
        if count <= MAX_DELTA_DOCUMENTS {
            let batch = [
                vector_input("d000", "changed length", vec![vec![0.0, 1.0]; 2]),
                vector_input("d001", "changed", vec![vec![0.0, 1.0]]),
                vector_input("d000", "last", vec![vec![0.8, 0.6]]),
            ]
            .into_iter()
            .map(|input| Arc::new(prepare(input, 1024, Some(2), TextEncoder::default()).unwrap()))
            .collect();
            store.write().insert_batch(batch).unwrap();
            assert_eq!(
                snapshot_results(&snapshot, &store, &[1.0, 0.0], 10, -1.0),
                live_scan_results(&store.read(), &[1.0, 0.0], 10, -1.0),
            );
        }
    }
}

#[test]
fn tracked_vector_changes_fall_back_after_slot_rebuilding() {
    let mut store = DocumentStore::default();
    for id in 0..1026 {
        put_vector(
            &mut store,
            &format!("d{id:04}"),
            "base",
            vec![vec![1.0, 0.0]],
        );
    }
    let snapshot =
        build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
            .unwrap();
    for id in 0..514 {
        delete_vector(&mut store, &format!("d{id:04}"));
    }
    assert_ne!(snapshot.identity, store.identity, "slots were not rebuilt");
    assert!(capture_stale_exact(&snapshot, &store).unwrap().is_none());
    let rebuilt =
        build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
            .unwrap();
    put_vector(&mut store, "d0514", "changed", vec![vec![0.0, 1.0]]);
    let store = RwLock::new(store);
    assert_eq!(
        snapshot_results(&rebuilt, &store, &[1.0, 0.0], 10, -1.0),
        live_scan_results(&store.read(), &[1.0, 0.0], 10, -1.0),
    );
}

#[test]
fn tracked_vector_changes_clear_complete_document_row_ranges() {
    for prefix in [0, 1, 63, 64, 65] {
        for replaced in [1, 2, 63, 64, 65, 127, 128, 129] {
            let mut store = DocumentStore::default();
            if prefix != 0 {
                put_vector(
                    &mut store,
                    "a-prefix",
                    &vec!["prefix"; prefix].join(" "),
                    vec![vec![0.0, 1.0]; prefix],
                );
            }
            put_vector(
                &mut store,
                "b-replaced",
                &vec!["obsolete"; replaced].join(" "),
                vec![vec![1.0, 0.0]; replaced],
            );
            put_vector(&mut store, "c-stable", "stable", vec![vec![0.8, 0.6]]);
            let snapshot =
                build_vector_snapshot(capture_vector_rows(&store, 2).unwrap().unwrap(), usize::MAX)
                    .unwrap();
            put_vector(&mut store, "b-replaced", "new", vec![vec![0.0, 1.0]]);
            let store = RwLock::new(store);
            assert_eq!(
                snapshot_results(&snapshot, &store, &[1.0, 0.0], 1, -1.0),
                live_scan_results(&store.read(), &[1.0, 0.0], 1, -1.0),
                "prefix={prefix}, replaced={replaced}"
            );
        }
    }
}

#[test]
fn dot_matches_the_scalar_sum_with_a_remainder() {
    let a: Vec<f32> = (0..19).map(|i| i as f32 * 0.25).collect();
    let b: Vec<f32> = (0..19).map(|i| 1.0 - i as f32 * 0.1).collect();
    let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    assert!((dot(&a, &b) - expected).abs() < 1e-4);
}

/// The snapshot holds the store's embedding allocations, not copies, and the graph-free
/// and graph snapshots both answer from them.
#[test]
fn snapshot_rows_share_the_store_embeddings() {
    const DIM: usize = 16;
    let axis = |lead: usize, second: usize, value: f64| -> Vec<f64> {
        let mut values = vec![0.0; DIM];
        values[lead] = 1.0;
        values[second] = value;
        values
    };
    let mut store = DocumentStore::default();
    for i in 0..40 {
        let input: DocumentInput = serde_json::from_value(json!({
            "id": format!("d{i:03}"),
            "text": format!("document {i} with a few words of text for two chunks here"),
            "chunkWords": 6, "overlapWords": 0,
            "embeddings": [axis(0, 1, i as f64 / 50.0), axis(2, 3, i as f64 / 40.0)]
        }))
        .unwrap();
        let document = prepare(input, 1024, Some(DIM), TextEncoder::default()).unwrap();
        store.insert(document.id.clone(), document);
    }
    let capture = capture_vector_rows(&store, DIM).unwrap().unwrap();
    assert_eq!(capture.rows.len(), 80);
    for (row, embedding) in capture.rows.iter().zip(&capture.embeddings) {
        let stored = store.documents[&row.document.id].chunks[row.chunk as usize]
            .embedding
            .as_ref()
            .unwrap();
        assert!(embedding.shares_allocation(stored));
    }
    for graph_min_rows in [usize::MAX, 1] {
        let capture = capture_vector_rows(&store, DIM).unwrap().unwrap();
        let snapshot = build_vector_snapshot(capture, graph_min_rows).unwrap();
        assert_eq!(snapshot.has_graph(), graph_min_rows == 1);
        for (row, embedding) in snapshot.rows.iter().zip(snapshot.exact.rows.iter()) {
            let stored = store.documents[&row.document.id].chunks[row.chunk as usize]
                .embedding
                .as_ref()
                .unwrap();
            assert!(embedding.shares_allocation(stored));
        }
        let query = Embedding::from_f64(
            &super::super::normalize_embedding(&axis(0, 1, 39.0 / 50.0), DIM).unwrap(),
        );
        let hits = snapshot
            .index(VectorIndexMode::Auto)
            .unwrap()
            .search(&query, 1, DEFAULT_EF)
            .unwrap();
        let row = &snapshot.rows[hits[0].0 as usize];
        assert_eq!((row.document.id.as_str(), row.chunk), ("d039", 0));
        assert!((hits[0].1 - 1.0).abs() < 1e-6);
    }
}

/// Screening must never change the canonical result: the best `k` rows, their order (ties by row) and their scores
/// are those of scoring every included row exactly, at the sizes either side of the screening threshold, with
/// duplicated rows (exact ties) and an eligibility filter.
#[test]
fn screened_canonical_search_equals_the_unscreened_ranking() {
    let mut state = 0x9e37_79b9_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0 - 0.5
    };
    for rows in [
        SCREEN_MIN_ROWS - 1,
        SCREEN_MIN_ROWS,
        SCREEN_MIN_ROWS + 1,
        1500,
    ] {
        let mut data: Vec<Vec<f32>> = (0..rows)
            .map(|_| {
                let row: Vec<f32> = (0..24).map(|_| next()).collect();
                let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
                row.iter().map(|v| v / norm).collect()
            })
            .collect();
        for row in (0..rows).step_by(7).skip(1) {
            data[row] = data[row - 1].clone();
        }
        let scan = ExactScan {
            dim: 24,
            rows: data.iter().cloned().map(Embedding::from_f32).collect(),
        };
        let query: Vec<f64> = {
            let raw: Vec<f64> = (0..24).map(|_| f64::from(next())).collect();
            let norm = raw.iter().map(|v| v * v).sum::<f64>().sqrt();
            raw.iter().map(|v| v / norm).collect()
        };
        for k in [1, 10, 100.min(rows)] {
            for include in [(|_: usize| true) as fn(usize) -> bool, |row| row % 3 != 0] {
                let mut expected: Vec<(u32, f64)> = (0..rows)
                    .filter(|&row| include(row))
                    .map(|row| (row as u32, scan.rows[row].cosine(&query)))
                    .collect();
                expected.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                expected.truncate(k);
                assert_eq!(
                    scan.search_canonical(&query, k, include),
                    expected,
                    "rows {rows} k {k}"
                );
            }
        }
    }
}
