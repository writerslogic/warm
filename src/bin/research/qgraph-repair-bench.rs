// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Issue #165: query-reinforced edge repair against the current graph and a matched random
//! control, on held-out queries. One base graph per run; every arm is derived from it.

use anyhow::{ensure, Context, Result};
use clap::Parser;
use rayon::prelude::*;
use serde_json::{json, Value};
use std::{fs, io::Read, path::PathBuf, time::Instant};
use writerslogic_warm::core::qgraph::{
    BuildParams, Graph, Metric, RepairParams, SearchKnobs, VGraph, VSearchParams,
};

#[derive(Parser)]
struct Args {
    /// Prepared ANN directory (train.f32, test.f32 from benchmarks/public/prepare.py).
    #[arg(long)]
    data: PathBuf,
    #[arg(long)]
    dim: usize,
    /// angular or euclidean.
    #[arg(long)]
    metric: String,
    /// Base rows used (a prefix of train.f32); exact truth is recomputed for it.
    #[arg(long)]
    n: usize,
    /// test.f32 rows given to construction; the rest of the permuted rows are evaluation.
    #[arg(long, default_value_t = 5000)]
    train_split: usize,
    /// Additional construction queries from train.f32 rows after the base prefix.
    #[arg(long, default_value_t = 0)]
    extra_train_rows: usize,
    #[arg(long, default_value_t = 165)]
    split_seed: u64,
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "10,16,24,32,48,64,96,128,192,256"
    )]
    ef: Vec<usize>,
    #[arg(long, default_value_t = 3)]
    repeats: usize,
    #[arg(long, default_value_t = 32)]
    repair_ef: usize,
    #[arg(long, default_value_t = 0)]
    extra_degree: usize,
    #[arg(long, default_value_t = 2)]
    antihub: usize,
    #[arg(long, default_value_t = 64)]
    entry_points: usize,
    #[arg(long)]
    cache: PathBuf,
    #[arg(long)]
    out: PathBuf,
}

const K: usize = 10;

fn read_rows(path: &PathBuf, dim: usize, skip: usize, rows: usize) -> Result<Vec<f32>> {
    let width = dim * 4;
    let size = fs::metadata(path)?.len() as usize;
    ensure!(
        size.is_multiple_of(width) && (skip + rows) * width <= size,
        "{} holds {} rows, need {}",
        path.display(),
        size / width,
        skip + rows
    );
    let mut file = fs::File::open(path)?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start((skip * width) as u64))?;
    let mut bytes = vec![0u8; rows * width];
    file.read_exact(&mut bytes)?;
    let values: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| f32::from_le_bytes(*v))
        .collect();
    ensure!(values.iter().all(|v| v.is_finite()), "nonfinite input");
    Ok(values)
}

struct Stream(u64);
impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn fnv(ids: &[u32]) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in ids.iter().flat_map(|v| v.to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Exact top-K per query: cosine for angular, squared L2 for euclidean.
fn truth(base: &[f32], queries: &[f32], dim: usize, angular: bool) -> Vec<u32> {
    let norms: Vec<f32> = base
        .chunks_exact(dim)
        .map(|r| r.iter().map(|v| v * v).sum())
        .collect();
    queries
        .par_chunks_exact(dim)
        .flat_map_iter(|q| {
            let mut best: Vec<(f32, u32)> = Vec::with_capacity(K + 1);
            for (i, row) in base.chunks_exact(dim).enumerate() {
                let d: f32 = q.iter().zip(row).map(|(a, b)| a * b).sum();
                let s = if angular {
                    if norms[i] > 0.0 {
                        -d / norms[i].sqrt()
                    } else {
                        0.0
                    }
                } else {
                    norms[i] - 2.0 * d
                };
                if best.len() < K || s < best[K - 1].0 {
                    let at = best.partition_point(|c| c.0 <= s);
                    best.insert(at, (s, i as u32));
                    best.truncate(K);
                }
            }
            best.into_iter().map(|c| c.1).collect::<Vec<_>>()
        })
        .collect()
}

fn main() -> Result<()> {
    let args = Args::parse();
    let dim = args.dim;
    let metric = match args.metric.as_str() {
        "angular" => Metric::Angular,
        "euclidean" => Metric::Euclidean,
        other => anyhow::bail!("unknown metric {other}"),
    };
    ensure!(args.repeats >= 1 && args.repeats <= 11 && !args.ef.is_empty());
    let base = read_rows(&args.data.join("train.f32"), dim, 0, args.n)?;
    let test_path = args.data.join("test.f32");
    let test_rows = fs::metadata(&test_path)?.len() as usize / (dim * 4);
    let all_test = read_rows(&test_path, dim, 0, test_rows)?;
    ensure!(
        args.train_split < test_rows,
        "train split leaves no evaluation queries"
    );

    let mut perm: Vec<u32> = (0..test_rows as u32).collect();
    let mut stream = Stream(args.split_seed);
    for i in (1..perm.len()).rev() {
        perm.swap(i, (stream.next() % (i as u64 + 1)) as usize);
    }
    let (train_ids, eval_ids) = perm.split_at(args.train_split);
    let pick = |ids: &[u32]| -> Vec<f32> {
        ids.iter()
            .flat_map(|&i| &all_test[i as usize * dim..(i as usize + 1) * dim])
            .copied()
            .collect()
    };
    let mut train_q = pick(train_ids);
    if args.extra_train_rows > 0 {
        train_q.extend(read_rows(
            &args.data.join("train.f32"),
            dim,
            args.n,
            args.extra_train_rows,
        )?);
    }
    let eval_q = pick(eval_ids);
    let angular = metric == Metric::Angular;

    let start = Instant::now();
    let eval_truth = truth(&base, &eval_q, dim, angular);
    let eval_truth_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let train_truth = truth(&base, &train_q, dim, angular);
    let train_truth_seconds = start.elapsed().as_secs_f64();
    eprintln!("truth {eval_truth_seconds:.1}s eval, {train_truth_seconds:.1}s train");
    // At full scale the shipped ground truth must agree with the recomputed truth.
    let base_rows = fs::metadata(args.data.join("train.f32"))?.len() as usize / (dim * 4);
    let shipped_agreement = if args.n == base_rows {
        let bytes = fs::read(args.data.join("neighbors.i32"))?;
        let width = bytes.len() / 4 / test_rows;
        ensure!(
            width >= K && bytes.len() == test_rows * width * 4,
            "neighbors.i32 shape"
        );
        let shipped: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| i32::from_le_bytes(*v) as u32)
            .collect();
        let hits: usize = eval_ids
            .iter()
            .enumerate()
            .map(|(qi, &row)| {
                let s = &shipped[row as usize * width..row as usize * width + K];
                eval_truth[qi * K..(qi + 1) * K]
                    .iter()
                    .filter(|id| s.contains(id))
                    .count()
            })
            .sum();
        Some(hits as f64 / (eval_ids.len() * K) as f64)
    } else {
        None
    };

    let params = BuildParams {
        metric,
        degree: 32,
        build_ef: 200,
        alpha: 1.0,
        seed: 0x5EED,
        ..Default::default()
    };
    let start = Instant::now();
    let graph = Graph::build(&base, dim, &params)?;
    let build_seconds = start.elapsed().as_secs_f64();
    graph.save(&args.cache).context("save base graph cache")?;
    let base_hash = format!("{:016x}", graph.structure_hash());
    drop(graph);
    eprintln!("base build {build_seconds:.1}s");

    let full = RepairParams {
        k: K,
        ef: args.repair_ef,
        extra_degree: args.extra_degree,
        failure_edges: true,
        antihub_in_degree: args.antihub,
        entry_points: args.entry_points,
        random_targets: false,
        seed: 0x5EED,
    };
    let arms: Vec<(&str, Option<RepairParams>)> = vec![
        ("base", None),
        (
            "failure",
            Some(RepairParams {
                antihub_in_degree: 0,
                entry_points: 0,
                ..full.clone()
            }),
        ),
        (
            "failure_antihub",
            Some(RepairParams {
                entry_points: 0,
                ..full.clone()
            }),
        ),
        ("full", Some(full.clone())),
        (
            "control_random",
            Some(RepairParams {
                random_targets: true,
                ..full.clone()
            }),
        ),
        (
            "entry_only",
            Some(RepairParams {
                failure_edges: false,
                antihub_in_degree: 0,
                ..full.clone()
            }),
        ),
    ];
    let mut indexes = Vec::new();
    let mut arm_reports = Vec::new();
    for (name, repair) in &arms {
        let mut g = Graph::load(&base, dim, &args.cache, &params)?;
        let start = Instant::now();
        let stats = match repair {
            Some(r) => Some(g.apply_query_repair_with_neighbors(&train_q, &train_truth, r)?),
            None => None,
        };
        let repair_seconds = start.elapsed().as_secs_f64();
        let hash = format!("{:016x}", g.structure_hash());
        let start = Instant::now();
        let index = VGraph::from_graph_with(&g, 8, 8, true, false, 3, false)?;
        let encode_seconds = start.elapsed().as_secs_f64();
        eprintln!("{name}: repair {repair_seconds:.1}s, {stats:?}");
        // In-sample check: recall of the construction queries themselves at the repair ef.
        let mut searcher = index.searcher_with(SearchKnobs::OFF);
        let mut in_sample = Vec::new();
        let mut hits = 0usize;
        for (qi, q) in train_q.chunks_exact(dim).enumerate() {
            let search = VSearchParams {
                ef: args.repair_ef,
                rerank: 16,
                patience: 0,
                screen: false,
                ..Default::default()
            };
            searcher.search(q, K, search, &mut in_sample)?;
            let t = &train_truth[qi * K..(qi + 1) * K];
            hits += in_sample.iter().filter(|id| t.contains(id)).count();
        }
        let train_recall = hits as f64 / (train_q.len() / dim * K) as f64;
        drop(searcher);
        arm_reports.push(json!({"arm": name, "repair": repair, "stats": stats,
            "repair_seconds": repair_seconds, "encode_seconds": encode_seconds,
            "structure_hash": hash, "index_bytes": index.index_bytes(),
            "train_recall_at_repair_ef_served": train_recall}));
        indexes.push(index);
    }

    let mut records: Vec<Value> = Vec::new();
    let mut found = Vec::new();
    for rep in 0..args.repeats {
        for (ei, &ef) in args.ef.iter().enumerate() {
            for offset in 0..arms.len() {
                let a = (offset + rep + ei) % arms.len();
                let mut searcher = indexes[a].searcher_with(SearchKnobs::OFF);
                let search = VSearchParams {
                    ef,
                    rerank: 16,
                    patience: 0,
                    screen: false,
                    ..Default::default()
                };
                let (mut hits, mut evaluations) = (0usize, 0usize);
                let start = Instant::now();
                for (qi, q) in eval_q.chunks_exact(dim).enumerate() {
                    evaluations += searcher.search(q, K, search, &mut found)?;
                    let t = &eval_truth[qi * K..(qi + 1) * K];
                    hits += found.iter().filter(|id| t.contains(id)).count();
                }
                let seconds = start.elapsed().as_secs_f64();
                let queries = eval_ids.len();
                records.push(json!({"arm": arms[a].0, "ef": ef, "repetition": rep,
                    "recall_at_10": hits as f64 / (queries * K) as f64,
                    "qps": queries as f64 / seconds,
                    "mean_evaluations": evaluations as f64 / queries as f64}));
            }
        }
        eprintln!("repetition {rep} done");
    }
    let report = json!({
        "schema_version": 1, "issue": 165, "data": args.data, "dim": dim, "metric": args.metric,
        "n": args.n, "k": K, "split": {
            "source": "permutation of test.f32 rows by splitmix64 Fisher-Yates",
            "split_seed": args.split_seed, "test_rows": test_rows,
            "train_test_rows": train_ids.len(), "train_ids_fnv": fnv(train_ids),
            "eval_test_rows": eval_ids.len(), "eval_ids_fnv": fnv(eval_ids),
            "extra_train_rows_from_base_suffix": args.extra_train_rows,
            "extra_train_row_range": [args.n, args.n + args.extra_train_rows],
            "train_ids": train_ids, "eval_ids": eval_ids},
        "build": {"degree": params.degree, "build_ef": params.build_ef, "alpha": params.alpha,
            "seed": params.seed, "encoding": "from_graph_with(8, 8, align, no reorder, 3-byte ids, no screen)",
            "build_seconds": build_seconds, "base_structure_hash": base_hash},
        "search": {"k": K, "rerank": 16, "patience": 0, "screen": false, "threads": 1,
            "knobs": "SearchKnobs::OFF", "timing": "one pass over the evaluation queries per (arm, ef, repetition), arm order rotated"},
        "truth": {"eval_seconds": eval_truth_seconds, "train_seconds": train_truth_seconds,
            "method": "exact f32 brute force over the base prefix",
            "shipped_top10_agreement_on_eval": shipped_agreement},
        "arms": arm_reports, "records": records,
    });
    fs::write(&args.out, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
