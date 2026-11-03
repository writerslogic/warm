// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::core::qgraph::diagnostics::{NavigationDiagnostic, NavigationEvent};

impl VGraph {
    /// Base-layer edges in original-ID order, decoded from the serving index.
    pub fn diagnostic_adjacency(&self) -> Vec<Vec<u32>> {
        let mut adjacency = vec![Vec::new(); self.n];
        for id in 0..self.n as u32 {
            self.for_each_neighbour(id as usize, |neighbor| {
                adjacency[self.original(id) as usize].push(self.original(neighbor));
            });
        }
        adjacency
    }

    /// Replays plain coded traversal with admission/eviction events and checks its
    /// visited set, ordered beam, results and expansion count against serving search.
    /// The replay is untimed and never changes the serving implementation.
    pub fn diagnose_navigation(
        &mut self,
        source: &Graph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
    ) -> Result<NavigationDiagnostic, WarmError> {
        if params.patience != 0 || params.neighbour_cap != 0 || params.reservoir != 0 {
            return Err(invalid(
                "navigation requires uncapped traversal without patience or reservoir".into(),
            ));
        }
        let stages = self.diagnose(source, query, k, params, None, false)?;
        let mut searcher = self.searcher_with(SearchKnobs::OFF);
        let mut production = Vec::new();
        let profile = searcher.profile_search(query, k, params, &mut production)?;
        let scratch = &mut searcher.scratch;
        let sq = self.encode_query(
            &scratch.qf,
            &mut scratch.qc,
            &mut scratch.qtmp,
            &mut scratch.qrot,
            &mut scratch.qs,
        );
        let w = if sq > 0.0 { 1.0 / sq } else { 1.0 };
        let score = |id| self.est(&scratch.qc, scratch.qsum, id, w);
        let entry = self.diagnostic_entry(score, &mut Vec::new());
        let mut pool = Pool {
            keys: Vec::new(),
            expanded: Vec::new(),
            #[cfg(feature = "experimental-batch-pool")]
            merge_keys: Vec::new(),
            #[cfg(feature = "experimental-batch-pool")]
            merge_expanded: Vec::new(),
            cap: params.ef.max(k).max(1),
            cursor: 0,
        };
        let mut seen = vec![false; self.n];
        let mut events = Vec::new();
        let offer = |id: u32, parent: Option<u32>, pool: &mut Pool, events: &mut Vec<_>| {
            let distance = score(id);
            let worst = pool
                .worst()
                .map(|c| (key_dist(c), self.original(key_id(c))));
            let result = pool.insert(key(distance, id));
            events.push(NavigationEvent::Offer {
                id: self.original(id),
                stored_id: id,
                parent: parent.map(|p| self.original(p)),
                distance,
                worst,
                admitted: result.is_some(),
                evicted: result.and_then(|(_, e)| e.map(|c| self.original(key_id(c)))),
            });
        };
        seen[entry as usize] = true;
        offer(entry, None, &mut pool, &mut events);
        let mut expansions = 0;
        let mut seed_cursor = 0;
        loop {
            let Some(candidate) = pool.next() else {
                if pool.keys.len() >= k {
                    break;
                }
                let seed = (seed_cursor..self.n)
                    .find(|&id| !seen[id])
                    .ok_or_else(|| invalid("navigation exhausted recovery seeds".into()))?;
                seed_cursor = seed + 1;
                seen[seed] = true;
                offer(seed as u32, None, &mut pool, &mut events);
                continue;
            };
            let id = key_id(candidate);
            expansions += 1;
            events.push(NavigationEvent::Expand {
                id: self.original(id),
            });
            self.for_each_neighbour(id as usize, |neighbor| {
                if !seen[neighbor as usize] {
                    seen[neighbor as usize] = true;
                    offer(neighbor, Some(id), &mut pool, &mut events);
                }
            });
        }
        if pool.keys != scratch.pool.keys
            || (0..self.n).any(|id| seen[id] != scratch.visited.seen(id as u32))
            || expansions != profile.expansions
            || production != stages.results
        {
            return Err(invalid(
                "navigation replay differs from serving traversal".into(),
            ));
        }
        Ok(NavigationDiagnostic {
            stages,
            entry: self.original(entry),
            events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Graph, VGraph) {
        let mut graph = Graph::build(
            &[5., 0., 8., 0., 4., 0., 1., 0., 0., 0.],
            2,
            &BuildParams {
                degree: 2,
                metric: Metric::Euclidean,
                ..Default::default()
            },
        )
        .unwrap();
        graph.entry = 0;
        graph.layers.clear();
        graph.adj = Adjacency::new(5, 2);
        graph.adj.set(0, &[1, 2]);
        graph.adj.set(1, &[3]);
        let index = VGraph::from_graph(&graph, true).unwrap();
        (graph, index)
    }

    #[test]
    fn records_eviction_rejection_expansion_and_disconnected_recovery() {
        let (graph, mut index) = fixture();
        assert_eq!(
            index.diagnostic_adjacency(),
            vec![vec![1, 2], vec![3], vec![], vec![], vec![]]
        );
        for ef in [1, 2, 3] {
            let trace = index
                .diagnose_navigation(
                    &graph,
                    &[0., 0.],
                    1,
                    VSearchParams {
                        ef,
                        rerank: ef,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(trace.entry, 0);
            assert!(trace.events.iter().any(|event| matches!(event, NavigationEvent::Offer { id: 1, admitted, .. } if *admitted == (ef > 1))));
            assert!(
                trace
                    .events
                    .iter()
                    .any(|event| matches!(event, NavigationEvent::Expand { id: 1 }))
                    == (ef > 2)
            );
            if ef == 2 {
                assert!(trace.events.iter().any(|event| matches!(
                    event,
                    NavigationEvent::Offer {
                        id: 2,
                        evicted: Some(1),
                        ..
                    }
                )));
            }
            assert!(!trace.stages.evaluated.contains(&4));
        }
        let trace = index
            .diagnose_navigation(
                &graph,
                &[0., 0.],
                5,
                VSearchParams {
                    ef: 5,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(trace.events.iter().any(|event| matches!(
            event,
            NavigationEvent::Offer {
                id: 4,
                parent: None,
                ..
            }
        )));
        assert_eq!(trace.stages.results.len(), 5);
        for params in [
            VSearchParams {
                patience: 1,
                ..Default::default()
            },
            VSearchParams {
                neighbour_cap: 1,
                ..Default::default()
            },
            VSearchParams {
                reservoir: 1,
                ..Default::default()
            },
        ] {
            assert!(index
                .diagnose_navigation(&graph, &[0., 0.], 1, params)
                .is_err());
        }
    }
}
