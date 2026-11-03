// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! K-mer encoder retrieval evaluation on seeded synthetic references.
//!
//! Random ACGT references; reads sampled from them (either strand) with per-base substitution
//! or indel noise; each read is scored against every reference and the rank of its source is
//! recorded. Methods: the `KmerEncoder` sequence vector scored with the engine's containment and
//! Jaccard overlap, exact canonical k-mer Jaccard and containment, bottom-s MinHash (Mash
//! estimator) and scaled MinHash containment (FracMinHash). All methods use the encoder's own
//! k-mer extraction, so alphabet, canonicalization and ambiguity rules are shared.
//!
//! Rank policy: `1 + #(other references scoring >= the source)`. Ties count against the method.
//!
//! `cargo run --release --features experimental --bin kmer-eval -- --out
//! benchmarks/results/kmer_encoder_eval_v1.json --code-rev <git rev>`

use rayon::prelude::*;
use serde_json::{json, Value};
use std::time::Instant;
use writerslogic_warm::core::entangled::{hash_u64, EntangledHVec};
use writerslogic_warm::core::kmer::{KmerConfig, KmerEncoder};

const MINHASH_SEED: u64 = 0x6b6d_6572_6d68_3031;
const SCALED_DENOM: u64 = 8;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Args {
    out: String,
    code_rev: String,
    seed: u64,
    refs: usize,
    ref_len: usize,
    read_len: usize,
    reads: usize,
    ks: Vec<usize>,
    dims: Vec<usize>,
}

fn parse_args() -> Args {
    let mut args = Args {
        out: "benchmarks/results/kmer_encoder_eval_v1.json".into(),
        code_rev: "unspecified".into(),
        seed: 20_261_103,
        refs: 1000,
        ref_len: 5000,
        read_len: 150,
        reads: 500,
        ks: vec![9, 11, 15, 21],
        dims: vec![1 << 16, 1 << 18, 1 << 20],
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let list = |s: String| -> Vec<usize> {
            s.split(',')
                .map(|x| x.parse().expect("integer list"))
                .collect()
        };
        match flag.as_str() {
            "--out" => args.out = value(),
            "--code-rev" => args.code_rev = value(),
            "--seed" => args.seed = value().parse().expect("seed"),
            "--refs" => args.refs = value().parse().expect("refs"),
            "--ref-len" => args.ref_len = value().parse().expect("ref-len"),
            "--read-len" => args.read_len = value().parse().expect("read-len"),
            "--reads" => args.reads = value().parse().expect("reads"),
            "--ks" => args.ks = list(value()),
            "--dims" => args.dims = list(value()),
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

struct Condition {
    name: String,
    substitution: f64,
    indel: f64,
}

fn conditions() -> Vec<Condition> {
    let mut all: Vec<Condition> = [0.0f64, 0.01, 0.05, 0.10, 0.15, 0.20]
        .iter()
        .map(|&p| Condition {
            name: format!("sub_{}pct", (p * 100.0).round() as u32),
            substitution: p,
            indel: 0.0,
        })
        .collect();
    for p in [0.01f64, 0.05] {
        all.push(Condition {
            name: format!("indel_{}pct", (p * 100.0).round() as u32),
            substitution: 0.0,
            indel: p,
        });
    }
    all
}

fn random_sequence(rng: &mut Rng, len: usize) -> Vec<u8> {
    (0..len).map(|_| b"ACGT"[rng.below(4)]).collect()
}

fn reverse_complement(sequence: &[u8]) -> Vec<u8> {
    sequence
        .iter()
        .rev()
        .map(|&b| match b {
            b'A' => b'T',
            b'C' => b'G',
            b'G' => b'C',
            _ => b'A',
        })
        .collect()
}

/// Substitutions replace a base by one of the other three; an indel position is a deletion or
/// an insertion of a random base before it with equal probability.
fn mutate(rng: &mut Rng, source: &[u8], substitution: f64, indel: f64) -> Vec<u8> {
    let mut out = Vec::with_capacity(source.len() + 16);
    for &base in source {
        if indel > 0.0 {
            let r = rng.unit();
            if r < indel / 2.0 {
                continue;
            }
            if r < indel {
                out.push(b"ACGT"[rng.below(4)]);
            }
        }
        if substitution > 0.0 && rng.unit() < substitution {
            let index = b"ACGT".iter().position(|&x| x == base).unwrap();
            out.push(b"ACGT"[(index + 1 + rng.below(3)) % 4]);
        } else {
            out.push(base);
        }
    }
    out
}

struct Read {
    source: usize,
    sequence: Vec<u8>,
}

fn sample_reads(args: &Args, refs: &[Vec<u8>], index: usize, cond: &Condition) -> Vec<Read> {
    let mut rng = Rng::new(hash_u64(args.seed, 100 + index as u64));
    (0..args.reads)
        .map(|_| {
            let source = rng.below(refs.len());
            let start = rng.below(args.ref_len - args.read_len + 1);
            let window = &refs[source][start..start + args.read_len];
            let mut sequence = mutate(&mut rng, window, cond.substitution, cond.indel);
            if rng.below(2) == 1 {
                sequence = reverse_complement(&sequence);
            }
            Read { source, sequence }
        })
        .collect()
}

fn wilson(hits: usize, n: usize) -> [f64; 2] {
    let (p, n) = (hits as f64 / n as f64, n as f64);
    let z = 1.96f64;
    let denom = 1.0 + z * z / n;
    let centre = (p + z * z / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
    [centre - half, centre + half]
}

/// Hits at rank 1 and rank 10 under the pessimistic tie policy.
fn recall(reads: &[Read], scorer: &(dyn Fn(&Read) -> Vec<f64> + Sync)) -> (usize, usize) {
    reads
        .par_iter()
        .map(|read| {
            let scores = scorer(read);
            let truth = scores[read.source];
            let rank = 1 + scores
                .iter()
                .enumerate()
                .filter(|&(i, &s)| i != read.source && s >= truth)
                .count();
            (usize::from(rank == 1), usize::from(rank <= 10))
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

fn distinct_codes(encoder: &KmerEncoder, sequence: &[u8]) -> Vec<u64> {
    let mut codes = encoder.kmer_codes(sequence).expect("valid sequence");
    codes.sort_unstable();
    codes.dedup();
    codes
}

fn intersection_sorted(small: &[u64], large: &[u64]) -> usize {
    small
        .iter()
        .filter(|c| large.binary_search(c).is_ok())
        .count()
}

fn bottom_sketch(codes: &[u64], size: usize) -> Vec<u64> {
    let mut hashes: Vec<u64> = codes.iter().map(|&c| hash_u64(MINHASH_SEED, c)).collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes.truncate(size);
    hashes
}

fn scaled_sketch(codes: &[u64]) -> Vec<u64> {
    let mut hashes: Vec<u64> = codes
        .iter()
        .map(|&c| hash_u64(MINHASH_SEED, c))
        .filter(|&h| h < u64::MAX / SCALED_DENOM)
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

/// Mash estimator: among the `size` smallest hashes of the union of the two sketches, the
/// fraction present in both.
fn bottom_jaccard(a: &[u64], b: &[u64], size: usize) -> f64 {
    let (mut i, mut j, mut taken, mut shared) = (0, 0, 0usize, 0usize);
    while taken < size && (i < a.len() || j < b.len()) {
        let (x, y) = (a.get(i).copied(), b.get(j).copied());
        match (x, y) {
            (Some(x), Some(y)) if x == y => {
                shared += 1;
                i += 1;
                j += 1;
            }
            (Some(x), Some(y)) if x < y => i += 1,
            (Some(_), Some(_)) => j += 1,
            (Some(_), None) => i += 1,
            (None, _) => j += 1,
        }
        taken += 1;
    }
    if taken == 0 {
        0.0
    } else {
        shared as f64 / taken as f64
    }
}

fn row(
    condition: &str,
    k: usize,
    method: &str,
    params: Value,
    bytes_per_ref: f64,
    n: usize,
    hits: (usize, usize),
) -> Value {
    json!({
        "condition": condition, "k": k, "method": method, "params": params,
        "bytes_per_ref": bytes_per_ref, "n_reads": n,
        "recall_at_1": hits.0 as f64 / n as f64, "recall_at_10": hits.1 as f64 / n as f64,
        "hits_at_1": hits.0, "hits_at_10": hits.1,
        "ci95_at_1": wilson(hits.0, n), "ci95_at_10": wilson(hits.1, n),
    })
}

fn main() {
    let args = parse_args();
    let started = Instant::now();
    let conds = conditions();
    let mut rng = Rng::new(hash_u64(args.seed, 1));
    let refs: Vec<Vec<u8>> = (0..args.refs)
        .map(|_| random_sequence(&mut rng, args.ref_len))
        .collect();
    let reads_by_cond: Vec<Vec<Read>> = conds
        .iter()
        .enumerate()
        .map(|(i, c)| sample_reads(&args, &refs, i, c))
        .collect();
    let mean_read_len: Vec<f64> = reads_by_cond
        .iter()
        .map(|r| r.iter().map(|x| x.sequence.len()).sum::<usize>() as f64 / r.len() as f64)
        .collect();

    let mut rows: Vec<Value> = Vec::new();
    let mut majority_check = Value::Null;
    let n = args.reads;

    for &k in &args.ks {
        eprintln!("k = {k}");
        let base = KmerConfig {
            k,
            canonical: true,
            seed: args.seed,
            ..KmerConfig::default()
        };
        let counter = KmerEncoder::new(base.clone()).unwrap();
        let ref_sets: Vec<Vec<u64>> = refs.iter().map(|r| distinct_codes(&counter, r)).collect();
        let mean_set = ref_sets.iter().map(Vec::len).sum::<usize>() as f64 / ref_sets.len() as f64;
        let bottom_sizes = [256usize, 2048];
        let bottoms: Vec<Vec<Vec<u64>>> = bottom_sizes
            .iter()
            .map(|&s| ref_sets.iter().map(|c| bottom_sketch(c, s)).collect())
            .collect();
        let scaled: Vec<Vec<u64>> = ref_sets.iter().map(|c| scaled_sketch(c)).collect();
        let mean_scaled = scaled.iter().map(Vec::len).sum::<usize>() as f64 / scaled.len() as f64;

        // SDR reference vectors per dim, plus a non-canonical encoder at the largest dim.
        let mut sdr_configs: Vec<(String, KmerConfig)> = args
            .dims
            .iter()
            .map(|&dim| {
                (
                    format!("sdr_canonical_dim{dim}"),
                    KmerConfig {
                        dim,
                        ..base.clone()
                    },
                )
            })
            .collect();
        if k == 15 {
            let dim = *args.dims.iter().max().unwrap();
            sdr_configs.push((
                format!("sdr_noncanonical_dim{dim}"),
                KmerConfig {
                    dim,
                    canonical: false,
                    ..base.clone()
                },
            ));
        }
        let sdrs: Vec<(String, KmerEncoder, Vec<EntangledHVec>, f64)> = sdr_configs
            .into_iter()
            .map(|(name, config)| {
                let encoder = KmerEncoder::new(config).unwrap();
                let vectors: Vec<EntangledHVec> = refs
                    .par_iter()
                    .map(|r| encoder.encode(r).unwrap())
                    .collect();
                let mean_active = vectors.iter().map(|v| v.indices().len()).sum::<usize>() as f64
                    / vectors.len() as f64;
                (name, encoder, vectors, mean_active)
            })
            .collect();

        if k == 15 && majority_check.is_null() {
            let (_, encoder, _, _) = &sdrs[0];
            let per_kmer: Vec<EntangledHVec> = counter
                .kmer_codes(&refs[0])
                .unwrap()
                .into_iter()
                .map(|c| encoder.kmer_sdr(c))
                .collect();
            let majority = EntangledHVec::bundle(&per_kmer);
            majority_check = json!({
                "k": k, "dim": encoder.config().dim, "bits_per_kmer": encoder.config().bits_per_kmer,
                "kmer_vectors_bundled": per_kmer.len(),
                "active_after_majority_bundle": majority.indices().len(),
                "active_after_union_bundle": sdrs[0].2[0].indices().len(),
            });
        }

        for (ci, cond) in conds.iter().enumerate() {
            let reads = &reads_by_cond[ci];
            let name = cond.name.as_str();

            let exact = |read: &Read, jaccard: bool| -> Vec<f64> {
                let codes = distinct_codes(&counter, &read.sequence);
                ref_sets
                    .iter()
                    .map(|set| {
                        let inter = intersection_sorted(&codes, set) as f64;
                        if jaccard {
                            inter / (codes.len() as f64 + set.len() as f64 - inter).max(1.0)
                        } else {
                            inter / (codes.len() as f64).max(1.0)
                        }
                    })
                    .collect()
            };
            rows.push(row(
                name,
                k,
                "exact_jaccard",
                json!({}),
                mean_set * 8.0,
                n,
                recall(reads, &|r| exact(r, true)),
            ));
            rows.push(row(
                name,
                k,
                "exact_containment",
                json!({}),
                mean_set * 8.0,
                n,
                recall(reads, &|r| exact(r, false)),
            ));

            for (si, &size) in bottom_sizes.iter().enumerate() {
                let sketches = &bottoms[si];
                let hits = recall(reads, &|r| {
                    let read_sketch = bottom_sketch(&distinct_codes(&counter, &r.sequence), size);
                    sketches
                        .iter()
                        .map(|s| bottom_jaccard(&read_sketch, s, size))
                        .collect()
                });
                rows.push(row(
                    name,
                    k,
                    "minhash_bottom_s_jaccard",
                    json!({ "s": size }),
                    (size * 8) as f64,
                    n,
                    hits,
                ));
            }
            let hits = recall(reads, &|r| {
                let read_sketch = scaled_sketch(&distinct_codes(&counter, &r.sequence));
                scaled
                    .iter()
                    .map(|s| {
                        intersection_sorted(&read_sketch, s) as f64
                            / (read_sketch.len() as f64).max(1.0)
                    })
                    .collect()
            });
            rows.push(row(
                name,
                k,
                "minhash_scaled_containment",
                json!({ "scale_denominator": SCALED_DENOM }),
                mean_scaled * 8.0,
                n,
                hits,
            ));

            for (sname, encoder, vectors, mean_active) in &sdrs {
                let params = json!({
                    "dim": encoder.config().dim, "bits_per_kmer": encoder.config().bits_per_kmer,
                    "canonical": encoder.config().canonical, "mean_active": mean_active,
                    "density": mean_active / encoder.config().dim as f64,
                });
                // Engine representation: sorted u32 index list.
                let bytes = mean_active * 4.0;
                let hits = recall(reads, &|r| {
                    let q = encoder.encode(&r.sequence).unwrap();
                    vectors
                        .iter()
                        .map(|v| q.containment_similarity(v))
                        .collect()
                });
                rows.push(row(
                    name,
                    k,
                    &format!("{sname}_containment"),
                    params.clone(),
                    bytes,
                    n,
                    hits,
                ));
                let hits = recall(reads, &|r| {
                    let q = encoder.encode(&r.sequence).unwrap();
                    vectors.iter().map(|v| q.similarity(v)).collect()
                });
                rows.push(row(
                    name,
                    k,
                    &format!("{sname}_jaccard"),
                    params,
                    bytes,
                    n,
                    hits,
                ));
            }
        }
        eprintln!("k = {k} done at {:.0}s", started.elapsed().as_secs_f64());
    }

    let report = json!({
        "schema": "kmer-encoder-eval-v1",
        "code_rev": args.code_rev,
        "data": "synthetic: i.i.d. uniform ACGT references; no real genomes were used",
        "seed": args.seed,
        "design": {
            "n_references": args.refs, "reference_length": args.ref_len,
            "read_length": args.read_len, "reads_per_condition": args.reads,
            "k_values": args.ks, "sdr_dims": args.dims, "sdr_bits_per_kmer": 2,
            "conditions": conds.iter().enumerate().map(|(i, c)| json!({
                "name": c.name, "substitution_rate": c.substitution, "indel_rate": c.indel,
                "mean_read_length": mean_read_len[i],
            })).collect::<Vec<_>>(),
            "noise_model": "per-base independent; substitution to one of the other 3 bases; indel position is a deletion or insertion of a random base with equal probability",
            "orientation": "each read is reverse-complemented with probability 0.5",
            "reads_sampled_with": "source reference and start uniform",
            "rank_policy": "rank = 1 + number of other references scoring >= the source reference (ties count against the method)",
            "chance_recall_at_1": 1.0 / args.refs as f64,
            "chance_recall_at_10": 10.0 / args.refs as f64,
            "bytes_per_ref": "exact: 8 bytes per distinct k-mer; minhash: 8 bytes per retained hash; sdr: 4 bytes per active index (engine sorted-u32 form)",
        },
        "majority_bundle_check": majority_check,
        "results": rows,
    });
    std::fs::write(
        &args.out,
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .expect("write result file");
    eprintln!(
        "wrote {} in {:.0}s",
        args.out,
        started.elapsed().as_secs_f64()
    );
}
