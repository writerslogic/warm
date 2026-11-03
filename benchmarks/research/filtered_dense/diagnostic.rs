fn diagnostic_document(id: usize, vector: &[u8], dimensions: usize) -> StoredDocument {
    assert_eq!(vector.len(), dimensions * 4);
    let vector: Vec<f64> = vector
        .chunks_exact(4)
        .map(|value| f64::from(f32::from_le_bytes(value.try_into().unwrap())))
        .collect();
    assert!(vector.iter().all(|value| value.is_finite()));
    let input = serde_json::from_value(json!({
        "id": format!("d{id:06}"), "text": "x".repeat(1024),
        "metadata": {"ordinal": id, "eligible10": id % 100 < 10, "eligible1": id % 100 == 0, "bucket100": id % 100},
        "embeddings": [vector],
    })).unwrap();
    documents::prepare(input, 1024, Some(dimensions), TextEncoder::default()).unwrap()
}

fn diagnostic_bytes(path: &std::path::Path, expected: usize) -> Vec<u8> {
    assert_eq!(std::fs::metadata(path).unwrap().len(), expected as u64);
    std::fs::read(path).unwrap()
}

#[test]
#[ignore = "isolated exposed-data diagnostic; requires explicit pinned input/output paths"]
fn frozen_filtered_dense_counterfactual() {
    use std::io::{Read, Write};
    let input = std::path::PathBuf::from(std::env::var("WARM_DENSE_DIAGNOSTIC_INPUT").unwrap());
    let output = std::path::PathBuf::from(std::env::var("WARM_DENSE_DIAGNOSTIC_OUTPUT").unwrap());
    std::fs::create_dir(&output).unwrap();
    for (name, dimensions) in [("coco-i2i-512-angular", 512), ("deep-image-96-angular", 96)] {
        let folder = input.join(name);
        let raw = diagnostic_bytes(&folder.join("train.f32"), 100000 * dimensions * 4);
        let mut query_file = std::fs::File::open(folder.join("calibration.f32")).unwrap();
        assert_eq!(
            query_file.metadata().unwrap().len(),
            (1000 * dimensions * 4) as u64
        );
        let mut queries = vec![0; 32 * dimensions * 4];
        query_file.read_exact(&mut queries).unwrap();
        let queries: Vec<Vec<f64>> = queries
            .chunks_exact(dimensions * 4)
            .map(|row| {
                row.chunks_exact(4)
                    .map(|value| f64::from(f32::from_le_bytes(value.try_into().unwrap())))
                    .collect()
            })
            .collect();
        let mut store = DocumentStore::default();
        for (id, row) in raw.chunks_exact(dimensions * 4).enumerate() {
            let document = diagnostic_document(id, row, dimensions);
            store.insert(document.id.clone(), document);
        }
        let mut stream = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(format!("{name}.ndjson")))
            .unwrap();
        for phase in ["baseline", "churn-maintained"] {
            if phase != "baseline" {
                for id in 0..100 {
                    let start = (id + 1) * dimensions * 4;
                    let document =
                        diagnostic_document(id, &raw[start..start + dimensions * 4], dimensions);
                    store.insert(document.id.clone(), document);
                }
                for id in 100..200 {
                    assert!(store.remove(&format!("d{id:06}")).is_some());
                }
                assert!(store.dense_index.is_none());
            }
            let trained = train(&store, dimensions);
            assert!(publish_dense_index(&mut store, &trained));
            let filters = [
                ("all", None),
                ("10pct", Some(json!({"eligible10": true}))),
                ("1pct", Some(json!({"eligible1": true}))),
                ("empty", Some(json!({"bucket100": -1}))),
            ];
            for (filter_name, filter) in filters {
                for repetition in 0..3 {
                    for (ordinal, query) in queries.iter().enumerate() {
                        let options = documents::SearchOptions {
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
                        let mut seconds = [0.0; 2];
                        for route in if (ordinal + repetition) % 2 == 0 {
                            [0, 1]
                        } else {
                            [1, 0]
                        } {
                            store.dense_index = (route == 0).then(|| Arc::clone(&trained.index));
                            let start = std::time::Instant::now();
                            let found =
                                documents::search(&store, "", &options, Some(dimensions)).unwrap();
                            seconds[route] = start.elapsed().as_secs_f64();
                            results[route] = serde_json::to_value(found).unwrap();
                        }
                        assert_eq!(
                            results[0], results[1],
                            "{name} {phase} {filter_name} {ordinal}"
                        );
                        let predicate_visits = std::cell::Cell::new(0);
                        let normalized = documents::normalize_embedding(query, dimensions).unwrap();
                        let (_, dots) =
                            trained
                                .index
                                .search_where(&normalized, 100, -1.0, |document| {
                                    predicate_visits.set(predicate_visits.get() + 1);
                                    documents::matches_document_metadata(document, &options)
                                });
                        writeln!(stream, "{}", json!({"state": phase, "filter": filter_name,
                            "repetition": repetition, "queryOrdinal": ordinal, "treeSeconds": seconds[0],
                            "scanSeconds": seconds[1], "metadataPredicateVisits": predicate_visits.get(),
                            "vectorDots": dots, "indexRows": trained.index.rows.len(), "indexNodes": trained.index.nodes.len(),
                            "results": results[0]})).unwrap();
                    }
                }
            }
            store.dense_index = Some(Arc::clone(&trained.index));
            stream.flush().unwrap();
        }
    }
}
