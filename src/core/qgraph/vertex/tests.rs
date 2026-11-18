// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::super::kernels::SplitMix;
use super::super::tests::{brute_force, clustered, gaussian};
use super::*;

#[test]
fn query_x4_estimates_match_serial_at_padding_boundaries() {
    for dim in [
        15, 16, 17, 31, 32, 33, 63, 64, 65, 111, 112, 113, 127, 128, 129,
    ] {
        let data = clustered(40, dim, 173);
        let queries = clustered(2, dim, 174);
        for metric in [Metric::Angular, Metric::Euclidean] {
            let graph = Graph::build(
                &data,
                dim,
                &BuildParams {
                    metric,
                    degree: 8,
                    build_ef: 16,
                    quantized_search: true,
                    ..Default::default()
                },
            )
            .unwrap();
            for bits in [4, 8] {
                for align in [false, true] {
                    let index =
                        VGraph::from_graph_with(&graph, bits, 8, align, true, 3, false).unwrap();
                    let mut searcher = index.searcher_with(SearchKnobs::OFF);
                    let scratch = &mut searcher.scratch;
                    for query in queries.chunks_exact(dim) {
                        scratch.qf[..dim].copy_from_slice(query);
                        index.encode_query(
                            &mut scratch.qf,
                            &mut scratch.qc,
                            &mut scratch.qtmp,
                            &mut scratch.qrot,
                            &mut scratch.qs,
                        );
                        let qsum = query_sum(&scratch.qc);
                        for ids in [[0, 1, 2, 3], [39, 16, 31, 0], [5, 5, 5, 5]] {
                            for w in [0.0, 1.0, 3.25] {
                                let expected =
                                    ids.map(|u| index.est(&scratch.qc, qsum, u, w).to_bits());
                                let actual = index
                                    .est4::<true>(&scratch.qc, qsum, ids, w)
                                    .map(f32::to_bits);
                                assert_eq!(actual, expected, "dim {dim}, bits {bits}");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn query_x4_preserves_admission_state_at_batch_boundaries() {
    let dim = 17;
    let data = clustered(48, dim, 175);
    let query = clustered(1, dim, 176);
    for metric in [Metric::Angular, Metric::Euclidean] {
        let graph = Graph::build(
            &data,
            dim,
            &BuildParams {
                metric,
                degree: 8,
                build_ef: 16,
                quantized_search: true,
                ..Default::default()
            },
        )
        .unwrap();
        for bits in [4, 8] {
            let index = VGraph::from_graph_with(&graph, bits, 8, true, true, 3, false).unwrap();
            for count in (0..=9).chain(15..=17) {
                for cap in [3, 4, 5, 8] {
                    for batch_est in [false, true] {
                        for patience in [0, 4] {
                            for reservoir in [0, 12] {
                                let knobs = SearchKnobs {
                                    batch_est,
                                    ..SearchKnobs::OFF
                                };
                                let mut serial = index.searcher_with(knobs);
                                let mut fused = index.searcher_with(knobs);
                                for s in [&mut serial.scratch, &mut fused.scratch] {
                                    s.qf[..dim].copy_from_slice(&query);
                                    index.encode_query(
                                        &mut s.qf,
                                        &mut s.qc,
                                        &mut s.qtmp,
                                        &mut s.qrot,
                                        &mut s.qs,
                                    );
                                    s.qsum = query_sum(&s.qc);
                                    s.pool.reset(cap);
                                    s.reservoir.reset(reservoir);
                                    for id in 32..48 {
                                        let kk = key(index.est(&s.qc, s.qsum, id, 1.0), id);
                                        s.pool.insert(kk);
                                        s.reservoir.offer(kk);
                                    }
                                    s.pool.next();
                                    s.fresh.extend(0..count);
                                }
                                let (mut a, mut b) = (patience, patience);
                                let expected = serial.scratch.score_fresh::<{
                                    cfg!(feature = "experimental-batch-pool")
                                }, false>(
                                    &index,
                                    cap.min(3),
                                    patience,
                                    &mut a,
                                    1.0,
                                );
                                let actual = fused.scratch.score_fresh::<{
                                    cfg!(feature = "experimental-batch-pool")
                                }, true>(
                                    &index,
                                    cap.min(3),
                                    patience,
                                    &mut b,
                                    1.0,
                                );
                                assert_eq!(actual, count as usize);
                                assert_eq!(actual, expected);
                                assert_eq!(a, b);
                                assert_eq!(serial.scratch.pool.keys, fused.scratch.pool.keys);
                                assert_eq!(
                                    serial.scratch.pool.expanded,
                                    fused.scratch.pool.expanded
                                );
                                assert_eq!(serial.scratch.pool.cursor, fused.scratch.pool.cursor);
                                assert_eq!(
                                    serial.scratch.reservoir.keys,
                                    fused.scratch.reservoir.keys
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn query_x4_preserves_search_results_work_and_reopening() {
    let directory = tempfile::tempdir().unwrap();
    for dim in [17, 33] {
        let mut data = clustered(80, dim, 177);
        let first = data[..dim].to_vec();
        data[7 * dim..8 * dim].copy_from_slice(&first);
        data[19 * dim..20 * dim].copy_from_slice(&first);
        let queries = clustered(2, dim, 178);
        for metric in [Metric::Angular, Metric::Euclidean] {
            let graph = Graph::build(
                &data,
                dim,
                &BuildParams {
                    metric,
                    degree: 16,
                    build_ef: 32,
                    quantized_search: true,
                    ..Default::default()
                },
            )
            .unwrap();
            for bits in [4, 8] {
                for mode in 0..3 {
                    if mode > 0 && metric != Metric::Angular {
                        continue;
                    }
                    let mut index =
                        VGraph::from_graph_with(&graph, bits, 8, true, true, 3, mode == 1).unwrap();
                    if mode == 2 {
                        index = index.with_blocks(&graph, 4).unwrap();
                    }
                    let path = directory.path().join("query-x4.warm");
                    index.save(&path).unwrap();
                    let reopened = VGraph::open(&path, Residency::Mapped).unwrap();
                    let knobs = SearchKnobs {
                        block_screen: mode == 2,
                        ..SearchKnobs::OFF
                    };
                    let mut serial = index.searcher_with(knobs);
                    let mut fused = reopened.searcher_with(knobs);
                    for (ef, neighbour_cap, patience, reservoir) in
                        [(3, 0, 0, 0), (4, 3, 1, 0), (5, 4, 4, 0), (17, 5, 0, 24)]
                    {
                        if mode > 0 && reservoir > 0 {
                            continue;
                        }
                        let params = VSearchParams {
                            ef,
                            rerank: 24,
                            neighbour_cap,
                            patience,
                            reservoir,
                            ..Default::default()
                        };
                        for k in [0, 1, 5] {
                            for query in queries.chunks_exact(dim) {
                                let (mut expected, mut actual) = (Vec::new(), Vec::new());
                                let (mut a, mut b) =
                                    (QueryProfile::default(), QueryProfile::default());
                                let ea = serial
                                    .scratch
                                    .search_inner::<
                                        true,
                                        { cfg!(feature = "experimental-batch-pool") },
                                        false,
                                        false,
                                    >(&index, query, k, params, &mut expected, &mut a)
                                    .unwrap();
                                let eb = fused
                                    .scratch
                                    .search_inner::<
                                        true,
                                        { cfg!(feature = "experimental-batch-pool") },
                                        true,
                                        false,
                                    >(&reopened, query, k, params, &mut actual, &mut b)
                                    .unwrap();
                                assert_eq!(expected, actual);
                                assert_eq!(ea, eb);
                                let work = |p: QueryProfile| {
                                    (
                                        p.expansions,
                                        p.neighbors_read,
                                        p.entry_evaluations,
                                        p.traversal_evaluations,
                                        p.rerank_evaluations,
                                        p.beam_retained,
                                        p.retained,
                                    )
                                };
                                assert_eq!(work(a), work(b));
                                assert_eq!(serial.scratch.pool.keys, fused.scratch.pool.keys);
                                assert_eq!(
                                    serial.scratch.pool.expanded,
                                    fused.scratch.pool.expanded
                                );
                                assert_eq!(serial.scratch.pool.cursor, fused.scratch.pool.cursor);
                                assert_eq!(
                                    serial.scratch.visited.words,
                                    fused.scratch.visited.words
                                );
                                assert_eq!(
                                    serial.scratch.reservoir.keys,
                                    fused.scratch.reservoir.keys
                                );
                                assert_eq!(serial.last_screened(), fused.last_screened());
                                let mut ordinary = Vec::new();
                                assert_eq!(
                                    fused.search(query, k, params, &mut ordinary).unwrap(),
                                    eb
                                );
                                assert_eq!(ordinary, actual);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A refused huge-page advice falls back to base pages with identical arrays and results,
/// and is counted. Runs the build twice in one process: advice failing, then as configured.
#[test]
fn refused_huge_page_advice_changes_nothing_but_the_counter() {
    // Large enough that the code rows span a 2 MiB-aligned interior.
    let (n, dim) = (40_000, 128);
    let data = clustered(n, dim, 3);
    let graph = Graph::build(
        &data,
        dim,
        &BuildParams {
            degree: 8,
            build_ef: 16,
            metric: Metric::Euclidean,
            ..BuildParams::default()
        },
    )
    .unwrap();
    let build = || VGraph::from_graph_with(&graph, 8, 8, true, true, 3, false).unwrap();
    FAIL_HUGE_ADVICE.store(true, Ordering::Relaxed);
    let failed_before = huge_page_status().failed_calls;
    let refused = build();
    FAIL_HUGE_ADVICE.store(false, Ordering::Relaxed);
    let advised = build();
    if cfg!(target_os = "linux") && huge_pages() {
        assert!(huge_page_status().failed_calls > failed_before);
    }
    assert_eq!(refused.codes.bytes(), advised.codes.bytes());
    assert_eq!(&refused.scales[..], &advised.scales[..]);
    assert_eq!(&refused.half_norms[..], &advised.half_norms[..]);
    let params = VSearchParams {
        ef: 40,
        rerank: 40,
        ..VSearchParams::default()
    };
    let (mut a, mut b) = (refused.searcher(), advised.searcher());
    let (mut ra, mut rb) = (Vec::new(), Vec::new());
    for q in data.chunks_exact(dim).step_by(997) {
        a.search(q, 10, params, &mut ra).unwrap();
        b.search(q, 10, params, &mut rb).unwrap();
        assert_eq!(ra, rb);
    }
}

#[test]
fn visited_bits_reset_between_searches() {
    // Ids at word edges and the last vertex of a count that is not a multiple of 64.
    let n = 64 * 5 + 3;
    let mut marks = Marks::new(n, false);
    let mut rng = SplitMix(5);
    for search in 0..300 {
        marks.next();
        let mut set = std::collections::HashSet::new();
        let fixed = [0, 63, 64, 127, n as u32 - 1];
        for i in 0..40 {
            let id = if i < fixed.len() && search % 2 == 0 {
                fixed[i]
            } else {
                (rng.next_u64() % n as u64) as u32
            };
            assert_eq!(marks.seen(id), set.contains(&id));
            assert_eq!(marks.insert(id), set.insert(id));
            assert!(marks.seen(id));
        }
        let ones: u32 = marks.words.iter().map(|w| w.count_ones()).sum();
        assert_eq!(ones as usize, set.len());
    }
}

#[test]
#[cfg(feature = "experimental-neighbor-admission")]
fn neighbor_admission_matches_ordered_set_model_at_boundaries() {
    fn check<const PROFILE: bool, const SCREEN: bool>(
        index: &VGraph,
        vertex: usize,
        list: &[u32],
        cap: usize,
    ) {
        let mut visited = Marks::new(index.n, false);
        for initial in [&[][..], &[0, 63, 64, 127, 128, 130][..], list] {
            visited.next();
            let mut seen = std::collections::HashSet::new();
            for &u in initial {
                visited.insert(u);
                seen.insert(u);
            }
            let limit = if cap == 0 { list.len() } else { cap };
            let expected: Vec<_> = list
                .iter()
                .take(limit)
                .copied()
                .filter(|&u| seen.insert(u))
                .collect();
            let mut fresh = Vec::new();
            let read =
                index.admit_neighbours::<PROFILE, SCREEN>(vertex, cap, &mut visited, &mut fresh);
            assert_eq!(fresh, expected, "cap={cap}, vertex={vertex}");
            // The existing profile counts all uncapped list entries, including duplicates.
            assert_eq!(read, if PROFILE && cap == 0 { list.len() } else { 0 });
            for u in 0..index.n as u32 {
                assert_eq!(visited.seen(u), seen.contains(&u));
            }
            let mut words = std::collections::HashSet::new();
            let touched: Vec<_> = initial
                .iter()
                .chain(&expected)
                .map(|&u| u >> 6)
                .filter(|&word| words.insert(word))
                .collect();
            assert_eq!(visited.touched, touched);
            visited.next();
            assert!(visited.words.iter().all(|&word| word == 0));
            assert!(visited.touched.is_empty());
        }
    }

    let dim = 17;
    let data = clustered(131, dim, 91);
    let lists = [
        Vec::new(),
        vec![130],
        vec![63, 64, 65, 127, 128, 129, 130, 63, 64, 0, 0],
        (0..63).rev().collect(),
        (0..64).rev().collect(),
        (0..65).rev().collect(),
    ];
    for metric in [Metric::Angular, Metric::Euclidean] {
        let mut graph = Graph::build(
            &data,
            dim,
            &BuildParams {
                metric,
                degree: 66,
                build_ef: 80,
                ..BuildParams::default()
            },
        )
        .unwrap();
        for bytes in [3, 4, 0] {
            let ordered: Vec<Vec<u32>> = lists
                .iter()
                .map(|list| {
                    let mut list = list.clone();
                    if bytes == 0 {
                        list.sort_unstable();
                        list.dedup();
                    }
                    list
                })
                .collect();
            for v in 0..graph.n {
                graph.adj.set(v, &ordered[v % ordered.len()]);
            }
            let mut index = VGraph::from_graph_with(
                &graph,
                8,
                8,
                true,
                false,
                bytes,
                metric == Metric::Angular,
            )
            .unwrap();
            for compact in [false, true] {
                if compact {
                    index.compact_adjacency().unwrap();
                    assert!(bytes == 0 || !index.adj_off.is_empty());
                }
                for (vertex, list) in ordered.iter().enumerate() {
                    for cap in [0, 1, 2, 63, 64, 65, 66, usize::MAX] {
                        check::<false, false>(&index, vertex, list, cap);
                        check::<true, false>(&index, vertex, list, cap);
                        if metric == Metric::Angular {
                            check::<false, true>(&index, vertex, list, cap);
                            check::<true, true>(&index, vertex, list, cap);
                        }
                    }
                }
            }
        }
    }
}

/// The pool before its flags became bits: one `bool` per key, moved with the keys.
struct ReferencePool {
    keys: Vec<u64>,
    expanded: Vec<bool>,
    cap: usize,
    cursor: usize,
}

impl ReferencePool {
    fn insert(&mut self, key: u64) -> Option<(usize, Option<u64>)> {
        let n = self.keys.len();
        if n >= self.cap && key >= self.keys[n - 1] {
            return None;
        }
        let pos = self.keys.partition_point(|&t| t < key);
        let mut evicted = None;
        if n >= self.cap {
            evicted = self.keys.pop();
            self.expanded.pop();
        }
        self.keys.insert(pos, key);
        self.expanded.insert(pos, false);
        if pos < self.cursor {
            self.cursor = pos;
        }
        Some((pos, evicted))
    }

    fn remove(&mut self, key: u64) {
        let pos = self.keys.partition_point(|&t| t < key);
        if pos < self.keys.len() && self.keys[pos] == key {
            self.keys.remove(pos);
            self.expanded.remove(pos);
            if pos < self.cursor {
                self.cursor -= 1;
            }
        }
    }

    fn next(&mut self) -> Option<u64> {
        while self.cursor < self.keys.len() {
            if !self.expanded[self.cursor] {
                self.expanded[self.cursor] = true;
                return Some(self.keys[self.cursor]);
            }
            self.cursor += 1;
        }
        None
    }

    fn peek_unexpanded(&self, n: usize) -> Option<u64> {
        (self.cursor..self.keys.len())
            .filter(|&i| !self.expanded[i])
            .nth(n)
            .map(|i| self.keys[i])
    }
}

#[test]
fn pool_bits_follow_the_bool_pool() {
    let mut rng = SplitMix(9);
    let mut pool = Pool {
        keys: Vec::new(),
        expanded: Vec::new(),
        #[cfg(feature = "experimental-batch-pool")]
        merge_keys: Vec::new(),
        #[cfg(feature = "experimental-batch-pool")]
        merge_expanded: Vec::new(),
        cap: 1,
        cursor: 0,
    };
    // Capacities on both sides of each word boundary, and one wider than any list here.
    for cap in [1usize, 2, 10, 63, 64, 65, 127, 128, 129, 300, 10_000] {
        for _ in 0..20 {
            pool.reset(cap);
            let mut model = ReferencePool {
                keys: Vec::new(),
                expanded: Vec::new(),
                cap,
                cursor: 0,
            };
            for _ in 0..600 {
                match rng.next_u64() % 8 {
                    0 | 1 => assert_eq!(pool.next(), model.next()),
                    2 if !model.keys.is_empty() => {
                        let k = model.keys[(rng.next_u64() % model.keys.len() as u64) as usize];
                        pool.remove(k);
                        model.remove(k);
                    }
                    3 => {
                        let n = (rng.next_u64() % 3) as usize;
                        assert_eq!(pool.peek_unexpanded(n), model.peek_unexpanded(n));
                    }
                    _ => {
                        let k = rng.next_u64() % 5000;
                        assert_eq!(pool.insert(k), model.insert(k));
                    }
                }
                assert_eq!(pool.keys, model.keys);
                assert_eq!(pool.cursor, model.cursor);
                let flags: Vec<bool> = (0..pool.keys.len()).map(|i| pool.is_expanded(i)).collect();
                assert_eq!(flags, model.expanded);
                let set: u32 = pool.expanded.iter().map(|w| w.count_ones()).sum();
                assert_eq!(set as usize, flags.iter().filter(|&&f| f).count());
            }
        }
    }
}

#[test]
#[cfg(feature = "experimental-batch-pool")]
fn batch_pool_matches_sequential_insertion() {
    fn check(pool: &Pool, model: &ReferencePool) {
        assert_eq!(pool.keys, model.keys);
        assert_eq!(pool.cursor, model.cursor);
        let flags: Vec<_> = (0..pool.keys.len()).map(|i| pool.is_expanded(i)).collect();
        assert_eq!(flags, model.expanded);
        assert_eq!(
            pool.expanded
                .iter()
                .map(|word| word.count_ones())
                .sum::<u32>() as usize,
            flags.iter().filter(|&&flag| flag).count()
        );
        for next in [0, 1, 2] {
            assert_eq!(pool.peek_unexpanded(next), model.peek_unexpanded(next));
        }
    }

    let mut rng = SplitMix(539365412);
    for cap in [1usize, 2, 7, 8, 9, 63, 64, 65, 127, 128, 129] {
        for k in [0, 1, cap - 1, cap, cap + 1] {
            for count in [0, 1, 7, 8, 9, cap + 1] {
                for fill in [0, cap / 2, cap] {
                    let mut pool = Pool {
                        keys: Vec::new(),
                        expanded: Vec::new(),
                        #[cfg(feature = "experimental-batch-pool")]
                        merge_keys: Vec::new(),
                        #[cfg(feature = "experimental-batch-pool")]
                        merge_expanded: Vec::new(),
                        cap,
                        cursor: 0,
                    };
                    let mut model = ReferencePool {
                        keys: Vec::new(),
                        expanded: Vec::new(),
                        cap,
                        cursor: 0,
                    };
                    let mut id = 0;
                    for _ in 0..fill {
                        let candidate = key(1000.0 + (rng.next_u64() % 16) as f32, id);
                        id += 1;
                        assert_eq!(pool.insert(candidate), model.insert(candidate));
                    }
                    for _ in 0..=fill {
                        assert_eq!(pool.next(), model.next());
                    }
                    for round in 0..8 {
                        let size = if round == 0 {
                            count
                        } else {
                            (rng.next_u64() % 17) as usize
                        };
                        let mut pending = Vec::new();
                        for _ in 0..size {
                            let distance = if round == 0 {
                                (rng.next_u64() % 16) as f32
                            } else {
                                (rng.next_u64() % 32) as f32 - 8.0
                            };
                            pending.push(key(distance, id));
                            id += 1;
                        }
                        let mut improved = false;
                        for &candidate in &pending {
                            if let Some((position, _)) = model.insert(candidate) {
                                improved |= position < k;
                            }
                        }
                        assert_eq!(pool.insert_batch(&mut pending, k), improved);
                        check(&pool, &model);
                        for _ in 0..3 {
                            assert_eq!(pool.next(), model.next());
                        }
                        if round % 2 == 1 && !model.keys.is_empty() {
                            let removed =
                                model.keys[(rng.next_u64() % model.keys.len() as u64) as usize];
                            pool.remove(removed);
                            model.remove(removed);
                        }
                        check(&pool, &model);
                    }
                }
            }
        }
    }
}

#[test]
#[cfg(feature = "experimental-batch-pool")]
fn batch_pool_equal_distances_preserve_distinct_ids_and_flags() {
    for cap in [1usize, 2, 7, 8, 9, 63, 64, 65, 127, 128, 129] {
        for fill in [cap - 1, cap] {
            for first_id in [10, 1000] {
                let mut pool = Pool {
                    keys: Vec::new(),
                    expanded: Vec::new(),
                    #[cfg(feature = "experimental-batch-pool")]
                    merge_keys: Vec::new(),
                    #[cfg(feature = "experimental-batch-pool")]
                    merge_expanded: Vec::new(),
                    cap,
                    cursor: 0,
                };
                let mut model = ReferencePool {
                    keys: Vec::new(),
                    expanded: Vec::new(),
                    cap,
                    cursor: 0,
                };
                for id in first_id..first_id + fill as u32 {
                    assert_eq!(pool.insert(key(1.0, id)), model.insert(key(1.0, id)));
                }
                for _ in 0..=fill {
                    assert_eq!(pool.next(), model.next());
                }
                let mut pending: Vec<_> = [0, 200, 201, 202, 203, 204, 205, 206, 207]
                    .map(|id| key(1.0, id))
                    .to_vec();
                let mut improved = false;
                for &candidate in &pending {
                    if let Some((position, _)) = model.insert(candidate) {
                        improved |= position < 1;
                    }
                }
                assert_eq!(pool.insert_batch(&mut pending, 1), improved);
                assert_eq!(pool.keys, model.keys);
                assert_eq!(pool.cursor, model.cursor);
                for (position, &expanded) in model.expanded.iter().enumerate() {
                    assert_eq!(pool.is_expanded(position), expanded);
                }
                while let Some(expected) = model.next() {
                    assert_eq!(pool.next(), Some(expected));
                }
                assert_eq!(pool.next(), None);
            }
        }
    }
}

#[test]
#[cfg(feature = "experimental-batch-pool")]
fn batch_pool_preserves_search_results_work_and_reopening() {
    let dim = 16;
    let data = clustered(160, dim, 53);
    let queries = clustered(3, dim, 54);
    let directory = tempfile::tempdir().unwrap();
    for metric in [Metric::Angular, Metric::Euclidean] {
        let graph = Graph::build(
            &data,
            dim,
            &BuildParams {
                metric,
                degree: 32,
                build_ef: 64,
                quantized_search: true,
                ..Default::default()
            },
        )
        .unwrap();
        for bits in [4, 8] {
            let index = VGraph::from_graph_with(&graph, bits, 8, true, true, 3, false).unwrap();
            let path = directory.path().join("batch-pool.warm");
            index.save(&path).unwrap();
            let reopened = VGraph::open(&path, Residency::Mapped).unwrap();
            for batch_est in [false, true] {
                let knobs = SearchKnobs {
                    batch_est,
                    ..SearchKnobs::OFF
                };
                let mut plain = index.searcher_with(knobs);
                let mut batched = reopened.searcher_with(knobs);
                for ef in [7, 8, 9, 64, 129] {
                    for k in [0, 1, 7, 8, 9, 64] {
                        for patience in [0, 1, 4] {
                            for reservoir in [0, 80] {
                                let params = VSearchParams {
                                    ef,
                                    rerank: 80,
                                    patience,
                                    reservoir,
                                    ..Default::default()
                                };
                                for query in queries.chunks_exact(dim) {
                                    let (mut expected, mut actual) = (Vec::new(), Vec::new());
                                    let (mut a, mut b) =
                                        (QueryProfile::default(), QueryProfile::default());
                                    let ea = plain
                                        .scratch
                                        .search_inner::<true, false, true, false>(
                                            &index,
                                            query,
                                            k,
                                            params,
                                            &mut expected,
                                            &mut a,
                                        )
                                        .unwrap();
                                    let eb = batched
                                        .scratch
                                        .search_inner::<true, true, true, false>(
                                            &reopened,
                                            query,
                                            k,
                                            params,
                                            &mut actual,
                                            &mut b,
                                        )
                                        .unwrap();
                                    assert_eq!(expected, actual);
                                    assert_eq!(ea, eb);
                                    let work = |p: QueryProfile| {
                                        (
                                            p.expansions,
                                            p.neighbors_read,
                                            p.entry_evaluations,
                                            p.traversal_evaluations,
                                            p.rerank_evaluations,
                                            p.beam_retained,
                                            p.retained,
                                        )
                                    };
                                    assert_eq!(work(a), work(b));
                                    assert_eq!(plain.scratch.pool.keys, batched.scratch.pool.keys);
                                    assert_eq!(
                                        plain.scratch.pool.expanded,
                                        batched.scratch.pool.expanded
                                    );
                                    assert_eq!(
                                        plain.scratch.pool.cursor,
                                        batched.scratch.pool.cursor
                                    );
                                    assert_eq!(
                                        plain.scratch.visited.words,
                                        batched.scratch.visited.words
                                    );
                                    assert_eq!(
                                        plain.scratch.reservoir.keys,
                                        batched.scratch.reservoir.keys
                                    );
                                    let mut unprofiled = Vec::new();
                                    assert_eq!(
                                        plain.search(query, k, params, &mut unprofiled).unwrap(),
                                        ea
                                    );
                                    assert_eq!(unprofiled, expected);
                                    assert_eq!(
                                        batched.search(query, k, params, &mut unprofiled).unwrap(),
                                        eb
                                    );
                                    assert_eq!(unprofiled, actual);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Development microbenchmark (`--ignored`): a search-shaped stream (one expansion, then
/// 20 inserts of keys drawn near the pool's range) on the bit pool and the bool pool.
#[test]
#[ignore]
fn microbench_pool() {
    use std::hint::black_box;
    use std::time::Instant;
    for cap in [10usize, 20, 40, 80, 160, 320] {
        let stream: Vec<u64> = {
            let mut rng = SplitMix(cap as u64);
            (0..200_000).map(|_| rng.next_u64() % 100_000).collect()
        };
        let mut best = [f64::INFINITY; 2];
        for _ in 0..7 {
            for (which, b) in best.iter_mut().enumerate() {
                let t = Instant::now();
                let mut pool = Pool {
                    keys: Vec::new(),
                    expanded: Vec::new(),
                    #[cfg(feature = "experimental-batch-pool")]
                    merge_keys: Vec::new(),
                    #[cfg(feature = "experimental-batch-pool")]
                    merge_expanded: Vec::new(),
                    cap: 1,
                    cursor: 0,
                };
                let mut model = ReferencePool {
                    keys: Vec::new(),
                    expanded: Vec::new(),
                    cap,
                    cursor: 0,
                };
                pool.reset(cap);
                let mut acc = 0u64;
                for (i, &k) in stream.iter().enumerate() {
                    if which == 0 {
                        if i % 21 == 0 {
                            if pool.next().is_none() {
                                pool.reset(cap);
                            }
                        } else if let Some((p, _)) = pool.insert(black_box(k)) {
                            acc += p as u64;
                        }
                    } else if i % 21 == 0 {
                        if model.next().is_none() {
                            model = ReferencePool {
                                keys: Vec::new(),
                                expanded: Vec::new(),
                                cap,
                                cursor: 0,
                            };
                        }
                    } else if let Some((p, _)) = model.insert(black_box(k)) {
                        acc += p as u64;
                    }
                }
                black_box(acc);
                *b = b.min(t.elapsed().as_nanos() as f64 / stream.len() as f64);
            }
        }
        println!(
            "{{\"cap\": {cap}, \"bits_ns_per_op\": {:.2}, \"bools_ns_per_op\": {:.2}}}",
            best[0], best[1]
        );
    }
}

#[test]
fn compact_lists_preserve_order_search_and_persisted_bytes() {
    let dim = 16;
    let data = clustered(64, dim, 7);
    let queries = clustered(8, dim, 8);
    let mut graph = Graph::build(
        &data,
        dim,
        &BuildParams {
            degree: 16,
            build_ef: 32,
            ..BuildParams::default()
        },
    )
    .unwrap();
    for v in 0..graph.n {
        let count = [0, 1, 15, 16][v % 4];
        let ids: Vec<u32> = (1..=count).map(|j| ((v + j) % graph.n) as u32).collect();
        graph.adj.set(v, &ids);
    }
    let directory = tempfile::tempdir().unwrap();
    for bytes in [3, 4] {
        for bits in [4, 8] {
            let mut index =
                VGraph::from_graph_with(&graph, bits, 8, true, true, bytes, false).unwrap();
            let lists: Vec<Vec<u32>> = (0..index.n)
                .map(|v| {
                    let mut ids = Vec::new();
                    index.for_each_neighbour(v, |u| ids.push(u));
                    ids
                })
                .collect();
            let before = directory.path().join("before");
            let after = directory.path().join("after");
            index.save(&before).unwrap();
            let old_bytes = index.index_bytes();
            let old_memory = index.memory_breakdown();
            let params = VSearchParams {
                ef: 32,
                rerank: 16,
                ..VSearchParams::default()
            };
            let expected: Vec<Vec<u32>> = {
                let mut searcher = index.searcher_with(SearchKnobs::OFF);
                queries
                    .chunks_exact(dim)
                    .map(|q| {
                        let mut out = Vec::new();
                        searcher.search(q, 10, params, &mut out).unwrap();
                        out
                    })
                    .collect()
            };
            index.compact_adjacency().unwrap();
            assert!(index.index_bytes() < old_bytes);
            let memory = index.memory_breakdown();
            assert_eq!(
                old_memory.adjacency_ids_bytes,
                graph.n * graph.degree * bytes as usize
            );
            assert_eq!(old_memory.adjacency_offsets_bytes, 0);
            assert_eq!(
                memory.adjacency_ids_bytes,
                lists.iter().map(Vec::len).sum::<usize>() * bytes as usize
            );
            assert_eq!(memory.adjacency_offsets_bytes, (graph.n + 1) * 4);
            assert_eq!(
                old_bytes - index.index_bytes(),
                old_memory.adjacency_ids_bytes
                    - memory.adjacency_ids_bytes
                    - memory.adjacency_offsets_bytes
            );
            for (v, want) in lists.iter().enumerate() {
                let mut actual = Vec::new();
                index.for_each_neighbour(v, |u| actual.push(u));
                assert_eq!(&actual, want);
            }
            let compact_bytes = index.index_bytes();
            index.compact_adjacency().unwrap();
            assert_eq!(index.index_bytes(), compact_bytes);
            index.save(&after).unwrap();
            assert_eq!(
                std::fs::read(&before).unwrap(),
                std::fs::read(&after).unwrap()
            );
            for residency in [Residency::Mapped, Residency::Prefaulted, Residency::Heap] {
                let reopened = VGraph::open(&after, residency).unwrap();
                let (mut a, mut b) = (
                    index.searcher_with(SearchKnobs::OFF),
                    reopened.searcher_with(SearchKnobs::OFF),
                );
                for i in 0..260 {
                    let qi = i % expected.len();
                    let q = &queries[qi * dim..(qi + 1) * dim];
                    let (mut x, mut y) = (Vec::new(), Vec::new());
                    let evals = a.search(q, 10, params, &mut x).unwrap();
                    b.search(q, 10, params, &mut y).unwrap();
                    assert_eq!(x, expected[qi]);
                    assert_eq!(x, y);
                    #[cfg(feature = "experimental")]
                    {
                        let profile = a.profile_search(q, 10, params, &mut y).unwrap();
                        assert_eq!(x, y);
                        assert_eq!(
                            evals,
                            profile.entry_evaluations
                                + profile.traversal_evaluations
                                + profile.rerank_evaluations
                        );
                        assert!(profile.neighbors_read >= profile.traversal_evaluations - 1);
                    }
                    #[cfg(not(feature = "experimental"))]
                    let _ = evals;
                }
            }
        }
    }
}

/// A cache whose list lengths claim more data than the file holds is refused by reading,
/// not by first allocating what the lengths claim.
#[test]
fn graph_cache_lengths_are_bounded_by_the_file() {
    let d = 16;
    let n = 64usize;
    let data = clustered(n, d, 4);
    let params = BuildParams {
        build_ef: 16,
        degree: 8,
        seed: 3,
        ..BuildParams::default()
    };
    let graph = Graph::build(&data, d, &params).expect("build");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.cache");
    graph.save(&path).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    // The adjacency lengths list starts with `n` as a u64, the second such word after
    // the magic (the first is the header's own `n`).
    let lens = (8..bytes.len() - 8)
        .step_by(8)
        .filter(|&at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) == n as u64)
        .nth(1)
        .expect("lengths list");
    for i in 0..n {
        let at = lens + 8 + 4 * i;
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    }
    let flat = lens + 8 + 4 * n;
    bytes[flat..flat + 8].copy_from_slice(&(n as u64 * u64::from(u32::MAX)).to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let error = Graph::load(&data, d, &path, &params)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("shorter than its lengths"), "{error}");
}

/// The cache round-trips the structure for the same data and parameters, and rejects a
/// file written for other data or for any other build parameter.
#[test]
fn graph_cache_round_trips_and_rejects_mismatches() {
    let d = 16;
    let data = clustered(600, d, 4);
    let params = BuildParams {
        build_ef: 32,
        degree: 8,
        alpha: 1.1,
        seed: 9,
        ..BuildParams::default()
    };
    let graph = Graph::build(&data, d, &params).expect("build");
    let path = std::env::temp_dir().join(format!("warm-qgraph-cache-{}", std::process::id()));
    graph.save(&path).unwrap();
    let back = Graph::load(&data, d, &path, &params).unwrap();
    assert_eq!(back.structure_hash(), graph.structure_hash());
    assert!(back.adj == graph.adj && back.layers == graph.layers);
    assert_eq!(
        (back.entry, &back.upper_ids),
        (graph.entry, &graph.upper_ids)
    );
    assert_eq!(back.vectors, graph.vectors);
    let rejects = |data: &[f32], p: &BuildParams, what: &str| {
        let err = Graph::load(data, d, &path, p)
            .err()
            .expect("a mismatched cache was accepted")
            .to_string();
        assert!(err.contains(what), "{what}: {err}");
    };
    for (what, p) in [
        (
            "degree",
            BuildParams {
                degree: 10,
                ..params.clone()
            },
        ),
        (
            "build_ef",
            BuildParams {
                build_ef: 33,
                ..params.clone()
            },
        ),
        (
            "alpha",
            BuildParams {
                alpha: 1.0,
                ..params.clone()
            },
        ),
        (
            "seed",
            BuildParams {
                seed: 10,
                ..params.clone()
            },
        ),
        (
            "metric",
            BuildParams {
                metric: Metric::Euclidean,
                ..params.clone()
            },
        ),
    ] {
        rejects(&data, &p, what);
    }
    let mut other = data.clone();
    other[d * 7 + 3] += 0.5;
    rejects(&other, &params, "data fingerprint");
    rejects(&data[..d * 599], &params, "n");
    std::fs::write(&path, b"HMSGRF01").unwrap();
    rejects(&data, &params, "bad magic");
    std::fs::remove_file(&path).unwrap();
}

/// On clustered rows whose norms spread over [0.1, 10], a Euclidean index (4-bit codes,
/// 8-bit residual re-rank, reordered 3-byte ids: the benchmark layout) finds the exact
/// Euclidean neighbours, while an angular index over the same rows, which ranks by
/// direction alone, clearly does not.
#[test]
fn euclidean_index_finds_euclidean_neighbours() {
    let (d, n, k) = (32, 4000, 10);
    let spread = |rows: &mut [f32], salt: usize| {
        let count = rows.len() / d;
        for (i, x) in rows.chunks_exact_mut(d).enumerate() {
            let s = 0.1 * 100f32.powf(((i * 7919 + salt) % count) as f32 / count as f32);
            x.iter_mut().for_each(|v| *v *= s);
        }
    };
    // Queries are held-out rows of the same clusters.
    let mut data = clustered(n + 100, d, 7);
    spread(&mut data, 0);
    let queries = data.split_off(n * d);
    let l2 = |a: &[f32], b: &[f32]| -> f64 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
            .sum()
    };
    let recall = |metric: Metric| -> f64 {
        let params = BuildParams {
            metric,
            degree: 32,
            build_ef: 100,
            ..BuildParams::default()
        };
        let g = Graph::build(&data, d, &params).expect("build");
        let index = VGraph::from_graph_with(&g, 4, 8, true, true, 3, false).expect("encode");
        let mut s = index.searcher_with(SearchKnobs::OFF);
        let mut out = Vec::new();
        let mut hits = 0;
        for q in queries.chunks_exact(d) {
            let mut all: Vec<(f64, u32)> = data
                .chunks_exact(d)
                .enumerate()
                .map(|(i, x)| (l2(q, x), i as u32))
                .collect();
            all.sort_by(|a, b| a.0.total_cmp(&b.0));
            let truth: Vec<u32> = all[..k].iter().map(|t| t.1).collect();
            let params = VSearchParams {
                ef: 64,
                rerank: 32,
                ..VSearchParams::default()
            };
            s.search(q, k, params, &mut out).expect("search");
            hits += out.iter().filter(|x| truth.contains(x)).count();
        }
        hits as f64 / (k * 100) as f64
    };
    let (euclidean, angular) = (recall(Metric::Euclidean), recall(Metric::Angular));
    assert!(euclidean >= 0.95, "Euclidean recall {euclidean}");
    assert!(
        angular < euclidean - 0.3,
        "angular recall {angular} against Euclidean {euclidean}"
    );
}

/// A Euclidean index refuses the angular-only encodings and accepts a zero query (its
/// nearest rows are the ones of smallest norm).
#[test]
fn euclidean_index_limits() {
    let d = 16;
    let mut data = clustered(500, d, 2);
    for (i, x) in data.chunks_exact_mut(d).enumerate() {
        x.iter_mut().for_each(|v| *v *= 1.0 + i as f32 / 50.0);
    }
    let params = BuildParams {
        metric: Metric::Euclidean,
        degree: 16,
        build_ef: 64,
        ..BuildParams::default()
    };
    let g = Graph::build(&data, d, &params).expect("build");
    assert!(VGraph::from_graph_with(&g, 8, 0, false, false, 4, true).is_err());
    assert!(super::super::QGraph::from_graph(&g, &params).is_err());
    let blocks = VGraph::from_graph(&g, false)
        .expect("encode")
        .with_blocks(&g, 4);
    assert!(blocks.is_err());
    let index = VGraph::from_graph(&g, true).expect("encode");
    let mut out = Vec::new();
    let params = VSearchParams {
        ef: 500,
        rerank: 500,
        ..VSearchParams::default()
    };
    index
        .searcher_with(SearchKnobs::OFF)
        .search(&[0.0; 16], 1, params, &mut out)
        .expect("zero query");
    let smallest = data
        .chunks_exact(d)
        .enumerate()
        .min_by(|a, b| {
            let n2 = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>();
            n2(a.1).total_cmp(&n2(b.1))
        })
        .map(|m| m.0 as u32);
    assert_eq!(out.first().copied(), smallest);
}

/// Per coordinate the 8-bit code is off by at most half a step, and the residual code
/// shrinks that error by about another factor of 127.
#[test]
fn quantization_error_is_bounded() {
    let mut rng = SplitMix(12);
    let x: Vec<f32> = (0..100).map(|_| gaussian(&mut rng)).collect();
    let mut c = vec![0i8; 112];
    let s = quantize(&x, &mut c);
    let mut r: Vec<f32> = x
        .iter()
        .zip(&c)
        .map(|(a, &q)| a - s * f32::from(q))
        .collect();
    assert!(r.iter().all(|e| e.abs() <= s / 2.0 + 1e-6));
    assert!(c[100..].iter().all(|&q| q == 0));
    let mut rc = vec![0i8; 112];
    let sr = quantize(&r, &mut rc);
    r.iter_mut()
        .zip(&rc)
        .for_each(|(a, &q)| *a -= sr * f32::from(q));
    assert!(r.iter().all(|e| e.abs() <= s / 254.0 + 1e-6));
    assert_eq!(quantize(&[0.0; 4], &mut c[..16]), 0.0);
}

fn recall(index: &VGraph, data: &[f32], queries: &[f32], d: usize, p: VSearchParams) -> f64 {
    let mut s = index.searcher();
    let mut out = Vec::new();
    let mut hits = 0;
    for q in queries.chunks_exact(d) {
        let truth = brute_force(data, d, q, 10);
        s.search(q, 10, p, &mut out).expect("search");
        assert_eq!(out.len(), 10);
        hits += out.iter().filter(|x| truth.contains(x)).count();
    }
    hits as f64 / (queries.len() / d * 10) as f64
}

#[test]
fn recall_on_clustered_data() {
    let d = 24;
    let data = clustered(4000, d, 1);
    let queries = clustered(100, d, 2);
    let graph = Graph::build(&data, d, &BuildParams::default()).expect("build");
    let plain = VGraph::from_graph(&graph, false).expect("encode");
    let fine = VGraph::from_graph(&graph, true).expect("encode");
    assert!(plain.index_bytes() < fine.index_bytes());
    let pool = VSearchParams {
        ef: 64,
        ..VSearchParams::default()
    };
    let rr = VSearchParams {
        ef: 64,
        rerank: 32,
        ..VSearchParams::default()
    };
    let r0 = recall(&plain, &data, &queries, d, pool);
    let r1 = recall(&fine, &data, &queries, d, rr);
    // Same regression guard as the edge-code index on this deliberately hard set.
    assert!(r0 >= 0.9, "recall {r0}");
    assert!(r1 >= r0, "rerank {r1} below pool order {r0}");
    let fine4 = VGraph::from_graph_with(&graph, 8, 4, false, false, 4, false).expect("encode");
    assert!(plain.index_bytes() < fine4.index_bytes());
    assert!(fine4.index_bytes() < fine.index_bytes());
    let r2 = recall(&fine4, &data, &queries, d, rr);
    assert!(r2 >= r0 - 0.02, "4-bit rerank {r2} below pool order {r0}");
    let lvq48 = VGraph::from_graph_with(&graph, 4, 8, true, false, 4, false).expect("encode");
    assert!(lvq48.index_bytes() < fine.index_bytes());
    // A sanity bound: 4-bit codes of 24 coordinates are coarse, so a wider pool and re-rank.
    let wide = VSearchParams {
        ef: 96,
        rerank: 64,
        ..VSearchParams::default()
    };
    let r3 = recall(&lvq48, &data, &queries, d, wide);
    assert!(
        r3 >= r0 - 0.05,
        "4-bit traversal with 8-bit residual {r3} vs {r0}"
    );
}

/// Neighbour-block codes estimate every neighbour from the expanded vertex's record; the
/// traversal must reach the same recall band as the per-row 8-bit estimates after the
/// residual re-rank, at every block width, with and without renumbering or 3-byte ids.
#[test]
fn block_codes_keep_recall() {
    let d = 24;
    let data = clustered(4000, d, 1);
    let queries = clustered(100, d, 2);
    let graph = Graph::build(&data, d, &BuildParams::default()).expect("build");
    let plain = VGraph::from_graph(&graph, true).expect("encode");
    let rr = VSearchParams {
        ef: 96,
        rerank: 64,
        ..VSearchParams::default()
    };
    let r0 = recall(&plain, &data, &queries, d, rr);
    for bits in [4u8, 2, 1] {
        for (reorder, id_bytes) in [(false, 4u8), (true, 3)] {
            let b = VGraph::from_graph_with(&graph, 8, 8, true, reorder, id_bytes, false)
                .expect("encode")
                .with_blocks(&graph, bits)
                .expect("blocks");
            assert!(b.has_blocks());
            assert!(b.index_bytes() > plain.index_bytes());
            let r = recall(&b, &data, &queries, d, rr);
            // 24 coordinates are few for 1-bit codes (a 1/sqrt(dim) relative error on
            // this deliberately hard set), so the band widens with coarseness; the
            // estimator itself is checked against exact inner products below.
            let slack = match bits {
                4 => 0.03,
                2 => 0.06,
                _ => 0.30,
            };
            assert!(
                r >= r0 - slack,
                "{bits}-bit blocks (reorder {reorder}, ids {id_bytes}) recall {r} vs {r0}"
            );
        }
    }
    let e = match VGraph::from_graph(&graph, true)
        .expect("encode")
        .with_blocks(&graph, 3)
    {
        Ok(_) => panic!("3 bits accepted"),
        Err(e) => e.to_string(),
    };
    assert!(e.contains("block bits"), "{e}");
}

/// The block estimate of a neighbour, in `est` units, must track the exact centred inner
/// product: Pearson correlation over the neighbours of 64 vertices above a per-width bar.
#[test]
fn block_estimates_track_exact_inner_products() {
    let d = 64;
    let data = clustered(3000, d, 5);
    let graph = Graph::build(&data, d, &BuildParams::default()).expect("build");
    // The index works on unit rows; so must the exact products.
    let unit = &graph.vectors;
    for (bits, bar) in [(4u8, 0.998), (2, 0.98), (1, 0.90)] {
        let g = VGraph::from_graph(&graph, false)
            .expect("encode")
            .with_blocks(&graph, bits)
            .expect("blocks");
        let mut s = g.searcher();
        let q: Vec<f32> = unit[7 * d..8 * d].to_vec();
        s.scratch.qf[..d].copy_from_slice(&q);
        let sq = g.encode_query(
            &mut s.scratch.qf,
            &mut s.scratch.qc,
            &mut s.scratch.qtmp,
            &mut s.scratch.qrot,
            &mut s.scratch.qs,
        );
        g.encode_block_query(&s.scratch.qf, sq, &mut s.scratch.bqrot, &mut s.scratch.bq);
        let centred =
            |u: usize| -> f32 { (0..d).map(|j| q[j] * (unit[u * d + j] - g.mean[j])).sum() };
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for v in 0..64usize {
            // The exact `<q, x_v - mean>` stands in for the expanded vertex's estimate.
            let ip_v = centred(v);
            let mut list = Vec::new();
            g.for_each_neighbour(v, |u| list.push(u));
            for b in 0..g.blocks_per_vertex {
                let (codes, scales, offsets) = g.block(v, b);
                scan32(codes, &s.scratch.bq.lut, g.block_m, &mut s.scratch.scan);
                for i in 0..BLOCK {
                    let Some(&u) = list.get(b * BLOCK + i) else {
                        break;
                    };
                    let ip = ip_v
                        + le_f32(scales, i)
                            * (s.scratch.bq.delta * s.scratch.scan[i] as f32
                                + s.scratch.bq.min_sum)
                        + le_f32(offsets, i);
                    xs.push(f64::from(ip));
                    ys.push(f64::from(centred(u as usize)));
                }
            }
        }
        let n = xs.len() as f64;
        let (mx, my) = (xs.iter().sum::<f64>() / n, ys.iter().sum::<f64>() / n);
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for (x, y) in xs.iter().zip(&ys) {
            sxy += (x - mx) * (y - my);
            sxx += (x - mx) * (x - mx);
            syy += (y - my) * (y - my);
        }
        let r = sxy / (sxx * syy).sqrt();
        eprintln!(
            "{bits}-bit block estimates: correlation {r:.4} over {} pairs",
            xs.len()
        );
        assert!(
            r > bar,
            "{bits}-bit block estimates: correlation {r} below {bar}"
        );
    }
}

/// Delta-coded lists hold the same neighbour sets as the 4-byte ones, in fewer bytes,
/// and the search returns the same ids; the block index decodes them in the same order
/// it was built in.
#[test]
fn delta_coded_adjacency_round_trips() {
    let d = 24;
    let data = clustered(4000, d, 1);
    let queries = clustered(50, d, 2);
    let graph = Graph::build(&data, d, &BuildParams::default()).expect("build");
    let wide = VGraph::from_graph_with(&graph, 4, 8, true, true, 4, false).expect("encode");
    let delta = VGraph::from_graph_with(&graph, 4, 8, true, true, 0, false).expect("encode");
    assert!(
        delta.index_bytes() < wide.index_bytes(),
        "{} vs {}",
        delta.index_bytes(),
        wide.index_bytes()
    );
    for v in 0..wide.n {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        wide.for_each_neighbour(v, |u| a.push(u));
        delta.for_each_neighbour(v, |u| b.push(u));
        a.sort_unstable();
        assert_eq!(a, b, "vertex {v}");
    }
    let p = VSearchParams {
        ef: 48,
        rerank: 16,
        ..VSearchParams::default()
    };
    let (mut s1, mut s2) = (wide.searcher(), delta.searcher());
    let (mut o1, mut o2) = (Vec::new(), Vec::new());
    for q in queries.chunks_exact(d) {
        s1.search(q, 10, p, &mut o1).expect("search");
        s2.search(q, 10, p, &mut o2).expect("search");
        assert_eq!(o1, o2);
    }
    let blocks = VGraph::from_graph_with(&graph, 8, 8, true, true, 0, false)
        .expect("encode")
        .with_blocks(&graph, 2)
        .expect("blocks");
    assert!(recall(&blocks, &data, &queries, d, p) > 0.8);
    assert_eq!(zigzag(-1), 1);
    assert_eq!(unzigzag(zigzag(-123_456)), -123_456);
    let mut buf = Vec::new();
    put_varint(&mut buf, 300);
    assert_eq!(get_varint(&buf), (300, 2));
}

/// Renumbering and aligning the rows change the layout, not the search: the same ids come
/// back (up to ties between equal estimates, which break by the stored id).
#[test]
fn reordered_aligned_index_returns_the_same_ids() {
    let d = 24;
    let data = clustered(3000, d, 5);
    let graph = Graph::build(&data, d, &BuildParams::default()).expect("build");
    let a = VGraph::from_graph(&graph, true).expect("encode");
    let b = VGraph::from_graph_with(&graph, 8, 8, true, true, 4, false).expect("encode");
    // 32-byte rows already sit inside one line; the id map is the only extra.
    assert_eq!(b.index_bytes(), a.index_bytes() + b.n * 4);
    let (mut sa, mut sb) = (a.searcher(), b.searcher());
    let (mut oa, mut ob) = (Vec::new(), Vec::new());
    let c = VGraph::from_graph_with(&graph, 8, 8, true, true, 3, false).expect("encode");
    assert_eq!(c.index_bytes(), b.index_bytes() - c.n * c.degree);
    let mut sc = c.searcher();
    let mut oc = Vec::new();
    let mut same = 0;
    for q in data.chunks_exact(d).take(200) {
        for p in [
            VSearchParams {
                ef: 32,
                ..VSearchParams::default()
            },
            VSearchParams {
                ef: 48,
                rerank: 16,
                ..VSearchParams::default()
            },
        ] {
            sa.search(q, 10, p, &mut oa).expect("search");
            sb.search(q, 10, p, &mut ob).expect("search");
            sc.search(q, 10, p, &mut oc).expect("search");
            assert_eq!(ob, oc, "3-byte ids changed the search");
            same += usize::from(oa == ob);
        }
    }
    assert!(same >= 396, "{same} of 400 searches agree");
}

#[test]
fn build_and_search_are_deterministic() {
    let d = 20;
    let data = clustered(3000, d, 3);
    let params = BuildParams {
        seed: 9,
        ..BuildParams::default()
    };
    let a =
        VGraph::from_graph(&Graph::build(&data, d, &params).expect("build"), true).expect("encode");
    let b =
        VGraph::from_graph(&Graph::build(&data, d, &params).expect("build"), true).expect("encode");
    assert!(
        a.codes.bytes() == b.codes.bytes()
            && a.residual[..] == b.residual[..]
            && a.adj[..] == b.adj[..]
    );
    assert!(a.scales[..] == b.scales[..] && a.res_scales[..] == b.res_scales[..]);
    assert_eq!((a.entry, &a.mean), (b.entry, &b.mean));
    let (mut sa, mut sb) = (a.searcher(), b.searcher());
    let (mut oa, mut ob) = (Vec::new(), Vec::new());
    let p = VSearchParams {
        ef: 32,
        rerank: 16,
        ..VSearchParams::default()
    };
    for q in data.chunks_exact(d).take(50) {
        sa.search(q, 10, p, &mut oa).expect("search");
        sb.search(q, 10, p, &mut ob).expect("search");
        assert_eq!(oa, ob);
    }
}

/// Every failure mode is an error that names its cause; no input panics.
#[test]
fn invalid_inputs_are_errors_that_name_the_cause() {
    let d = 16;
    let data = clustered(500, d, 4);
    let err = |r: Result<Graph, WarmError>| match r {
        Err(e) => e.to_string(),
        Ok(_) => panic!("an invalid input was accepted"),
    };
    assert!(err(Graph::build(&[], d, &BuildParams::default())).contains("empty"));
    assert!(err(Graph::build(&data[..d + 1], d, &BuildParams::default())).contains("ragged"));
    assert!(err(Graph::build(&data, 0, &BuildParams::default())).contains("ragged"));
    let mut bad = data.clone();
    bad[3 * d + 2] = f32::NAN;
    let e = err(Graph::build(&bad, d, &BuildParams::default()));
    assert!(e.contains("non-finite") && e.contains("row 3"), "{e}");
    let odd = BuildParams {
        degree: 7,
        ..BuildParams::default()
    };
    assert!(err(Graph::build(&data, d, &odd)).contains("even"));
    let graph = Graph::build(&data, d, &BuildParams::default()).expect("build");
    let e = match VGraph::from_graph_with(&graph, 5, 8, false, false, 4, false) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("bits 5 was accepted"),
    };
    assert!(e.contains("bits"), "{e}");
    let index = VGraph::from_graph(&graph, true).expect("encode");
    let mut s = index.searcher();
    let mut out = Vec::new();
    let p = VSearchParams::default();
    let e = s
        .search(&data[..d - 1], 10, p, &mut out)
        .expect_err("short query")
        .to_string();
    assert!(e.contains("dimension"), "{e}");
    let mut q = data[..d].to_vec();
    q[5] = f32::INFINITY;
    let e = s.search(&q, 10, p, &mut out).expect_err("inf").to_string();
    assert!(e.contains("non-finite") && e.contains('5'), "{e}");
    let e = s
        .search(&vec![0.0; d], 10, p, &mut out)
        .expect_err("zero")
        .to_string();
    assert!(e.contains("zero"), "{e}");
    // Adversarial sizes stay bounded by the vertex count: a huge ef and k finish and
    // return at most n ids.
    let huge = VSearchParams {
        ef: usize::MAX,
        rerank: usize::MAX,
        ..VSearchParams::default()
    };
    let evals = s.search(&data[..d], 10, huge, &mut out).expect("search");
    assert_eq!(out.len(), 10);
    assert!(evals <= 2 * index.len() + 10);
    s.search(&data[..d], 5000, huge, &mut out).expect("search");
    assert!(out.len() <= index.len());
    // A zero data row is allowed and never returned ahead of real neighbours by a NaN.
    let mut with_zero = data.clone();
    with_zero[..d].fill(0.0);
    let g0 = Graph::build(&with_zero, d, &BuildParams::default()).expect("build");
    let i0 = VGraph::from_graph(&g0, true).expect("encode");
    let mut s0 = i0.searcher();
    s0.search(&data[d..2 * d], 10, p, &mut out).expect("search");
    assert_eq!(out.len(), 10);
    assert!(out.iter().all(|&v| (v as usize) < i0.len()));
}

/// The screen only decides which neighbours get the 8-bit estimate: with a wide margin
/// it skips nothing and the ids are identical; at one sigma recall stays close.
#[test]
fn screen_is_transparent_at_a_wide_margin() {
    let d = 24;
    let data = clustered(3000, d, 5);
    let queries = clustered(100, d, 2);
    let graph = Graph::build(&data, d, &BuildParams::default()).expect("build");
    let plain = VGraph::from_graph(&graph, true).expect("encode");
    let screened = VGraph::from_graph_with(&graph, 8, 8, false, false, 4, true).expect("encode");
    assert!(screened.has_screen() && screened.index_bytes() > plain.index_bytes());
    let (mut sa, mut sb) = (plain.searcher(), screened.searcher());
    let (mut oa, mut ob) = (Vec::new(), Vec::new());
    let wide = VSearchParams {
        ef: 64,
        rerank: 16,
        screen_sigmas: 1e6,
        ..VSearchParams::default()
    };
    for q in queries.chunks_exact(d) {
        let ea = sa.search(q, 10, wide, &mut oa).expect("search");
        let eb = sb.search(q, 10, wide, &mut ob).expect("search");
        assert_eq!((ea, &oa), (eb, &ob));
        assert_eq!(sb.last_screened(), 0);
    }
    let one = VSearchParams {
        ef: 64,
        rerank: 16,
        ..VSearchParams::default()
    };
    let r0 = recall(&plain, &data, &queries, d, one);
    let r1 = recall(&screened, &data, &queries, d, one);
    assert!(r1 >= r0 - 0.02, "screened {r1} vs {r0}");
    let mut skipped = 0;
    for q in queries.chunks_exact(d) {
        sb.search(q, 10, one, &mut ob).expect("search");
        skipped += sb.last_screened();
    }
    assert!(skipped > 0, "the screen never skipped a neighbour");
}

#[test]
fn query_lists_find_neighbours_without_changing_plain_search_and_survive_reopening() {
    let d = 16;
    let data = clustered(3000, d, 41);
    // Training and evaluation queries share one distribution (the same cluster centres), and
    // the evaluation queries are held out of the lists.
    let drawn = clustered(660, d, 42);
    let (training, queries) = drawn.split_at(600 * d);
    let (k, per_query) = (10, 20);
    let neighbours: Vec<u32> = training
        .chunks_exact(d)
        .flat_map(|q| brute_force(&data, d, q, per_query))
        .collect();
    let graph = Graph::build(&data, d, &BuildParams::default()).unwrap();
    let plain = VGraph::from_graph_with(&graph, 8, 8, true, true, 3, false).unwrap();
    let mut listed = VGraph::from_graph_with(&graph, 8, 8, true, true, 3, false).unwrap();
    let lists = QueryLists::build(
        training,
        d,
        true,
        &neighbours,
        per_query,
        per_query,
        24,
        3000,
    )
    .unwrap();
    assert_eq!(lists.lists(), 24);
    assert!(QueryLists::build(training, d + 1, true, &neighbours, per_query, 5, 24, 3000).is_err());
    listed.set_query_lists(lists).unwrap();
    let base = VSearchParams {
        ef: 32,
        rerank: 32,
        ..VSearchParams::default()
    };
    let probe = VSearchParams {
        probes: 3,
        probe_only: true,
        ..base
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lists.idx");
    listed.save(&path).unwrap();
    let reopened = VGraph::open(&path, Residency::Heap).unwrap();
    assert_eq!(
        reopened.query_lists().map(QueryLists::listed),
        listed.query_lists().map(QueryLists::listed)
    );
    let (mut a, mut b, mut c, mut e) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut hits = 0;
    for q in queries.chunks_exact(d) {
        plain.searcher().search(q, k, base, &mut a).unwrap();
        listed.searcher().search(q, k, base, &mut b).unwrap();
        assert_eq!(a, b, "lists changed a search without probes");
        listed.searcher().search(q, k, probe, &mut c).unwrap();
        reopened.searcher().search(q, k, probe, &mut e).unwrap();
        assert_eq!(c, e, "reopened lists answer differently");
        assert_eq!(c.len(), k);
        let truth = brute_force(&data, d, q, k);
        hits += c.iter().filter(|x| truth.contains(x)).count();
    }
    let recall = hits as f64 / (queries.len() / d * k) as f64;
    assert!(recall >= 0.9, "probe-only recall {recall}");
    let mut damaged = std::fs::read(&path).unwrap();
    let at = damaged.len() - 1;
    damaged.truncate(at);
    std::fs::write(&path, &damaged).unwrap();
    assert!(VGraph::open(&path, Residency::Heap).is_err());
}
