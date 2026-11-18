// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

/// Lexical latency microbenchmark on an NFCorpus-shaped synthetic store (3633 one-chunk
/// documents, Zipf vocabulary, 324 queries); run ignored with `--nocapture`, and set
/// `WARM_LEX_BENCH_OUT` to write the phase timings as JSON.
use super::*;
use crate::core::encoding::TextEncoder;
use serde_json::json;
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn zipf_table(size: usize, exponent: f64) -> Vec<f64> {
    let mut cumulative = Vec::with_capacity(size);
    let mut total = 0.0;
    for i in 1..=size {
        total += 1.0 / (i as f64).powf(exponent);
        cumulative.push(total);
    }
    for value in &mut cumulative {
        *value /= total;
    }
    cumulative
}

fn sample(table: &[f64], rng: &mut Rng) -> usize {
    let u = rng.unit();
    table.partition_point(|&c| c < u).min(table.len() - 1)
}

fn word(i: usize) -> String {
    format!("w{i}x")
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn zipf_store(documents: usize, vocabulary: usize, rng: &mut Rng) -> DocumentStore {
    let body = zipf_table(vocabulary, 0.9);
    let mut store = DocumentStore::default();
    for id in 0..documents {
        let words: Vec<String> = (0..200).map(|_| word(sample(&body, rng))).collect();
        let input = serde_json::from_value(json!({
            "id": format!("MED-{id}"), "text": words.join(" "), "storeText": false,
            "metadata": {"title": format!("title {id}"), "year": 2000 + id % 20},
            "chunkWords": 4096, "overlapWords": 0,
        }))
        .unwrap();
        let document = super::super::prepare(input, 1024, None, TextEncoder::default()).unwrap();
        store.insert(document.id.clone(), document);
    }
    store
}

fn zipf_queries(count: usize, vocabulary: usize, rng: &mut Rng) -> Vec<String> {
    let table = zipf_table(vocabulary, 1.0);
    (0..count)
        .map(|_| {
            let n = 3 + (rng.next() % 5) as usize;
            (0..n)
                .map(|_| word(sample(&table, rng)))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
#[ignore]
fn lexical_latency_microbench() {
    let documents = 3633;
    let vocabulary = 25_000;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (store, queries) = match crate::compatibility::var("WARM_LEX_BENCH_INPUT") {
        Ok(path) => {
            let input: Value =
                serde_json::from_slice(&std::fs::read(path).expect("bench input")).unwrap();
            let mut store = DocumentStore::default();
            for row in input["documents"].as_array().unwrap() {
                let input = serde_json::from_value(json!({
                    "id": row["id"], "text": row["text"], "storeText": false,
                    "chunkWords": 4096, "overlapWords": 0,
                }))
                .unwrap();
                let document =
                    super::super::prepare(input, 1024, None, TextEncoder::default()).unwrap();
                store.insert(document.id.clone(), document);
            }
            let queries = input["queries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|q| q["text"].as_str().unwrap().to_owned())
                .collect();
            (store, queries)
        }
        Err(_) => (
            zipf_store(documents, vocabulary, &mut rng),
            zipf_queries(324, vocabulary, &mut rng),
        ),
    };
    {
        let k: u32 = crate::compatibility::var("WARM_LEX_BENCH_K")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);
        let options = SearchOptions {
            k: Some(k),
            candidate_limit: Some(k.max(100)),
            lexical_weight: Some(1.0),
            semantic_weight: Some(0.0),
            ..Default::default()
        };
        let passes = 8;
        let mut prepare_ns = Vec::new();
        let mut capture_ns = Vec::new();
        let mut execute_ns = Vec::new();
        let mut total_ns = Vec::new();
        let mut postings = 0usize;
        let mut results = 0usize;
        for pass in 0..passes {
            for text in &queries {
                let t0 = Instant::now();
                let prepared = prepare_search(text, &options, None).unwrap();
                let t1 = Instant::now();
                let captured = capture_search(&store, vec![prepared]);
                let t2 = Instant::now();
                let hits = execute_search(&captured, 0).unwrap();
                let t3 = Instant::now();
                if pass == 0 {
                    let query = &captured.queries[0].prepared;
                    postings += (0..2)
                        .map(|form| {
                            query.query_terms[form]
                                .iter()
                                .map(|word| {
                                    store.postings[form].get(word).map_or(0, PostingList::len)
                                })
                                .sum::<usize>()
                        })
                        .sum::<usize>();
                    results += hits.len();
                }
                prepare_ns.push((t1 - t0).as_nanos() as u64);
                capture_ns.push((t2 - t1).as_nanos() as u64);
                execute_ns.push((t3 - t2).as_nanos() as u64);
                total_ns.push((t3 - t0).as_nanos() as u64);
            }
        }
        let summary = |name: &str, values: &mut Vec<u64>| {
            values.sort_unstable();
            let mean = values.iter().sum::<u64>() / values.len() as u64;
            json!({"phase": name, "mean_ns": mean, "p50_ns": percentile(values, 0.5),
               "p99_ns": percentile(values, 0.99)})
        };
        let report = json!({
            "store_chunks": store.chunk_count,
            "documents": documents, "vocabulary": vocabulary, "queries": queries.len(),
            "passes": passes, "postings_per_query": postings as f64 / queries.len() as f64,
            "results_per_query": results as f64 / queries.len() as f64,
            "phases": [summary("prepare", &mut prepare_ns), summary("capture", &mut capture_ns),
                       summary("execute", &mut execute_ns), summary("total", &mut total_ns)],
        });
        println!("{report}");
        if let Ok(path) = crate::compatibility::var("WARM_LEX_BENCH_OUT") {
            std::fs::write(path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
        }
    }
}
