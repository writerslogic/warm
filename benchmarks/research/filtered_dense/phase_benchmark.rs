fn interleaved_cosines(rows: &[&super::super::Embedding], query: &[f64]) -> Vec<f64> {
    assert!(rows.iter().all(|row| row.len() == query.len()));
    let mut result = Vec::with_capacity(rows.len());
    let mut blocks = rows.chunks_exact(8);
    for block in &mut blocks {
        let mut sums = [std::iter::empty::<f64>().sum::<f64>(); 8];
        for (column, &value) in query.iter().enumerate() {
            for lane in 0..8 {
                sums[lane] += value * f64::from(block[lane][column]);
            }
        }
        result.extend(sums.map(|value| value.clamp(-1.0, 1.0)));
    }
    result.extend(blocks.remainder().iter().map(|row| row.cosine(query)));
    result
}

#[test]
fn interleaved_score_bits_and_tail_boundaries() {
    for count in [7, 8, 9] {
        for dimensions in [1, 2, 3, 96, 512] {
            for zeros in [false, true] {
                let rows: Vec<_> = (0..count)
                    .map(|row| {
                        super::super::Embedding::from_f32(
                            (0..dimensions)
                                .map(|column| {
                                    if zeros {
                                        -0.0
                                    } else {
                                        ((row * 31 + column * 17) % 73) as f32 / 73.0 - 0.5
                                    }
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let query: Vec<_> = (0..dimensions).map(|i| 1.0 / (i + 1) as f64).collect();
                let references: Vec<_> = rows.iter().collect();
                let expected: Vec<_> = rows
                    .iter()
                    .map(|row| row.cosine(&query).to_bits())
                    .collect();
                let actual: Vec<_> = interleaved_cosines(&references, &query)
                    .into_iter()
                    .map(f64::to_bits)
                    .collect();
                assert_eq!(actual, expected, "{count} {dimensions} {zeros}");
            }
        }
    }
}

fn phase_measure(
    stream: &mut std::fs::File,
    name: &str,
    state: &str,
    filter_name: &str,
    filter: Value,
    store: &DocumentStore,
    queries: &[Vec<f64>],
) {
    use sha2::{Digest, Sha256};
    use std::{hint::black_box, io::Write, time::Instant};
    let dimensions = queries[0].len();
    for repetition in 0..3 {
        for (ordinal, vector) in queries.iter().enumerate() {
            let options = SearchOptions {
                k: Some(10),
                candidate_limit: Some(100),
                embedding: Some(vector.clone()),
                filter: Some(filter.clone()),
                lexical_weight: Some(0.0),
                semantic_weight: Some(1.0),
                min_semantic_score: Some(-1.0),
                ..Default::default()
            };
            let t0 = Instant::now();
            let prepared = prepare_search("", &options, Some(dimensions)).unwrap();
            let t1 = Instant::now();
            let captured = capture_search(store, vec![prepared]);
            let t2 = Instant::now();
            let ranked = rank_search(&captured, 0);
            let t3 = Instant::now();
            let results: Vec<_> = ranked
                .into_iter()
                .map(|hit| hit.read(captured.queries[0].text.as_deref()).unwrap())
                .collect();
            let t4 = Instant::now();
            drop(captured);
            let t5 = Instant::now();
            let scan = capture_postings(
                store,
                prepare_search("", &options, Some(dimensions)).unwrap(),
            );
            let query = scan.prepared.embedding.as_ref().unwrap();
            let embeddings: Vec<_> = scan
                .chunks
                .iter()
                .filter_map(|chunk| chunk.chunk().embedding.as_ref())
                .collect();
            let mut seconds = [0.0; 2];
            let mut scores = [Vec::new(), Vec::new()];
            for offset in 0..2 {
                let method = (ordinal + repetition + offset) % 2;
                let start = Instant::now();
                scores[method] = black_box(if method == 0 {
                    embeddings.iter().map(|row| row.cosine(query)).collect()
                } else {
                    interleaved_cosines(&embeddings, query)
                });
                seconds[method] = start.elapsed().as_secs_f64();
            }
            let mut hash = Sha256::new();
            for (&left, &right) in scores[0].iter().zip(&scores[1]) {
                assert_eq!(left.to_bits(), right.to_bits());
                hash.update(left.to_bits().to_le_bytes());
            }
            assert_eq!(scores[0].len(), scores[1].len());
            let reference = CapturedSearch {
                queries: vec![scan],
            };
            let actual = serde_json::to_value(results).unwrap();
            assert_eq!(
                actual,
                serde_json::to_value(execute_search(&reference, 0).unwrap()).unwrap()
            );
            writeln!(stream, "{}", json!({"dataset": name, "state": state, "filter": filter_name,
                "repetition": repetition, "queryOrdinal": ordinal,
                "prepareSeconds": (t1-t0).as_secs_f64(), "captureSeconds": (t2-t1).as_secs_f64(),
                "rankSeconds": (t3-t2).as_secs_f64(), "readSeconds": (t4-t3).as_secs_f64(),
                "releaseSeconds": (t5-t4).as_secs_f64(), "totalSeconds": (t5-t0).as_secs_f64(),
                "scalarSeconds": seconds[0], "interleavedSeconds": seconds[1],
                "dotComparisons": scores[0].len(), "scoreBitsSha256": hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
                "results": actual})).unwrap();
        }
    }
    stream.flush().unwrap();
}

#[test]
#[ignore = "frozen exposed-data phase diagnostic; explicit paths required"]
fn frozen_filtered_phase_diagnostic() {
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
        for state in ["baseline", "churn-maintained"] {
            if state != "baseline" {
                for id in 0..100 {
                    let document = bounded_document(id, vector(id + 1), dimensions, false);
                    store.insert(document.id.clone(), document);
                }
                for id in 100..200 {
                    store.remove(&format!("d{id:06}")).unwrap();
                }
            }
            bounded_train(&mut store, dimensions);
            for (label, filter) in [
                ("10pct", json!({"eligible10": true})),
                ("1pct", json!({"eligible1": true})),
            ] {
                phase_measure(&mut stream, name, state, label, filter, &store, &queries);
            }
        }
    }
}
