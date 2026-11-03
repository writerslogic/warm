fn bounded_document(
    id: usize,
    vector: Vec<f64>,
    dimensions: usize,
    analytic: bool,
) -> StoredDocument {
    let mut metadata = json!({"ordinal": id, "eligible10": id % 100 < 10,
        "eligible1": id % 100 == 0, "bucket100": id % 100});
    if analytic {
        metadata["even"] = json!(id % 2 == 0);
    }
    let input = serde_json::from_value(json!({"id": format!("d{id:06}"), "text": "x".repeat(1024),
        "metadata": metadata, "embeddings": [vector]}))
    .unwrap();
    super::super::prepare(input, 1024, Some(dimensions), TextEncoder::default()).unwrap()
}

fn bounded_train(store: &mut DocumentStore, dimensions: usize) {
    let trained = super::super::build_dense_index(
        super::super::capture_dense_index(store, dimensions)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(store, &trained));
}

fn bounded_measure(
    stream: &mut std::fs::File,
    name: &str,
    state: &str,
    filter_name: &str,
    filter: Option<Value>,
    store: &DocumentStore,
    queries: &[Vec<f64>],
) {
    use std::io::Write;
    let dimensions = queries[0].len();
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
            let mut values = [Value::Null, Value::Null, Value::Null];
            let mut seconds = [0.0; 3];
            for offset in 0..3 {
                let route = (ordinal + repetition + offset) % 3;
                let start = std::time::Instant::now();
                let results = {
                    let prepared = prepare_search("", &options, Some(dimensions)).unwrap();
                    let captured = match route {
                        0 => CapturedSearch {
                            queries: vec![capture_original_query(
                                store,
                                prepared,
                                store.dense_index.clone(),
                            )],
                        },
                        1 => capture_search(store, vec![prepared]),
                        2 => CapturedSearch {
                            queries: vec![capture_postings(store, prepared)],
                        },
                        _ => unreachable!(),
                    };
                    execute_search(&captured, 0).unwrap()
                };
                seconds[route] = start.elapsed().as_secs_f64();
                values[route] = serde_json::to_value(results).unwrap();
            }
            assert_eq!(
                values[0], values[2],
                "legacy/scan {name} {state} {filter_name} {ordinal}"
            );
            assert_eq!(
                values[1], values[2],
                "bounded/scan {name} {state} {filter_name} {ordinal}"
            );
            let captured = capture_search(
                store,
                vec![prepare_search("", &options, Some(dimensions)).unwrap()],
            );
            let query = &captured.queries[0];
            let (route, dots, budget) = match (&query.dense, &query.scan) {
                (Some(index), Some(_)) => {
                    let budget = query.count * (dimensions + 1) / 16;
                    match index.search_where_limited(
                        query.prepared.embedding.as_ref().unwrap(),
                        query.prepared.limit,
                        query.prepared.minimum,
                        |document| query.prepared.matches(document),
                        budget,
                    ) {
                        Some((_, dots)) => ("tree", Some(dots), Some(budget)),
                        None => ("exhausted-then-scan", None, Some(budget)),
                    }
                }
                (None, _) => ("scan", None, None),
                (Some(_), None) => ("unlimited-tree", None, None),
            };
            writeln!(
                stream,
                "{}",
                json!({"dataset": name, "state": state, "filter": filter_name,
                "repetition": repetition, "queryOrdinal": ordinal, "originalSeconds": seconds[0],
                "boundedSeconds": seconds[1], "scanSeconds": seconds[2], "boundedRoute": route,
                "completedTreeDots": dots, "eligibleChunks": query.count, "workBudget": budget,
                "results": values[1]})
            )
            .unwrap();
        }
    }
    stream.flush().unwrap();
}

#[test]
#[ignore = "frozen exposed-data timing probe; explicit input/output paths required"]
fn frozen_bounded_dense_comparison() {
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
                .collect::<Vec<_>>()
        };
        let mut query_file = std::fs::File::open(input.join(name).join("calibration.f32")).unwrap();
        assert_eq!(
            query_file.metadata().unwrap().len(),
            (1000 * dimensions * 4) as u64
        );
        let mut query_bytes = vec![0; 32 * dimensions * 4];
        query_file.read_exact(&mut query_bytes).unwrap();
        let queries: Vec<Vec<f64>> = query_bytes
            .chunks_exact(dimensions * 4)
            .map(|row| {
                row.chunks_exact(4)
                    .map(|bytes| f64::from(f32::from_le_bytes(bytes.try_into().unwrap())))
                    .collect()
            })
            .collect();
        let mut store = DocumentStore::default();
        for id in 0..100000 {
            let document = bounded_document(id, vector(id), dimensions, false);
            store.insert(document.id.clone(), document);
        }
        for phase in ["baseline", "churn-maintained"] {
            if phase != "baseline" {
                for id in 0..100 {
                    let document = bounded_document(id, vector(id + 1), dimensions, false);
                    store.insert(document.id.clone(), document);
                }
                for id in 100..200 {
                    store.remove(&format!("d{id:06}")).unwrap();
                }
                assert!(store.dense_index.is_none());
            }
            bounded_train(&mut store, dimensions);
            for (filter_name, filter) in [
                ("all", None),
                ("10pct", Some(json!({"eligible10": true}))),
                ("1pct", Some(json!({"eligible1": true}))),
                ("empty", Some(json!({"bucket100": -1}))),
            ] {
                bounded_measure(
                    &mut stream,
                    name,
                    phase,
                    filter_name,
                    filter,
                    &store,
                    &queries,
                );
            }
        }
    }
    for count in [8192, 32768] {
        for dimensions in [2, 4] {
            let mut store = DocumentStore::default();
            for id in 0..count {
                let mut vector = vec![0.0; dimensions];
                vector[0] = if id < 256 { 1.0 } else { -1.0 };
                if id >= 256 {
                    vector[1] = (id % 5) as f64 / 100.0;
                }
                let document = bounded_document(id, vector, dimensions, true);
                store.insert(document.id.clone(), document);
            }
            bounded_train(&mut store, dimensions);
            let queries: Vec<_> = (0..32)
                .map(|ordinal| {
                    let mut query = vec![0.0; dimensions];
                    query[0] = 1.0;
                    query[1] = (ordinal as f64 - 16.0) / 1000.0;
                    query
                })
                .collect();
            bounded_measure(
                &mut stream,
                &format!("separated-{count}-{dimensions}"),
                "baseline",
                "50pct",
                Some(json!({"even": true})),
                &store,
                &queries,
            );
        }
    }
}
