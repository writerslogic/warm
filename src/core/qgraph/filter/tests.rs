// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn audit_regression_filtered_index_rejects_unrepresentable_shapes() {
    let params = FilterBuildParams::default();
    assert!(FilteredIndex::build(&[], 16, &[0], &[], usize::MAX, &params).is_err());
    let maximum = (u32::MAX / (255 * 255)) as usize;
    for dim in [maximum - 1, maximum, maximum + 1] {
        let data = vec![0; dim];
        let built = FilteredIndex::build(&data, dim, &[0, 1], &[0], 1, &params);
        if dim > maximum {
            assert!(built.is_err());
        } else {
            let mut out = Vec::new();
            built
                .unwrap()
                .search(
                    &vec![255; dim],
                    &[0],
                    1,
                    &FilterSearchParams::exact(),
                    &mut out,
                )
                .unwrap();
            assert_eq!(out, vec![((65025 * dim) as u32, 0)]);
        }
    }
    assert!(FilteredIndex::build(
        &[0; 16],
        16,
        &[0, 1],
        &[0],
        1,
        &FilterBuildParams {
            ivf_min: 0,
            train_per_list: 0,
            ..params
        },
    )
    .is_err());
}

#[test]
fn audit_regression_filtered_search_bounds_result_capacity_and_batch_products() {
    let index = FilteredIndex::build(
        &[0; 16],
        16,
        &[0, 1],
        &[0],
        1,
        &FilterBuildParams::default(),
    )
    .unwrap();
    for k in [0, 1, 2, usize::MAX] {
        let mut out = Vec::new();
        index
            .search(&[0; 16], &[0], k, &FilterSearchParams::exact(), &mut out)
            .unwrap();
        assert_eq!(out.len(), usize::from(k != 0));
    }
    assert!(index
        .search_batch(
            &[0; 32],
            &[0, 1, 2],
            &[0, 0],
            usize::MAX,
            &FilterSearchParams::exact(),
            &mut []
        )
        .is_err());
    assert!(index
        .search_batch(
            &[0; 16],
            &[1, 1],
            &[0],
            0,
            &FilterSearchParams::exact(),
            &mut []
        )
        .is_err());
}

struct Fixture {
    data: Vec<u8>,
    indptr: Vec<u64>,
    ids: Vec<u32>,
    dim: usize,
}

/// Clustered u8 rows with a heavy-tailed label distribution; coordinates are drawn from a
/// few values so exact distance ties occur.
fn fixture(n: usize, dim: usize, labels: u32, seed: u64) -> Fixture {
    let mut s = seed;
    let mut next = || {
        s = mix(s);
        s
    };
    let centers: Vec<Vec<u8>> = (0..8)
        .map(|_| (0..dim).map(|_| (next() % 256) as u8).collect())
        .collect();
    let mut data = Vec::with_capacity(n * dim);
    let mut indptr = vec![0u64];
    let mut ids = Vec::new();
    for _ in 0..n {
        let c = &centers[(next() % 8) as usize];
        for &v in c {
            let noise = (next() % 5) as i32 * 8 - 16;
            data.push((i32::from(v) + noise).clamp(0, 255) as u8);
        }
        let tags = 1 + next() % 5;
        for _ in 0..tags {
            // Zipf-like: small label ids are common.
            let r = (next() % 10_000) as f64 / 10_000.0;
            ids.push(((r * r * r) * f64::from(labels)) as u32);
        }
        indptr.push(ids.len() as u64);
    }
    Fixture {
        data,
        indptr,
        ids,
        dim,
    }
}

fn brute(f: &Fixture, deleted: &[bool], q: &[u8], labels: &[u32], k: usize) -> Vec<(u32, u32)> {
    let n = f.data.len() / f.dim;
    let mut all: Vec<(u32, u32)> = (0..n)
        .filter(|&p| !deleted[p])
        .filter(|&p| {
            let tags = &f.ids[f.indptr[p] as usize..f.indptr[p + 1] as usize];
            labels.iter().all(|l| tags.contains(l))
        })
        .map(|p| {
            let row = &f.data[p * f.dim..(p + 1) * f.dim];
            let d: u32 = row
                .iter()
                .zip(q)
                .map(|(&a, &b)| (i32::from(a) - i32::from(b)).pow(2) as u32)
                .sum();
            (d, p as u32)
        })
        .collect();
    all.sort_unstable();
    all.truncate(k);
    all
}

fn queries(f: &Fixture, count: usize, labels: u32, seed: u64) -> Vec<(Vec<u8>, Vec<u32>)> {
    let mut s = seed;
    let mut next = || {
        s = mix(s);
        s
    };
    let n = f.data.len() / f.dim;
    (0..count)
        .map(|i| {
            let p = (next() % n as u64) as usize;
            let q: Vec<u8> = f.data[p * f.dim..(p + 1) * f.dim]
                .iter()
                .map(|&v| v.wrapping_add((next() % 3) as u8))
                .collect();
            let tags = &f.ids[f.indptr[p] as usize..f.indptr[p + 1] as usize];
            let mut ls = vec![tags[(next() % tags.len() as u64) as usize]];
            if i % 2 == 1 {
                ls.push((next() % u64::from(labels)) as u32);
            }
            (q, ls)
        })
        .collect()
}

fn small_params() -> FilterBuildParams {
    FilterBuildParams {
        ivf_min: 200,
        cluster_size: 50,
        max_lists: 64,
        bitset_min: 300,
        kmeans_iters: 4,
        train_per_list: 16,
        copy_min: 400,
        seed: 7,
        pq_m: 0,
        pq_sample: 1_000,
        pq_iters: 4,
    }
}

#[test]
fn screen_with_full_rerank_equals_filtered_brute_force() {
    let labels = 40;
    let f = fixture(4_000, 32, labels, 1);
    let build = FilterBuildParams {
        pq_m: 8,
        ..small_params()
    };
    let idx = FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &build).unwrap();
    let deleted = vec![false; 4_000];
    let p = FilterSearchParams {
        brute_max: 0,
        ..FilterSearchParams::exact()
    };
    let mut out = Vec::new();
    for (q, ls) in queries(&f, 300, labels, 2) {
        idx.search(&q, &ls, 10, &p, &mut out).unwrap();
        assert_eq!(out, brute(&f, &deleted, &q, &ls, 10), "labels {ls:?}");
    }
    assert!(idx.stats().ivf > 0);
}

#[test]
fn exact_setting_equals_filtered_brute_force() {
    let labels = 40;
    let f = fixture(4_000, 32, labels, 1);
    let idx = FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &small_params()).unwrap();
    assert!(
        idx.ivf_labels() > 0,
        "fixture must exercise the inverted-file path"
    );
    let deleted = vec![false; 4_000];
    let mut out = Vec::new();
    for (q, ls) in queries(&f, 300, labels, 2) {
        // brute_max 0 forces the inverted file for every label that has one.
        let ivf_exact = FilterSearchParams {
            brute_max: 0,
            ..FilterSearchParams::exact()
        };
        for p in [FilterSearchParams::exact(), ivf_exact] {
            idx.search(&q, &ls, 10, &p, &mut out).unwrap();
            assert_eq!(out, brute(&f, &deleted, &q, &ls, 10), "labels {ls:?}");
        }
    }
    let s = idx.stats();
    assert!(s.brute > 0 && s.ivf > 0, "{s:?}");
}

#[test]
fn deletes_are_never_returned_and_rest_stays_exact() {
    let labels = 30;
    let f = fixture(3_000, 20, labels, 3);
    let mut idx =
        FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &small_params()).unwrap();
    let mut deleted = vec![false; 3_000];
    for id in (0..3_000).step_by(3) {
        assert!(idx.delete(id as u32));
        deleted[id] = true;
    }
    assert!(!idx.delete(0));
    assert_eq!(idx.live(), 2_000);
    let mut out = Vec::new();
    for (q, ls) in queries(&f, 200, labels, 4) {
        idx.search(
            &q,
            &ls,
            7,
            &FilterSearchParams {
                brute_max: 0,
                ..FilterSearchParams::exact()
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(out, brute(&f, &deleted, &q, &ls, 7));
    }
}

#[test]
fn batch_matches_single_and_pads_with_minus_one() {
    let labels = 25;
    let f = fixture(2_000, 16, labels, 5);
    let idx = FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &small_params()).unwrap();
    let qs = queries(&f, 64, labels, 6);
    let mut flat = Vec::new();
    let mut indptr = vec![0u64];
    let mut lids = Vec::new();
    for (q, ls) in &qs {
        flat.extend_from_slice(q);
        lids.extend_from_slice(ls);
        indptr.push(lids.len() as u64);
    }
    let k = 10;
    let p = FilterSearchParams::default();
    let mut out = vec![0i64; qs.len() * k];
    idx.search_batch(&flat, &indptr, &lids, k, &p, &mut out)
        .unwrap();
    let mut one = Vec::new();
    for (i, (q, ls)) in qs.iter().enumerate() {
        idx.search(q, ls, k, &p, &mut one).unwrap();
        let row = &out[i * k..(i + 1) * k];
        for (j, &got) in row.iter().enumerate() {
            assert_eq!(got, one.get(j).map_or(-1, |&(_, id)| i64::from(id)));
        }
    }
    // A label nobody holds and an out-of-range label both match nothing.
    idx.search(&qs[0].0, &[labels + 100], k, &p, &mut one)
        .unwrap();
    assert!(one.is_empty());
}

#[test]
fn tuning_never_changes_a_result() {
    let labels = 25;
    let f = fixture(3_000, 32, labels, 9);
    let mut qs = queries(&f, 96, labels, 10);
    // Repeated label sets (reversed order too) with other vectors, for the intersection
    // cache.
    for i in 0..48 {
        let mut ls = qs[i].1.clone();
        ls.reverse();
        qs.push((qs[i + 48].0.clone(), ls));
    }
    let mut flat = Vec::new();
    let mut indptr = vec![0u64];
    let mut lids = Vec::new();
    for (q, ls) in &qs {
        flat.extend_from_slice(q);
        lids.extend_from_slice(ls);
        indptr.push(lids.len() as u64);
    }
    let k = 10;
    // Bitsets for every label, for the big ones only, and for none; with and without
    // tombstones.
    for bitset_min in [1, 300, usize::MAX] {
        for deletes in [false, true] {
            let build = FilterBuildParams {
                bitset_min,
                ..small_params()
            };
            let mut idx =
                FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &build).unwrap();
            if deletes {
                for id in (0..3_000).step_by(7) {
                    idx.delete(id);
                }
            }
            // Both routes: a small brute_max sends the big labels to their lists.
            for p in [
                FilterSearchParams::default(),
                FilterSearchParams {
                    brute_max: 20,
                    target: 60,
                    ..FilterSearchParams::default()
                },
            ] {
                idx.set_tuning(FilterTuning::plain());
                let mut want = vec![0i64; qs.len() * k];
                idx.search_batch(&flat, &indptr, &lids, k, &p, &mut want)
                    .unwrap();
                for bits in 0..32u32 {
                    idx.set_tuning(FilterTuning {
                        sort_queries: bits & 1 != 0,
                        prefetch: [0, 1, 24][(bits >> 1) as usize % 3],
                        wide: bits & 2 != 0,
                        skip_tombstones: bits & 4 != 0,
                        block_filter: bits & 8 != 0,
                        cache_intersections: bits & 16 != 0,
                        ..FilterTuning::plain()
                    });
                    let mut got = vec![0i64; qs.len() * k];
                    idx.search_batch(&flat, &indptr, &lids, k, &p, &mut got)
                        .unwrap();
                    assert_eq!(got, want, "{bitset_min} {deletes} {bits}");
                }
                // Bands that restate `target` leave every result as it was.
                idx.set_tuning(FilterTuning {
                    class_params: [[(p.target, 0); 4]; 2],
                    ..FilterTuning::plain()
                });
                let mut got = vec![0i64; qs.len() * k];
                idx.search_batch(&flat, &indptr, &lids, k, &p, &mut got)
                    .unwrap();
                assert_eq!(got, want, "bands {bitset_min} {deletes}");
            }
        }
    }
}

#[test]
fn reused_scratch_preserves_results_across_routes_and_early_returns() {
    let f = fixture(1_000, 20, 12, 71);
    for copy_min in [0, usize::MAX] {
        let mut idx = FilteredIndex::build(
            &f.data,
            f.dim,
            &f.indptr,
            &f.ids,
            0,
            &FilterBuildParams {
                ivf_min: 20,
                cluster_size: 17,
                max_lists: 32,
                bitset_min: 250,
                copy_min,
                ..small_params()
            },
        )
        .unwrap();
        let mut scratch = SearchScratch::default();
        let mut reused = Vec::new();
        let mut fresh = Vec::new();
        let mut deleted = vec![false; 1_000];
        let qs = queries(&f, 30, 12, 72);
        for round in 0..2 {
            if round == 1 {
                for id in (0..1_000).step_by(7) {
                    idx.delete(id as u32);
                    deleted[id] = true;
                }
            }
            for (i, (q, labels)) in qs.iter().enumerate() {
                let mut duplicated = labels.clone();
                duplicated.extend_from_slice(labels);
                for k in [0, 1, 7, 8, 9, 31] {
                    for params in [
                        FilterSearchParams::exact(),
                        FilterSearchParams {
                            brute_max: 0,
                            intersect_max: 0,
                            rerank: 0,
                            target: 80,
                            min_lists: 2,
                        },
                        FilterSearchParams {
                            brute_max: 0,
                            ..FilterSearchParams::exact()
                        },
                    ] {
                        idx.reset_stats();
                        idx.search(q, &duplicated, k, &params, &mut fresh).unwrap();
                        let fresh_stats = idx.stats();
                        idx.reset_stats();
                        idx.search_with_scratch(
                            q,
                            &duplicated,
                            k,
                            &params,
                            &mut reused,
                            &mut scratch,
                        )
                        .unwrap();
                        assert_eq!(reused, fresh, "round {round}, query {i}, k {k}");
                        assert_eq!(idx.stats(), fresh_stats);
                        if params.target == usize::MAX && k != 0 {
                            assert_eq!(reused, brute(&f, &deleted, q, labels, k));
                        }
                        for empty_labels in [&[][..], &[u32::MAX][..]] {
                            idx.search_with_scratch(
                                q,
                                empty_labels,
                                k,
                                &params,
                                &mut reused,
                                &mut scratch,
                            )
                            .unwrap();
                            assert!(reused.is_empty());
                        }
                        assert!(idx
                            .search_with_scratch(
                                &q[..19],
                                labels,
                                k,
                                &params,
                                &mut reused,
                                &mut scratch
                            )
                            .is_err());
                        assert!(reused.is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn batched_scratch_matches_independent_exact_results_after_deletions() {
    let f = fixture(600, 17, 10, 81);
    let mut idx =
        FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &small_params()).unwrap();
    let mut deleted = vec![false; 600];
    for id in (0..600).step_by(4) {
        idx.delete(id as u32);
        deleted[id] = true;
    }
    let mut qs = queries(&f, 64, 10, 82);
    for (i, (_, labels)) in qs.iter_mut().enumerate() {
        match i % 5 {
            0 => labels.clear(),
            1 => labels.push(u32::MAX),
            2 => labels.extend_from_within(..),
            _ => {}
        }
    }
    let flat: Vec<u8> = qs.iter().flat_map(|(q, _)| q.iter().copied()).collect();
    let mut indptr = vec![0u64];
    let mut label_ids = Vec::new();
    for (_, labels) in &qs {
        label_ids.extend_from_slice(labels);
        indptr.push(label_ids.len() as u64);
    }
    for k in [1, 9, 10, 11] {
        let mut out = vec![-7; qs.len() * k];
        idx.search_batch(
            &flat,
            &indptr,
            &label_ids,
            k,
            &FilterSearchParams::exact(),
            &mut out,
        )
        .unwrap();
        for ((q, labels), got) in qs.iter().zip(out.chunks(k)) {
            let expected = if labels.is_empty() {
                Vec::new()
            } else {
                brute(&f, &deleted, q, labels, k)
            };
            for (slot, &actual) in got.iter().enumerate() {
                assert_eq!(
                    actual,
                    expected.get(slot).map_or(-1, |&(_, id)| i64::from(id))
                );
            }
        }
    }
}

#[test]
fn rejects_malformed_input() {
    let f = fixture(100, 16, 5, 9);
    let p = small_params();
    assert!(FilteredIndex::build(&f.data, 15, &f.indptr, &f.ids, 0, &p).is_err());
    assert!(FilteredIndex::build(&f.data, 16, &f.indptr[..50], &f.ids, 0, &p).is_err());
    assert!(FilteredIndex::build(&f.data, 16, &f.indptr, &f.ids, 2, &p).is_err());
    let idx = FilteredIndex::build(&f.data, 16, &f.indptr, &f.ids, 0, &p).unwrap();
    let mut out = Vec::new();
    assert!(idx
        .search(&[0u8; 8], &[0], 3, &FilterSearchParams::default(), &mut out)
        .is_err());
}
#[test]
fn centroid_batches_preserve_probe_order_at_tails_padding_and_ties() {
    for dim in [15usize, 16, 17, 191, 192, 193] {
        for lists in [1usize, 2, 3, 4, 5, 7, 8, 9] {
            let data: Vec<u8> = (0..lists * dim)
                .map(|i| ((i * 73 + i / dim * 17) % 256) as u8)
                .collect();
            let indptr: Vec<u64> = (0..=lists as u64).collect();
            let ids = vec![0u32; lists];
            let idx = FilteredIndex::build(
                &data,
                dim,
                &indptr,
                &ids,
                1,
                &FilterBuildParams {
                    ivf_min: usize::MAX,
                    ..FilterBuildParams::default()
                },
            )
            .unwrap();
            let mut q = vec![0i8; idx.stride];
            for (j, v) in q[..dim].iter_mut().enumerate() {
                *v = if j % 2 == 0 { -128 } else { 127 };
            }
            let qsum: i32 = q.iter().map(|&v| i32::from(v)).sum();
            let qnorm: i64 = q.iter().map(|&v| i64::from(v).pow(2)).sum();
            let mut centroids = vec![0i8; lists * idx.stride];
            for c in 0..lists {
                for (j, v) in centroids[c * idx.stride..c * idx.stride + dim]
                    .iter_mut()
                    .enumerate()
                {
                    *v = if c % 3 == 0 {
                        q[j]
                    } else if c % 3 == 1 {
                        0
                    } else {
                        !q[j]
                    };
                }
            }
            let mut reference: Vec<(i64, usize)> = centroids
                .chunks_exact(idx.stride)
                .enumerate()
                .map(|(c, row)| {
                    let distance = q
                        .iter()
                        .zip(row)
                        .map(|(&a, &b)| (i64::from(a) - i64::from(b)).pow(2))
                        .sum();
                    (distance, c)
                })
                .collect();
            reference.sort_unstable();
            for copied in [false, true] {
                let members: Vec<u32> = (0..lists as u32).rev().collect();
                let ivf = LabelIvf {
                    centroid_norms: centroid_norms(&centroids, idx.stride),
                    centroids: centroids.clone(),
                    offsets: (0..=lists as u32).collect(),
                    rows: if copied {
                        members
                            .iter()
                            .flat_map(|&id| idx.row(id).iter().copied())
                            .collect()
                    } else {
                        Vec::new()
                    },
                    codes: Vec::new(),
                    bids: Vec::new(),
                    blocks: Vec::new(),
                    norms: if copied {
                        members.iter().map(|&id| idx.norms[id as usize]).collect()
                    } else {
                        Vec::new()
                    },
                    members,
                };
                for (target, min_lists) in [(1, 1), (3, 1), (1, 5), (usize::MAX, 1)] {
                    let count = target.max(min_lists).min(lists);
                    let mut expected: Vec<(i64, u32)> = reference[..count]
                        .iter()
                        .map(|&(_, c)| {
                            let id = ivf.members[c];
                            let distance: i64 = q
                                .iter()
                                .zip(idx.row(id))
                                .map(|(&a, &b)| (i64::from(a) - i64::from(b)).pow(2))
                                .sum();
                            (distance - qnorm, id)
                        })
                        .collect();
                    expected.sort_unstable();
                    expected.truncate(3);
                    let mut top = TopK {
                        k: 3,
                        items: Vec::new(),
                    };
                    let scored = idx.search_ivf(
                        &ivf,
                        (&q, qsum),
                        &[],
                        &FilterSearchParams {
                            target,
                            min_lists,
                            ..FilterSearchParams::default()
                        },
                        &mut top,
                        &mut IvfScratch::default(),
                    );
                    assert_eq!(scored, count);
                    assert_eq!(top.items, expected, "dim={dim}, lists={lists}, copied={copied}, target={target}, min_lists={min_lists}");
                }
            }
        }
    }
}

#[test]
fn streamed_exact_matches_brute_at_batch_boundaries() {
    for count in [0, 3, 4, 5, 127, 128, 129] {
        for dim in [16, 20] {
            for bitset_min in [1, usize::MAX] {
                let n = count + 2;
                let mut f = Fixture {
                    data: (0..n * dim).map(|i| ((i / dim) % 9) as u8).collect(),
                    indptr: vec![0],
                    ids: Vec::new(),
                    dim,
                };
                for id in 0..n {
                    f.ids.push(0);
                    if id < count {
                        f.ids.push(1);
                    }
                    f.indptr.push(f.ids.len() as u64);
                }
                let mut idx = FilteredIndex::build(
                    &f.data,
                    dim,
                    &f.indptr,
                    &f.ids,
                    3,
                    &FilterBuildParams {
                        ivf_min: usize::MAX,
                        bitset_min,
                        ..FilterBuildParams::default()
                    },
                )
                .unwrap();
                let mut deleted = vec![false; n];
                let q = vec![4; dim];
                let mut out = Vec::new();
                for delete_first in [false, true] {
                    if delete_first {
                        assert!(idx.delete(0));
                        deleted[0] = true;
                    }
                    for labels in [vec![1], vec![1, 1], vec![0, 1, 0], vec![2]] {
                        idx.reset_stats();
                        idx.search(
                            &q,
                            &labels,
                            7,
                            &FilterSearchParams {
                                brute_max: 0,
                                ..FilterSearchParams::default()
                            },
                            &mut out,
                        )
                        .unwrap();
                        assert_eq!(out, brute(&f, &deleted, &q, &labels, 7));
                        let expected_scored = (0..n)
                            .filter(|&id| !deleted[id])
                            .filter(|&id| {
                                let tags = &f.ids[f.indptr[id] as usize..f.indptr[id + 1] as usize];
                                labels.iter().all(|label| tags.contains(label))
                            })
                            .count();
                        assert_eq!(
                            idx.stats(),
                            FilterStats {
                                brute: 1,
                                ivf: 0,
                                scored: expected_scored as u64,
                            }
                        );
                    }
                    for labels in [vec![], vec![3]] {
                        idx.search(&q, &labels, 7, &FilterSearchParams::exact(), &mut out)
                            .unwrap();
                        assert!(out.is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn streamed_exact_keeps_intersection_routing_boundary() {
    let f = fixture(2_000, 20, 12, 701);
    let idx = FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &small_params()).unwrap();
    let q = &f.data[..f.dim];
    let labels = [0, 1];
    let mut order = labels;
    order.sort_by_key(|&label| (idx.label_len(label), label));
    let rarest = order[0];
    assert_ne!(idx.ivf_slot[rarest as usize], u32::MAX);
    let mut matches = Vec::new();
    idx.intersect(rarest, &order[1..], &mut matches, &mut Vec::new());
    assert!(!matches.is_empty());
    assert!(matches.len() < idx.label_len(rarest));
    let mut out = Vec::new();
    for brute_max in [matches.len() - 1, matches.len(), matches.len() + 1] {
        idx.reset_stats();
        idx.search(
            q,
            &labels,
            10,
            &FilterSearchParams {
                brute_max,
                target: usize::MAX,
                min_lists: 1,
                intersect_max: usize::MAX,
                rerank: 0,
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(out, brute(&f, &vec![false; 2_000], q, &labels, 10));
        assert_eq!(idx.stats().brute, u64::from(matches.len() <= brute_max));
        assert_eq!(idx.stats().ivf, u64::from(matches.len() > brute_max));
        assert_eq!(idx.stats().scored, matches.len() as u64);
    }
}
#[test]
fn combined_parallel_batch_matches_serial_results_and_work() {
    let f = fixture(4000, 17, 25, 4367);
    let mut idx =
        FilteredIndex::build(&f.data, f.dim, &f.indptr, &f.ids, 0, &small_params()).unwrap();
    for id in (0..4000).step_by(7) {
        idx.delete(id);
    }
    let qs = queries(&f, 256, 25, 7543);
    let mut flat = Vec::new();
    let mut offsets = vec![0];
    let mut labels = Vec::new();
    let params = FilterSearchParams {
        brute_max: 40,
        target: 128,
        min_lists: 1,
        intersect_max: 1000,
        rerank: 0,
    };
    let mut expected = Vec::new();
    let mut found = Vec::new();
    idx.reset_stats();
    for (q, ls) in &qs {
        flat.extend_from_slice(q);
        labels.extend_from_slice(ls);
        offsets.push(labels.len() as u64);
        idx.search(q, ls, 10, &params, &mut found).unwrap();
        for j in 0..10 {
            expected.push(found.get(j).map_or(-1, |&(_, id)| i64::from(id)));
        }
    }
    let work = idx.stats();
    idx.reset_stats();
    let mut actual = vec![-1; expected.len()];
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(|| {
            idx.search_batch(&flat, &offsets, &labels, 10, &params, &mut actual)
                .unwrap();
        });
    assert_eq!(actual, expected);
    assert_eq!(idx.stats(), work);
}
