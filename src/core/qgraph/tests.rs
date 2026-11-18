// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn graph_build_parameters_match_durable_bounds() {
    let row = [1.0_f32, 0.0];
    for (degree, valid) in [
        (0, false),
        (1, false),
        (2, true),
        (65_535, false),
        (65_536, true),
        (65_537, false),
        (65_538, false),
        (usize::MAX - 1, false),
    ] {
        let params = BuildParams {
            degree,
            ..BuildParams::default()
        };
        assert_eq!(
            validate_build(&row, 2, &params).is_ok(),
            valid,
            "degree {degree}"
        );
    }
    for (build_ef, valid) in [
        (0, false),
        (1, true),
        (1_048_575, true),
        (1_048_576, true),
        (1_048_577, false),
        (usize::MAX, false),
    ] {
        let params = BuildParams {
            build_ef,
            ..BuildParams::default()
        };
        assert_eq!(
            validate_build(&row, 2, &params).is_ok(),
            valid,
            "build_ef {build_ef}"
        );
    }
    for (alpha, valid) in [
        (f32::NEG_INFINITY, false),
        (f32::NAN, false),
        (0.0, false),
        (f32::from_bits(1.0f32.to_bits() - 1), false),
        (1.0, true),
        (1.2, true),
        (f32::INFINITY, false),
    ] {
        let params = BuildParams {
            alpha,
            ..BuildParams::default()
        };
        assert_eq!(
            Graph::build(&row, 2, &params).is_ok(),
            valid,
            "alpha {alpha}"
        );
    }
    let oversized = BuildParams {
        degree: usize::MAX - 1,
        ..BuildParams::default()
    };
    assert!(Graph::build(&row, 2, &oversized).is_err());
    assert!(Graph::build_owned(row.to_vec(), 2, &oversized).is_err());
}

#[cfg(feature = "experimental-build-search")]
#[test]
fn build_pool_retirement_matches_ordered_merge_without_growth() {
    for limit in [
        1usize, 2, 3, 7, 8, 9, 31, 32, 33, 63, 64, 65, 199, 200, 201, 499, 500, 501,
    ] {
        for len in [0, limit - 1, limit] {
            let original: Vec<_> = (0..len)
                .map(|id| Candidate {
                    dist: (id / 3) as f32,
                    id: id as u32,
                    done: id % 3 == 0,
                })
                .collect();
            let bound = original.last().map_or(0.0, |c| c.dist);
            for dist in [
                f32::NEG_INFINITY,
                -1.0,
                -0.0,
                0.0,
                0.25,
                1.0,
                bound - 0.25,
                bound,
                bound + 0.25,
                f32::INFINITY,
                f32::from_bits(0x7fc0_1234),
            ] {
                let candidate = Candidate {
                    dist,
                    id: limit as u32,
                    done: false,
                };
                let expected_pos = if len == limit && dist >= bound {
                    None
                } else {
                    Some(original.iter().take_while(|c| c.dist <= dist).count())
                };
                let expected: Vec<_> = match expected_pos {
                    Some(pos) => original[..pos]
                        .iter()
                        .copied()
                        .chain(std::iter::once(candidate))
                        .chain(original[pos..].iter().copied())
                        .take(limit)
                        .collect(),
                    None => original.clone(),
                };
                let mut pool = Vec::with_capacity(limit);
                pool.extend_from_slice(&original);
                let capacity = pool.capacity();
                assert_eq!(
                    insert_build_candidate(&mut pool, candidate, limit),
                    expected_pos
                );
                assert_eq!(
                    pool.capacity(),
                    capacity,
                    "limit={limit}, len={len}, dist={dist}"
                );
                let bits = |entries: &[Candidate]| {
                    entries
                        .iter()
                        .map(|c| (c.dist.to_bits(), c.id, c.done))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    bits(&pool),
                    bits(&expected),
                    "limit={limit}, len={len}, dist={dist}"
                );
            }
        }
    }
}

/// Angular rows normalize to unit length in direction for finite magnitudes whose squared
/// norm overflows or underflows f32.
#[test]
fn angular_rows_normalize_at_extreme_magnitudes() {
    let mut rows = vec![
        1e20f32, 1e20, 0.0, 0.0, 1e-30, 1e-30, 1e-30, 1e-30, 3.0, 4.0, 0.0, 0.0,
    ];
    prepare_rows(&mut rows, 4, Metric::Angular);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    for (got, want) in rows[..8]
        .iter()
        .zip([half, half, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5])
    {
        assert!((got - want).abs() < 1e-6, "{rows:?}");
    }
    assert_eq!(&rows[8..], &[0.6, 0.8, 0.0, 0.0]);
}
use kernels::raw_estimates;

#[test]
fn build_visited_reuses_words_across_resets_and_graph_sizes() {
    let mut visited = BuildVisited::default();
    for n in [4097, 129, 128, 127, 65, 64, 63, 1, 0, 1, 63, 64, 65, 4097] {
        for _ in 0..3 {
            visited.reset(n);
            let mut reference = std::collections::HashSet::new();
            for id in (0..n as u32).rev().chain(0..n as u32) {
                assert_eq!(visited.insert(id), reference.insert(id));
            }
            assert_eq!(visited.touched.len(), n.div_ceil(64));
            let mut unique = std::collections::HashSet::new();
            assert!(visited
                .touched
                .iter()
                .all(|&word| visited.bits[word as usize] != 0 && unique.insert(word)));
            visited.reset(n);
            assert!(visited.touched.is_empty());
            assert!(visited.bits.iter().all(|&word| word == 0));
        }
    }
}

#[test]
fn build_visited_records_each_sparse_word_once() {
    let mut visited = BuildVisited::default();
    visited.reset(8_421_866);
    for id in [0, 1, 63, 64, 65, 127, 128, 8_421_865] {
        assert!(visited.insert(id));
        assert!(!visited.insert(id));
    }
    assert_eq!(visited.touched, [0, 1, 2, 131_591]);
    visited.reset(0);
    visited.reset(8_421_866);
    for id in [0, 63, 64, 127, 128, 8_421_865] {
        assert!(visited.insert(id));
    }
}

#[test]
#[ignore = "synthetic visited-reset timing diagnostic; run in isolation with --release"]
fn build_visited_reset_workload() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    let n = 8_421_866usize;
    for (visits, iterations) in [(64, 20_000), (1024, 10_000), (16_384, 1000), (n, 4)] {
        let ids: Vec<_> = (0..visits).map(|i| (i * 104_729 % n) as u32).collect();
        let words: std::collections::HashSet<_> = ids.iter().map(|id| id / 64).collect();
        let mut visited = BuildVisited::default();
        visited.reset(n);
        for &id in &ids {
            assert!(visited.insert(id));
        }
        let mut reset_time = Duration::ZERO;
        let start = Instant::now();
        for _ in 0..iterations {
            let reset_start = Instant::now();
            black_box(&mut visited).reset(black_box(n));
            reset_time += reset_start.elapsed();
            for &id in &ids {
                black_box(visited.insert(black_box(id)));
            }
        }
        println!(
            "{{\"vertices\":{n},\"visits\":{visits},\"touched_words\":{},\"full_words\":{},\"iterations\":{iterations},\"reset_ns\":{},\"total_ns\":{}}}",
            words.len(), n.div_ceil(64), reset_time.as_nanos(), start.elapsed().as_nanos()
        );
    }
}

pub(super) fn gaussian(rng: &mut SplitMix) -> f32 {
    let u = rng.next_f32().max(1e-7);
    let v = rng.next_f32();
    (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
}

fn unit(mut x: Vec<f32>) -> Vec<f32> {
    let n = dot(&x, &x).sqrt();
    x.iter_mut().for_each(|v| *v /= n);
    x
}

/// Mean of the shipped per-edge estimator over rotation seeds must match `<q, u>`, for the
/// automatic code length (128 bits at d = 100) and for 512-bit codes.
/// The WHT-with-signs rotation is not Haar, so the bound is empirical: 400 seeds put the
/// standard error near 0.003; tolerances are 0.02 (float query) and 0.03 (4-bit query).
#[test]
fn edge_estimator_is_unbiased_over_rotations() {
    let d = 100;
    let mut rng = SplitMix(42);
    for bits in [0, 0, 0, 512, 512, 512] {
        let q = unit((0..d).map(|_| gaussian(&mut rng)).collect());
        let v = unit((0..d).map(|_| gaussian(&mut rng)).collect());
        let u = unit(v.iter().map(|x| x + 0.15 * gaussian(&mut rng)).collect());
        let truth = dot(&q, &u);
        let (mut float_sum, mut quant_sum) = (0f64, 0f64);
        let seeds = 400;
        for seed in 0..seeds {
            let rot = Rotation::new(d, bits, seed);
            let p = rot.padded();
            let (mut pq, mut pv, mut pu) = (vec![0.0; p], vec![0.0; p], vec![0.0; p]);
            rot.apply(&q, &mut pq);
            rot.apply(&v, &mut pv);
            rot.apply(&u, &mut pu);
            let mut code = vec![0u64; p / 64];
            let (k, m, pop) = edge_factors(&pv, &pu, &mut code);
            let ipv = dot(&pq, &pv);
            let sq: f32 = pq
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    if code[i / 64] >> (i % 64) & 1 == 1 {
                        *x
                    } else {
                        -*x
                    }
                })
                .sum();
            float_sum += f64::from(ipv + k + m * sq);
            let mut qc = QueryCode::new(p / 64);
            qc.encode(&pq);
            let mut raw = [0u32];
            raw_estimates(&code, &qc.planes, p / 64, &mut raw);
            let est = ipv + k + m * (2.0 * qc.lo * pop - qc.sum + 2.0 * qc.delta * raw[0] as f32);
            quant_sum += f64::from(est);
        }
        let float_mean = float_sum / f64::from(seeds as u32);
        let quant_mean = quant_sum / f64::from(seeds as u32);
        assert!(
            (float_mean - f64::from(truth)).abs() < 0.02,
            "{float_mean} vs {truth}"
        );
        assert!(
            (quant_mean - f64::from(truth)).abs() < 0.03,
            "{quant_mean} vs {truth}"
        );
    }
}

pub(super) fn clustered(n: usize, d: usize, seed: u64) -> Vec<f32> {
    let mut rng = SplitMix(seed);
    let centers: Vec<Vec<f32>> = (0..20)
        .map(|_| (0..d).map(|_| gaussian(&mut rng)).collect())
        .collect();
    (0..n)
        .flat_map(|i| {
            let c = &centers[i % centers.len()];
            c.iter()
                .map(|x| x + 0.4 * gaussian(&mut rng))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) fn brute_force(data: &[f32], d: usize, q: &[f32], k: usize) -> Vec<u32> {
    let mut s: Vec<(f32, u32)> = data
        .chunks_exact(d)
        .enumerate()
        .map(|(i, x)| (-dot(x, q) / dot(x, x).sqrt(), i as u32))
        .collect();
    s.sort_by(|a, b| a.0.total_cmp(&b.0));
    s.into_iter().take(k).map(|x| x.1).collect()
}

#[test]
fn recall_on_clustered_data() {
    let d = 24;
    let data = clustered(4000, d, 1);
    let queries = clustered(100, d, 2);
    let index = QGraph::build(&data, d, &BuildParams::default()).expect("build");
    let mut s = index.searcher();
    let mut out = Vec::new();
    let mut hits = 0;
    for q in queries.chunks_exact(d) {
        let truth = brute_force(&data, d, q, 10);
        s.search(
            q,
            10,
            SearchParams {
                ef: 64,
                max_exact: 0,
            },
            &mut out,
        );
        hits += out.iter().filter(|x| truth.contains(x)).count();
    }
    let recall = hits as f64 / 1000.0;
    // Regression guard on a deliberately hard set (24 dimensions of noise per cluster).
    assert!(recall >= 0.9, "recall {recall}");
}

/// Each profile builds a deterministic graph that still finds the neighbours, the default
/// is the `Standard` schedule, and every other profile changes the graph.
#[test]
fn build_profiles_are_deterministic_and_searchable() {
    let d = 24;
    let data = clustered(1500, d, 1);
    let queries = clustered(100, d, 2);
    let base = BuildParams {
        build_ef: 400,
        ..BuildParams::default()
    };
    assert_eq!(base.profile, BuildProfile::Standard);
    let default_hash = Graph::build(&data, d, &base)
        .expect("build")
        .structure_hash();
    let mut seen = Vec::new();
    for profile in [
        BuildProfile::Standard,
        BuildProfile::MaxRecall,
        BuildProfile::High,
        BuildProfile::Balanced,
        BuildProfile::Fast,
    ] {
        assert_eq!(BuildProfile::from_name(profile.name()), Some(profile));
        let params = BuildParams {
            profile,
            ..base.clone()
        };
        let hash = Graph::build(&data, d, &params)
            .expect("build")
            .structure_hash();
        assert_eq!(
            hash,
            Graph::build(&data, d, &params)
                .expect("build")
                .structure_hash()
        );
        if profile == BuildProfile::Standard {
            assert_eq!(hash, default_hash);
        }
        assert!(
            !seen.contains(&hash),
            "{profile:?} must build its own graph"
        );
        seen.push(hash);
        let index = QGraph::build(&data, d, &params).expect("build");
        let mut s = index.searcher();
        let mut out = Vec::new();
        let mut hits = 0;
        for q in queries.chunks_exact(d) {
            let truth = brute_force(&data, d, q, 10);
            let p = SearchParams {
                ef: 64,
                max_exact: 0,
            };
            s.search(q, 10, p, &mut out);
            hits += out.iter().filter(|x| truth.contains(x)).count();
        }
        let recall = hits as f64 / 1000.0;
        assert!(recall >= 0.85, "{profile:?} recall {recall}");
    }
    assert_eq!(BuildProfile::from_name("nope"), None);
}

/// The reverse-edge prune that skips re-checks inside a closed prefix must equal a full
/// prune of the merged list.
#[test]
fn prune_merged_matches_prune() {
    let d = 12;
    let mut data = clustered(600, d, 5);
    data.chunks_exact_mut(d).for_each(|x| {
        let n = dot(x, x).sqrt();
        x.iter_mut().for_each(|v| *v /= n);
    });
    let rows = Rows {
        data: &data,
        dim: d,
        norms: None,
        quantized: false,
        bound: None,
        codes: None,
    };
    let mut rng = SplitMix(11);
    for trial in 0..200 {
        let t = (rng.next_u64() % 600) as u32;
        let (r, alpha) = (4 + trial % 13, [1.0, 1.2][trial % 2]);
        let all =
            |ids: &[u32]| -> Vec<(f32, u32)> { ids.iter().map(|&u| (rows.d2(t, u), u)).collect() };
        let pool: Vec<u32> = (0..60)
            .map(|_| (rng.next_u64() % 600) as u32)
            .filter(|&u| u != t)
            .collect();
        let mut list = rows.prune(t, all(&pool), alpha, r);
        let closed = list.len();
        for _ in 0..1 + trial % 9 {
            let u = (rng.next_u64() % 600) as u32;
            if u != t && !list.contains(&u) {
                list.push(u);
            }
        }
        assert_eq!(
            rows.prune_merged(t, &list, closed, alpha, r),
            rows.prune(t, all(&list), alpha, r),
            "trial {trial}"
        );
    }
}

/// The code filters of the construction search, the prune and the fill only skip exact
/// work, so they build the very graph exact scoring builds, including on near-duplicate
/// rows (bounds at their tightest) and a zero row; for Euclidean rows too, with norms
/// spread over two decades and an offset from the origin (bounds scaled by the norms).
#[test]
fn build_codes_leave_the_graph_unchanged() {
    for metric in [Metric::Angular, Metric::Euclidean] {
        for d in [100, 160] {
            let n = 1200;
            let mut data = clustered(n, d, 3);
            let mut rng = SplitMix(5);
            if metric == Metric::Euclidean {
                for (i, x) in data.chunks_exact_mut(d).enumerate() {
                    let s = 0.1 * 100f32.powf((i * 7919 % n) as f32 / n as f32);
                    x.iter_mut().for_each(|v| *v = s * (*v + 3.0));
                }
            }
            for i in (0..n).step_by(7) {
                let j = (rng.next_u64() % n as u64) as usize;
                for k in 0..d {
                    data[i * d + k] = data[j * d + k] * (1.0 + 1e-6 * gaussian(&mut rng));
                }
            }
            data[5 * d..6 * d].fill(0.0);
            let norms = prepare_rows(&mut data, d, metric);
            let codes = BuildCodes::new(&data, d, norms_opt(&norms));
            let build = |codes: Option<&BuildCodes>| {
                let rows = Rows {
                    data: &data,
                    dim: d,
                    norms: norms_opt(&norms),
                    codes,
                    quantized: false,
                    bound: None,
                };
                let order: Vec<u32> = (0..n as u32).rev().collect();
                let mut graph = Adjacency::new(n, 12);
                for alpha in [1.0, 1.2] {
                    rows.insert_pass(&mut graph, &order, 0, 40, alpha, 12, &[]);
                }
                rows.fill(&mut graph, 12);
                graph
            };
            assert_eq!(build(Some(&codes)), build(None), "{metric:?}, d = {d}");
        }
    }
}

#[test]
fn locality_order_leaves_the_graph_unchanged() {
    let (n, d) = (6000, 16);
    let mut data = clustered(n, d, 3);
    let norms = prepare_rows(&mut data, d, Metric::Euclidean);
    let rows = Rows {
        data: &data,
        dim: d,
        norms: norms_opt(&norms),
        codes: None,
        quantized: false,
        bound: None,
    };
    let order: Vec<u32> = (0..n as u32).rev().collect();
    let rank = rows.locality_rank(&order);
    assert_eq!(
        rank.len(),
        n,
        "a build this size must take the locality order"
    );
    let build = |rank: &[u32]| {
        let mut graph = Adjacency::new(n, 12);
        for alpha in [1.0, 1.2] {
            rows.insert_pass(&mut graph, &order, 0, 40, alpha, 12, rank);
        }
        graph
    };
    assert_eq!(build(&rank), build(&[]));
}

#[test]
fn build_and_search_are_deterministic() {
    let d = 16;
    let data = clustered(3000, d, 3);
    let params = BuildParams {
        seed: 9,
        ..BuildParams::default()
    };
    let a = QGraph::build(&data, d, &params).expect("build");
    let b = QGraph::build(&data, d, &params).expect("build");
    assert_eq!(a.entry, b.entry);
    assert!(a.blocks == b.blocks);
    let (mut sa, mut sb) = (a.searcher(), b.searcher());
    let (mut oa, mut ob) = (Vec::new(), Vec::new());
    let p = SearchParams {
        ef: 32,
        max_exact: 0,
    };
    for q in data.chunks_exact(d).take(50) {
        sa.search(q, 10, p, &mut oa);
        sb.search(q, 10, p, &mut ob);
        assert_eq!(oa, ob);
    }
}

/// The graphs a build produces, pinned by structure hash: a change to how construction
/// stores or schedules its work must leave every list, layer and the entry unchanged.
#[test]
fn build_reproduces_the_pinned_graphs() {
    let cases = [
        (Metric::Angular, false, 16, 0x2b16_3721_02ba_f39b),
        (Metric::Euclidean, true, 128, 0xe014_c782_9bed_e9aa),
    ];
    for (metric, quantized_search, d, want) in cases {
        let data = clustered(2500, d, 11);
        let params = BuildParams {
            metric,
            quantized_search,
            degree: 16,
            build_ef: 48,
            seed: 7,
            profile: BuildProfile::MaxRecall,
            ..BuildParams::default()
        };
        let g = Graph::build(&data, d, &params).expect("build");
        assert_eq!(g.structure_hash(), want, "{metric:?}");
    }
}

#[test]
fn exact_budget_caps_work() {
    let d = 16;
    let data = clustered(2000, d, 4);
    let index = QGraph::build(&data, d, &BuildParams::default()).expect("build");
    let mut s = index.searcher();
    let mut out = Vec::new();
    let q = &data[..d];
    let capped = s.search(
        q,
        10,
        SearchParams {
            ef: 128,
            max_exact: 20,
        },
        &mut out,
    );
    assert_eq!(out.len(), 10);
    let free = s.search(
        q,
        10,
        SearchParams {
            ef: 128,
            max_exact: 0,
        },
        &mut out,
    );
    assert!(capped < free);
    assert_eq!(capped - index.descend(q).1, 20);
}
