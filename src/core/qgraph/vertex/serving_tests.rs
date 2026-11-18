// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn connected_search_fingerprints_and_evaluation_counts() {
    let rows: Vec<f32> = (0..64)
        .flat_map(|v| (0..16).map(move |d| if v & (1 << (d % 6)) == 0 { -1.0 } else { 1.0 }))
        .collect();
    for metric in [VMetric::Angular, VMetric::Euclidean, VMetric::InnerProduct] {
        let mut graph = if metric == VMetric::InnerProduct {
            lifted(&rows, 16)
        } else {
            Graph::build(
                &rows,
                16,
                &BuildParams {
                    metric: if metric == VMetric::Angular {
                        Metric::Angular
                    } else {
                        Metric::Euclidean
                    },
                    degree: 8,
                    build_ef: 16,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        graph.entry = 0;
        graph.layers.clear();
        graph.upper_ids.clear();
        for v in 0..graph.n {
            let neighbors: Vec<_> = [1, 3]
                .map(|offset| ((v + offset) % graph.n) as u32)
                .to_vec();
            graph.adj.set(v, &neighbors);
        }
        for bits in [4, 8] {
            let index = if metric == VMetric::InnerProduct {
                VGraph::from_lifted_ip_graph(&graph, bits, 8, true, true, 3).unwrap()
            } else {
                VGraph::from_graph_with(&graph, bits, 8, true, true, 3, false).unwrap()
            };
            for (knob_id, knobs) in [
                SearchKnobs::OFF,
                SearchKnobs {
                    prefetch: 3,
                    batch_est: true,
                    frontier: 0.8,
                    ..SearchKnobs::OFF
                },
            ]
            .into_iter()
            .enumerate()
            {
                let mut searcher = index.searcher_with(knobs);
                let mut out = Vec::new();
                let mut fingerprint = 0xcbf29ce484222325u64;
                let mut evaluations = 0usize;
                for ef in [8, 16] {
                    for reservoir in [0, 16] {
                        for patience in [0, 1] {
                            for query in rows.as_chunks::<16>().0.iter().step_by(7) {
                                let count = searcher
                                    .search(
                                        query,
                                        5,
                                        VSearchParams {
                                            ef,
                                            rerank: 16,
                                            reservoir,
                                            patience,
                                            ..Default::default()
                                        },
                                        &mut out,
                                    )
                                    .unwrap();
                                assert_eq!(out.len(), 5);
                                evaluations += count;
                                for value in
                                    out.iter()
                                        .map(|&id| u64::from(id))
                                        .chain(
                                            searcher.scratch.results.iter().map(
                                                |&(distance, _)| u64::from(distance.to_bits()),
                                            ),
                                        )
                                        .chain([count as u64])
                                {
                                    fingerprint = (fingerprint ^ value).wrapping_mul(0x100000001b3);
                                }
                            }
                        }
                    }
                }
                println!("CONNECTED {metric:?} {bits} {knob_id} {fingerprint} {evaluations}");
            }
        }
    }
}

fn disconnect(graph: &mut Graph, component_size: usize) {
    graph.entry = 0;
    graph.layers.clear();
    graph.upper_ids.clear();
    for v in 0..graph.n {
        let first = v / component_size * component_size;
        let neighbors: Vec<_> = (first..(first + component_size).min(graph.n))
            .filter(|&u| u != v)
            .map(|u| u as u32)
            .collect();
        graph.adj.set(v, &neighbors);
    }
}

fn assert_complete(searcher: &VSearcher<'_>, out: &[u32], k: usize, rerank: usize) {
    assert_eq!(out.len(), k.min(searcher.index.n));
    assert_eq!(
        out.iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        out.len()
    );
    assert!(out.iter().all(|&id| (id as usize) < searcher.index.n));
    assert!(searcher.scratch.keys.windows(2).all(|w| w[0] <= w[1]));
    if rerank > 0 {
        assert!(searcher.scratch.results.iter().all(|r| r.0.is_finite()));
        assert!(searcher
            .scratch
            .results
            .windows(2)
            .all(|w| { w[0].0.total_cmp(&w[1].0).then(w[0].1.cmp(&w[1].1)).is_le() }));
    }
}

#[test]
fn extreme_angular_queries_preserve_complete_order() {
    for dim in [15, 16, 17, 31, 32, 33] {
        let mut rows = vec![0.0; dim * dim];
        for row in 0..dim {
            rows[row * dim + row] = 1.0;
        }
        let graph = Graph::build(
            &rows,
            dim,
            &BuildParams {
                degree: 4,
                build_ef: 64,
                metric: Metric::Angular,
                ..Default::default()
            },
        )
        .unwrap();
        for bits in [4, 8] {
            let index = VGraph::from_graph_with(&graph, bits, 8, true, true, 3, false).unwrap();
            let mut searcher = index.searcher_with(SearchKnobs::OFF);
            let mut out = Vec::new();
            for scale in [1.0, 1e-30, 1e-40, f32::MAX] {
                let query: Vec<_> = (1..=dim).map(|i| (i as f32 / dim as f32) * scale).collect();
                searcher
                    .search(
                        &query,
                        dim,
                        VSearchParams {
                            ef: dim,
                            rerank: dim,
                            ..Default::default()
                        },
                        &mut out,
                    )
                    .unwrap();
                assert_eq!(out, (0..dim as u32).rev().collect::<Vec<_>>());
                assert_complete(&searcher, &out, dim, dim);
            }
        }
    }
}

#[test]
fn disconnected_graph_returns_the_requested_number_of_distinct_neighbors() {
    let rows: Vec<f32> = (0..67 * 2).map(|i| (i * 7 % 31 + 1) as f32).collect();
    for metric in [VMetric::Angular, VMetric::Euclidean, VMetric::InnerProduct] {
        let mut graph = if metric == VMetric::InnerProduct {
            lifted(&rows, 2)
        } else {
            Graph::build(
                &rows,
                2,
                &BuildParams {
                    metric: if metric == VMetric::Angular {
                        Metric::Angular
                    } else {
                        Metric::Euclidean
                    },
                    degree: 2,
                    build_ef: 16,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        disconnect(&mut graph, 3);
        for bits in [4, 8] {
            for id_bytes in [0, 3, 4] {
                let index = if metric == VMetric::InnerProduct {
                    VGraph::from_lifted_ip_graph(&graph, bits, 8, true, true, id_bytes).unwrap()
                } else {
                    VGraph::from_graph_with(
                        &graph,
                        bits,
                        8,
                        true,
                        true,
                        id_bytes,
                        metric == VMetric::Angular,
                    )
                    .unwrap()
                };
                for (ef, reservoir, rerank, patience, neighbour_cap) in [
                    (0, 0, 0, 0, 0),
                    (8, 0, 67, 1, 1),
                    (8, 1, 67, 1, 0),
                    (8, 68, 67, 1, 1),
                ] {
                    let knobs = SearchKnobs {
                        prefetch: 3,
                        batch_est: true,
                        frontier: 0.0,
                        rerank_z: 1.0,
                        ..SearchKnobs::OFF
                    };
                    let mut searcher = index.searcher_with(knobs);
                    let params = VSearchParams {
                        ef,
                        reservoir,
                        rerank,
                        patience,
                        neighbour_cap,
                        screen: reservoir == 0,
                        screen_sigmas: 0.0,
                        ..Default::default()
                    };
                    let mut out = Vec::new();
                    for k in [0, 1, 2, 3, 4, 63, 64, 65, 66, 67, 68, usize::MAX] {
                        let evaluations =
                            searcher.search(&[1.0, 2.0], k, params, &mut out).unwrap();
                        assert_complete(&searcher, &out, k, rerank);
                        let first = out.clone();
                        assert_eq!(
                            searcher.search(&[1.0, 2.0], k, params, &mut out).unwrap(),
                            evaluations
                        );
                        assert_eq!(out, first);
                    }
                }
            }
        }
    }
}

#[test]
fn recovery_expands_the_seed_and_counts_its_score() {
    let rows = [
        1.0, 0.0, 2.0, 0.0, 3.0, 0.0, -10.0, 0.0, 5.0, 0.0, 6.0, 0.0, 100.0, 0.0,
    ];
    let mut graph = lifted(&rows, 2);
    disconnect(&mut graph, 1);
    graph.adj.set(0, &[1]);
    graph.adj.set(1, &[2]);
    graph.adj.set(3, &[5]);
    graph.adj.set(5, &[4]);
    let index = VGraph::from_lifted_ip_graph(&graph, 8, 8, false, false, 4).unwrap();
    let mut searcher = index.searcher_with(SearchKnobs::OFF);
    for reservoir in [0, 4] {
        let params = VSearchParams {
            ef: 4,
            rerank: 4,
            reservoir,
            patience: 1,
            ..Default::default()
        };
        let mut out = Vec::new();
        let evaluations = searcher.search(&[1.0, 0.0], 4, params, &mut out).unwrap();
        assert_eq!(out, [5, 4, 2, 1]);
        assert_eq!(evaluations, 10);
        assert!(!searcher.scratch.visited.seen(6));
        #[cfg(feature = "experimental")]
        {
            let profile = searcher
                .profile_search(&[1.0, 0.0], 4, params, &mut out)
                .unwrap();
            assert_eq!(profile.traversal_evaluations, 6);
            assert_eq!(
                profile.entry_evaluations
                    + profile.traversal_evaluations
                    + profile.rerank_evaluations,
                evaluations
            );
        }
    }
    searcher
        .search(
            &[0.0, 0.0],
            7,
            VSearchParams {
                rerank: 7,
                ..Default::default()
            },
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(
        searcher
            .scratch
            .results
            .iter()
            .map(|r| r.1)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6]
    );
}

#[test]
fn disconnected_blocks_retain_enough_expansions_with_stopping() {
    let rows = crate::core::qgraph::tests::clustered(67, 16, 77);
    let mut graph = Graph::build(
        &rows,
        16,
        &BuildParams {
            degree: 32,
            build_ef: 64,
            ..Default::default()
        },
    )
    .unwrap();
    for component_size in [3, 32, 33] {
        disconnect(&mut graph, component_size);
        for bits in [1, 2, 4] {
            for id_bytes in [0, 3, 4] {
                let index = VGraph::from_graph_with(&graph, 4, 8, true, true, id_bytes, false)
                    .unwrap()
                    .with_blocks(&graph, bits)
                    .unwrap();
                for (fuse, block_screen) in [(false, false), (true, false), (false, true)] {
                    let mut searcher = index.searcher_with(SearchKnobs {
                        frontier: 0.0,
                        block_screen,
                        ..SearchKnobs::OFF
                    });
                    for rerank in [0, 67] {
                        let params = VSearchParams {
                            ef: 1,
                            rerank,
                            patience: 1,
                            fuse,
                            screen_sigmas: 0.0,
                            ..Default::default()
                        };
                        let mut out = Vec::new();
                        for k in [
                            0,
                            component_size - 1,
                            component_size,
                            component_size + 1,
                            66,
                            67,
                            68,
                        ] {
                            searcher.search(&rows[..16], k, params, &mut out).unwrap();
                            assert_complete(&searcher, &out, k, rerank);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn recovery_reuses_marks_and_fused_traces_across_epoch_rollover() {
    let rows = crate::core::qgraph::tests::clustered(67, 16, 79);
    let mut graph = Graph::build(
        &rows,
        16,
        &BuildParams {
            degree: 2,
            build_ef: 16,
            ..Default::default()
        },
    )
    .unwrap();
    disconnect(&mut graph, 3);
    let index = VGraph::from_graph(&graph, true)
        .unwrap()
        .with_blocks(&graph, 2)
        .unwrap();
    let mut searcher = index.searcher_with(SearchKnobs::OFF);
    let mut out = Vec::new();
    for iteration in 0..260 {
        for fuse in [false, true] {
            let k = if iteration % 2 == 0 { 4 } else { 65 };
            let query = &rows[(iteration % 67) * 16..][..16];
            let params = VSearchParams {
                ef: 1,
                rerank: 67,
                patience: 1,
                fuse,
                ..Default::default()
            };
            let mut fresh = index.searcher_with(SearchKnobs::OFF);
            let mut expected = Vec::new();
            let evaluations = fresh.search(query, k, params, &mut expected).unwrap();
            assert_eq!(
                searcher.search(query, k, params, &mut out).unwrap(),
                evaluations
            );
            assert_eq!(out, expected);
            assert_complete(&searcher, &out, k, params.rerank);
        }
    }
}

#[test]
fn singleton_search_bounds_k_without_recovery() {
    let graph = lifted(&[2.0, 0.0], 2);
    let index = VGraph::from_lifted_ip_graph(&graph, 8, 8, false, false, 4).unwrap();
    let mut searcher = index.searcher_with(SearchKnobs::OFF);
    for k in [0, 1, 2, usize::MAX] {
        let mut out = Vec::new();
        searcher
            .search(
                &[1.0, 0.0],
                k,
                VSearchParams {
                    ef: 0,
                    ..Default::default()
                },
                &mut out,
            )
            .unwrap();
        assert_complete(&searcher, &out, k, 0);
    }
}

fn lifted(rows: &[f32], dim: usize) -> Graph {
    let norms: Vec<f64> = rows
        .chunks_exact(dim)
        .map(|row| row.iter().map(|&x| f64::from(x).powi(2)).sum())
        .collect();
    let max = norms.iter().copied().fold(0.0, f64::max);
    let data: Vec<f32> = rows
        .chunks_exact(dim)
        .zip(norms)
        .flat_map(|(row, norm)| {
            row.iter()
                .copied()
                .chain([(max - norm).max(0.0).sqrt() as f32])
        })
        .collect();
    Graph::build_owned(
        data,
        dim + 1,
        &BuildParams {
            metric: Metric::Euclidean,
            degree: 2,
            build_ef: 16,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn two_hop_search_finds_neighbor_behind_a_rejected_vertex() {
    let mut graph = lifted(&[0.0, 0.0, 0.8, 0.0, 0.2, 0.0, 1.0, 0.0], 2);
    graph.entry = 0;
    graph.layers.clear();
    graph.upper_ids.clear();
    graph.adj.set(0, &[1, 2]);
    graph.adj.set(1, &[]);
    graph.adj.set(2, &[3]);
    graph.adj.set(3, &[]);
    let index = VGraph::from_lifted_ip_graph(&graph, 8, 8, false, false, 4).unwrap();
    let mut searcher = index.searcher_with(SearchKnobs::OFF);
    let mut out = Vec::new();
    for ef in [1, 2, 3] {
        let params = VSearchParams {
            ef,
            rerank: ef,
            ..Default::default()
        };
        searcher.search(&[1.0, 0.0], 1, params, &mut out).unwrap();
        assert_eq!(out, if ef == 1 { vec![1] } else { vec![3] });
        searcher
            .search_two_hop(&[1.0, 0.0], 1, params, &mut out)
            .unwrap();
        assert_eq!(out, [3]);
    }
    let capped = VSearchParams {
        neighbour_cap: 1,
        ..Default::default()
    };
    assert!(searcher
        .search_two_hop(&[1.0, 0.0], 1, capped, &mut out)
        .is_err());
}

#[test]
fn two_hop_search_handles_cycles_and_repeated_queries() {
    let mut graph = lifted(&[0.0, 0.0, 0.6, 0.0, 0.3, 0.0, 0.8, 0.0, 1.0, 0.0], 2);
    graph.entry = 0;
    graph.layers.clear();
    graph.upper_ids.clear();
    graph.adj.set(0, &[1, 2]);
    graph.adj.set(1, &[2, 3]);
    graph.adj.set(2, &[1, 3]);
    graph.adj.set(3, &[1, 4]);
    graph.adj.set(4, &[]);
    let index = VGraph::from_lifted_ip_graph(&graph, 8, 8, false, false, 4).unwrap();
    let mut searcher = index.searcher_with(SearchKnobs::OFF);
    let params = VSearchParams {
        ef: 1,
        rerank: 1,
        ..Default::default()
    };
    let mut out = Vec::new();
    for _ in 0..3 {
        searcher
            .search_two_hop(&[1.0, 0.0], 1, params, &mut out)
            .unwrap();
        assert_eq!(out, [4]);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cached_two_hop_search_matches_fresh_search_across_indexes() {
    let build = |rows, seed| {
        let data = crate::core::qgraph::tests::clustered(rows, 16, seed);
        let graph = Graph::build(
            &data,
            16,
            &BuildParams {
                degree: 8,
                build_ef: 32,
                ..Default::default()
            },
        )
        .unwrap();
        VGraph::from_graph(&graph, true).unwrap()
    };
    let (first, larger, different) = (build(65, 1), build(127, 2), build(65, 3));
    let queries = crate::core::qgraph::tests::clustered(3, 16, 4);
    for index in [&first, &larger, &different, &first] {
        for query in queries.as_chunks::<16>().0 {
            for k in [1, 4, 17] {
                let params = VSearchParams {
                    ef: 32,
                    rerank: 32,
                    ..Default::default()
                };
                let (mut expected, mut actual) = (Vec::new(), Vec::new());
                let evaluations = index
                    .searcher()
                    .search_two_hop(query, k, params, &mut expected)
                    .unwrap();
                assert_eq!(
                    index
                        .search_two_hop_cached(query, k, params, &mut actual)
                        .unwrap(),
                    evaluations
                );
                assert_eq!(actual, expected);
                SNAPSHOT_SCRATCH.with(|slot| {
                    let _held = slot.borrow_mut();
                    index
                        .search_two_hop_cached(query, k, params, &mut actual)
                        .unwrap();
                    assert_eq!(actual, expected);
                });
            }
        }
    }
    assert!(first
        .search_two_hop_cached(&[0.0; 16], 1, VSearchParams::default(), &mut Vec::new())
        .is_err());
}

#[test]
fn reservoir_recovers_a_beam_rejection_without_changing_expansions() {
    let mut graph = lifted(&[0.0, 0.0, 0.8, 0.0, 1.0, 381.0], 2);
    graph.entry = 0;
    graph.layers.clear();
    graph.upper_ids.clear();
    graph.adj.set(0, &[1, 2]);
    graph.adj.set(1, &[]);
    graph.adj.set(2, &[]);
    let index = VGraph::from_lifted_ip_graph(&graph, 8, 8, false, false, 4).unwrap();
    let mut searcher = index.searcher_with(SearchKnobs::OFF);
    let mut out = Vec::new();
    let params = VSearchParams {
        ef: 1,
        rerank: 3,
        ..Default::default()
    };
    searcher.search(&[1.0, 0.0], 1, params, &mut out).unwrap();
    assert_eq!(out, [1]);
    let beam = searcher.scratch.pool.keys.clone();
    let marks = searcher.scratch.visited.words.clone();
    for capacity in [1, 2, 3, usize::MAX] {
        let params = VSearchParams {
            reservoir: capacity,
            ..params
        };
        searcher.search(&[1.0, 0.0], 1, params, &mut out).unwrap();
        assert_eq!(searcher.scratch.pool.keys, beam);
        assert_eq!(searcher.scratch.visited.words, marks);
        assert_eq!(out, if capacity == 1 { vec![1] } else { vec![2] });
        assert_eq!(searcher.scratch.results.len(), capacity.min(3));
        #[cfg(feature = "experimental")]
        {
            let profile = searcher
                .profile_search(&[1.0, 0.0], 1, params, &mut out)
                .unwrap();
            assert_eq!(profile.expansions, 2);
            assert_eq!(profile.rerank_evaluations, capacity.min(3));
            assert_eq!(profile.beam_retained, 1);
            assert_eq!(profile.retained, capacity.min(3));
        }
    }
}

#[test]
fn retention_preserves_traversal_with_stopping_and_batch_scoring() {
    let rows = crate::core::qgraph::tests::clustered(256, 32, 77);
    for metric in [Metric::Angular, Metric::Euclidean] {
        let graph = Graph::build(
            &rows,
            32,
            &BuildParams {
                metric,
                degree: 8,
                build_ef: 32,
                ..Default::default()
            },
        )
        .unwrap();
        let index = VGraph::from_graph_with(&graph, 4, 8, true, true, 3, false).unwrap();
        for patience in [0, 1, 4] {
            for knobs in [
                SearchKnobs::OFF,
                SearchKnobs {
                    prefetch: 3,
                    batch_est: true,
                    frontier: 0.8,
                    block_screen: false,
                    rerank_z: 0.0,
                },
            ] {
                let mut a = index.searcher_with(knobs);
                let mut b = index.searcher_with(knobs);
                for q in rows.as_chunks::<32>().0.iter().take(8) {
                    let mut out = Vec::new();
                    let p = VSearchParams {
                        ef: 12,
                        rerank: 40,
                        patience,
                        ..Default::default()
                    };
                    let ea = a.search(q, 5, p, &mut out).unwrap();
                    let eb = b
                        .search(q, 5, VSearchParams { reservoir: 40, ..p }, &mut out)
                        .unwrap();
                    assert_eq!(a.scratch.pool.keys, b.scratch.pool.keys);
                    assert_eq!(a.scratch.pool.expanded, b.scratch.pool.expanded);
                    assert_eq!(a.scratch.visited.words, b.scratch.visited.words);
                    assert_eq!(ea - a.scratch.results.len(), eb - b.scratch.results.len());
                }
            }
        }
    }
}

#[test]
fn reservoir_includes_upper_layer_scores_once() {
    let mut graph = lifted(&[0.0, 0.0, 0.8, 0.0, 1.0, 381.0], 2);
    graph.upper_ids = vec![0, 1, 2];
    graph.layers = vec![vec![vec![1, 2], vec![0, 2], vec![0, 1]]];
    for v in 0..3 {
        graph.adj.set(v, &[]);
    }
    let index = VGraph::from_lifted_ip_graph(&graph, 8, 8, false, false, 4).unwrap();
    let mut searcher = index.searcher_with(SearchKnobs::OFF);
    let mut out = Vec::new();
    for _ in 0..260 {
        searcher
            .search(
                &[1.0, 0.0],
                1,
                VSearchParams {
                    ef: 1,
                    rerank: 3,
                    reservoir: 3,
                    ..Default::default()
                },
                &mut out,
            )
            .unwrap();
        assert_eq!(out, [2]);
        assert_eq!(searcher.scratch.keys.len(), 3);
    }
}

#[test]
fn direct_ip_preserves_magnitudes_zero_queries_and_topology() {
    let rows = [1.0, 0.0, 0.0, 3.0, 0.0, 3.0, -2.0, 0.0];
    let mut graph = lifted(&rows, 2);
    for v in 0..graph.n {
        graph.adj.set(v, &[((v + 1) % graph.n) as u32]);
    }
    for bits in [4, 8] {
        for reorder in [false, true] {
            let index = VGraph::from_lifted_ip_graph(&graph, bits, 8, true, reorder, 3).unwrap();
            assert_eq!(index.metric(), VMetric::InnerProduct);
            assert!(index.half_norms.is_empty());
            for v in 0..index.n {
                let mut neighbors = Vec::new();
                index.for_each_neighbour(v, |u| neighbors.push(index.original(u)));
                assert_eq!(
                    neighbors,
                    graph.neighbors(index.original(v as u32) as usize)
                );
            }
            let mut searcher = index.searcher_with(SearchKnobs::OFF);
            let mut out = Vec::new();
            let params = VSearchParams {
                ef: 4,
                rerank: 4,
                reservoir: 4,
                ..Default::default()
            };
            for (query, expected) in [
                ([1.0, 1.0], vec![1, 2, 0, 3]),
                ([0.0, 0.0], vec![0, 1, 2, 3]),
                ([-1.0, 0.0], vec![3, 1, 2, 0]),
            ] {
                searcher.search(&query, 4, params, &mut out).unwrap();
                assert_eq!(out, expected);
            }
            assert!(searcher
                .search(&[f32::NAN, 0.0], 1, params, &mut out)
                .is_err());
            assert!(searcher.search(&[0.0], 1, params, &mut out).is_err());
            assert!(index.with_blocks(&graph, 2).is_err());
        }
    }
}

#[test]
fn direct_ip_removes_the_128_dimension_stride_cliff() {
    let mut rows = vec![0.0; 3 * 128];
    rows[0] = 1.0;
    rows[128] = 2.0;
    rows[256] = -1.0;
    let graph = lifted(&rows, 128);
    let old = VGraph::from_graph_with(&graph, 8, 8, true, true, 3, false).unwrap();
    let new = VGraph::from_lifted_ip_graph(&graph, 8, 8, true, true, 3).unwrap();
    assert_eq!(
        (old.dim, old.padded, old.stride, old.res_stride),
        (129, 144, 256, 144)
    );
    assert_eq!(
        (new.dim, new.padded, new.stride, new.res_stride),
        (128, 128, 128, 128)
    );
    assert_eq!(
        old.index_bytes() - new.index_bytes(),
        148 * graph.n + 16 * 4
    );
    assert_eq!(old.entry, new.entry);
    assert_eq!(old.upper_ids, new.upper_ids);
    assert_eq!(old.layers, new.layers);
    assert_eq!(&*old.ids, &*new.ids);
    assert_eq!(&*old.adj3, &*new.adj3);
    for dim in [127, 129] {
        let mut rows = vec![0.0; 3 * dim];
        rows[0] = 1.0;
        rows[dim] = 2.0;
        rows[2 * dim] = -1.0;
        let graph = lifted(&rows, dim);
        let old = VGraph::from_graph_with(&graph, 8, 8, true, true, 3, false).unwrap();
        let new = VGraph::from_lifted_ip_graph(&graph, 8, 8, true, true, 3).unwrap();
        assert_eq!((new.stride, new.res_stride), (old.stride, old.res_stride));
        assert_eq!(old.index_bytes() - new.index_bytes(), 4 * graph.n);
    }
}
