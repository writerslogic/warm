#[test]
fn semantic_scan_preserves_boundaries_and_captured_owners() {
    for count in [127, 128, 129] {
        let mut store = DocumentStore::default();
        for id in 0..count - 1 + 128 {
            let text = if id == 0 {
                "first chunk second chunk"
            } else {
                "first chunk"
            };
            let vectors = if id == 0 {
                json!([[1.0, 0.0], [-1.0, 0.0]])
            } else {
                json!([[1.0, 0.0]])
            };
            let input = serde_json::from_value(json!({
                "id": format!("d{id:04}"), "text": text,
                "chunkWords": 2, "overlapWords": 0, "metadata": {"group": if id < count - 1 { 0 } else { 9 }},
                "embeddings": vectors
            })).unwrap();
            let mut document =
                super::super::prepare(input, 1024, Some(2), TextEncoder::default()).unwrap();
            if id % 7 == 0 {
                document.chunks[0].embedding = None;
            }
            store.insert(document.id.clone(), document);
        }
        for trained in [false, true] {
            if trained {
                bounded_train(&mut store, 2);
            }
            for limit in [99, 100, 101] {
                for minimum in [-1.0, 0.0, 1.0_f64.next_down(), 1.0] {
                    for fusion in ["blend", "rrf"] {
                        let options = SearchOptions {
                            embedding: Some(vec![1.0, 0.0]),
                            filter: Some(json!({"group": 0})),
                            lexical_weight: Some(0.0),
                            semantic_weight: Some(1.0),
                            min_semantic_score: Some(minimum),
                            candidate_limit: Some(limit),
                            k: Some(10),
                            fusion: Some(fusion.into()),
                            ..Default::default()
                        };
                        let captured = capture_search(
                            &store,
                            vec![prepare_search("", &options, Some(2)).unwrap()],
                        );
                        let expected =
                            super::super::search_impl(&store, "", &options, Some(2), false)
                                .unwrap();
                        assert_eq!(
                            serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
                            serde_json::to_value(&expected).unwrap()
                        );
                        let original = store.remove("d0001").unwrap();
                        let mut replacement = original.as_ref().clone();
                        replacement.metadata = json!({"group": 9});
                        replacement.chunks[0].embedding = None;
                        store.insert(replacement.id.clone(), replacement);
                        assert_eq!(
                            serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
                            serde_json::to_value(expected).unwrap()
                        );
                        let fresh = capture_search(
                            &store,
                            vec![prepare_search("", &options, Some(2)).unwrap()],
                        );
                        assert_eq!(
                            serde_json::to_value(execute_search(&fresh, 0).unwrap()).unwrap(),
                            serde_json::to_value(
                                super::super::search_impl(&store, "", &options, Some(2), false)
                                    .unwrap()
                            )
                            .unwrap()
                        );
                        store.insert(original.id.clone(), original);
                    }
                }
            }
        }
    }
}

fn semantic_scan_measure(
    stream: &mut std::fs::File,
    name: &str,
    state: &str,
    filter_name: &str,
    filter: Option<Value>,
    store: &DocumentStore,
    queries: &[Vec<f64>],
) {
    use std::{io::Write, time::Instant};
    for repetition in 0..3 {
        for (ordinal, query) in queries.iter().enumerate() {
            let options = SearchOptions {
                k: Some(10),
                candidate_limit: Some(100),
                embedding: Some(query.clone()),
                filter: filter.clone(),
                lexical_weight: Some(0.0),
                semantic_weight: Some(1.0),
                min_semantic_score: Some(-1.0),
                ..Default::default()
            };
            let mut results = [Value::Null, Value::Null];
            let mut elapsed = [0.0; 2];
            for offset in 0..2 {
                let method = (ordinal + repetition + offset) % 2;
                let start = Instant::now();
                let prepared = prepare_search("", &options, Some(query.len())).unwrap();
                let captured = if method == 0 {
                    baseline_capture_query(store, prepared)
                } else {
                    capture_query(store, prepared)
                };
                let ranked = if method == 0 {
                    baseline_execute_query(&captured)
                } else {
                    execute_query(&captured)
                };
                let rows: Vec<_> = ranked
                    .into_iter()
                    .map(|hit| hit.read(captured.text.as_deref()).unwrap())
                    .collect();
                drop(captured);
                elapsed[method] = start.elapsed().as_secs_f64();
                results[method] = serde_json::to_value(rows).unwrap();
            }
            assert_eq!(
                results[0], results[1],
                "{name} {state} {filter_name} {ordinal}"
            );
            writeln!(
                stream,
                "{}",
                json!({"dataset": name, "state": state, "filter": filter_name,
                "repetition": repetition, "queryOrdinal": ordinal, "baselineSeconds": elapsed[0],
                "candidateSeconds": elapsed[1], "results": results[1]})
            )
            .unwrap();
        }
    }
    stream.flush().unwrap();
}

#[test]
#[ignore = "frozen exposed-data semantic scan screen; explicit paths required"]
fn frozen_semantic_scan_comparison() {
    use std::io::Read;
    let input = std::path::PathBuf::from(std::env::var("WARM_DENSE_DIAGNOSTIC_INPUT").unwrap());
    let output = std::path::PathBuf::from(std::env::var("WARM_DENSE_DIAGNOSTIC_OUTPUT").unwrap());
    std::fs::create_dir(&output).unwrap();
    let mut stream = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("queries.ndjson"))
        .unwrap();
    for (name, dimensions) in [("coco-i2i-512-angular", 512), ("deep-image-96-angular", 96)] {
        let raw = std::fs::read(input.join(name).join("train.f32")).unwrap();
        assert_eq!(raw.len(), 100000 * dimensions * 4);
        let vector = |id: usize| {
            raw[id * dimensions * 4..(id + 1) * dimensions * 4]
                .chunks_exact(4)
                .map(|bytes| f64::from(f32::from_le_bytes(bytes.try_into().unwrap())))
                .collect()
        };
        let mut file = std::fs::File::open(input.join(name).join("calibration.f32")).unwrap();
        assert_eq!(
            file.metadata().unwrap().len(),
            (1000 * dimensions * 4) as u64
        );
        let mut bytes = vec![0; 32 * dimensions * 4];
        file.read_exact(&mut bytes).unwrap();
        let queries: Vec<Vec<f64>> = bytes
            .chunks_exact(dimensions * 4)
            .map(|row| {
                row.chunks_exact(4)
                    .map(|word| f64::from(f32::from_le_bytes(word.try_into().unwrap())))
                    .collect()
            })
            .collect();
        let mut store = DocumentStore::default();
        for id in 0..100000 {
            let document = bounded_document(id, vector(id), dimensions, false);
            store.insert(document.id.clone(), document);
        }
        for churn in [false, true] {
            if churn {
                for id in 0..100 {
                    let document = bounded_document(id, vector(id + 1), dimensions, false);
                    store.insert(document.id.clone(), document);
                }
                for id in 100..200 {
                    store.remove(&format!("d{id:06}")).unwrap();
                }
            }
            store.dense_index = None;
            store.dense_changes.clear();
            for trained in [false, true] {
                if trained {
                    bounded_train(&mut store, dimensions);
                }
                let state = match (churn, trained) {
                    (false, false) => "baseline-untrained",
                    (false, true) => "baseline-trained",
                    (true, false) => "churn-untrained",
                    (true, true) => "churn-trained",
                };
                for (label, filter) in [
                    ("all", None),
                    ("10pct", Some(json!({"eligible10": true}))),
                    ("1pct", Some(json!({"eligible1": true}))),
                    ("empty", Some(json!({"bucket100": -1}))),
                ] {
                    semantic_scan_measure(
                        &mut stream,
                        name,
                        state,
                        label,
                        filter,
                        &store,
                        &queries,
                    );
                }
            }
        }
    }
}
