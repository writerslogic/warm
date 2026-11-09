// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Directed reachability of the bottom layer and a data-only repair for sink components.
//!
//! A strongly connected component with no edge leaving it is a sink: a search that enters
//! it can never leave, so true neighbours outside it are unreachable from that beam.
//! [`Graph::add_escape_edges`] gives each small sink one edge to the nearest row of the
//! largest component. It uses no queries and no neighbour truth, keeps every existing edge,
//! and only fills a slot the build left free, so fixed-width encodings keep their size.

use super::Graph;
use crate::core::error::WarmError;
use rayon::prelude::*;

/// Members of a sink tried as the source of its escape edge, in id order; also the nearest
/// reachable vertices tried as the source of an in-edge.
const SOURCE_TRIES: usize = 8;
/// Sinks larger than this are left alone: they are clusters, not stragglers.
const MAX_SINK: usize = 4096;
const UNREACHABLE_SAMPLE: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct ConnectivityStats {
    pub vertices: usize,
    pub components: usize,
    pub largest_component: usize,
    /// Components with no edge leaving them, other than the largest.
    pub sink_components: usize,
    pub sink_vertices: usize,
    /// Vertices the bottom layer cannot reach from the entry and the upper-layer vertices.
    pub unreachable_from_entry: usize,
    pub zero_in_degree: usize,
    /// Escape edges added by [`Graph::add_escape_edges`] (zero for a plain report).
    pub escape_edges: usize,
    /// Sinks left because no member had a free slot or the sink exceeded the size limit.
    pub skipped_sinks: usize,
    /// In-edges added by [`Graph::add_reach_edges`] to vertices the entry could not reach.
    pub reach_edges: usize,
    /// Of those, edges that replaced the farthest out-edge of a full source list.
    pub reach_swaps: usize,
    /// The first few vertices the entry cannot reach, in id order.
    pub unreachable_sample: Vec<u32>,
}

struct Components {
    label: Vec<u32>,
    sizes: Vec<usize>,
}

impl Graph {
    /// Component structure and reachability of the bottom layer.
    pub fn connectivity(&self) -> ConnectivityStats {
        let components = self.strong_components();
        self.stats_from(&components)
    }

    /// Adds one edge from each small sink component to the nearest row of the largest
    /// component, then reports the structure after the repair. Deterministic.
    pub fn add_escape_edges(&mut self) -> Result<ConnectivityStats, WarmError> {
        let components = self.strong_components();
        let main = components
            .sizes
            .iter()
            .enumerate()
            .max_by_key(|&(c, &s)| (s, std::cmp::Reverse(c)))
            .map(|(c, _)| c as u32)
            .unwrap_or(0);
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); components.sizes.len()];
        for (v, &c) in components.label.iter().enumerate() {
            if c != main && components.sizes[c as usize] <= MAX_SINK {
                members[c as usize].push(v as u32);
            }
        }
        let sinks = self.sink_labels(&components, main);
        let main_ids: Vec<u32> = (0..self.n as u32)
            .filter(|&v| components.label[v as usize] == main)
            .collect();
        let mut added = 0usize;
        let mut skipped = 0usize;
        let mut plan: Vec<(u32, u32)> = Vec::new();
        for c in sinks {
            let group = &members[c as usize];
            let free: Vec<u32> = group
                .iter()
                .copied()
                .filter(|&v| self.adj.get(v as usize).len() < self.degree)
                .take(SOURCE_TRIES)
                .collect();
            if group.is_empty() || free.is_empty() {
                skipped += 1;
                continue;
            }
            let best = free
                .par_iter()
                .map(|&s| {
                    let row = &self.vectors[s as usize * self.dim..(s as usize + 1) * self.dim];
                    main_ids
                        .iter()
                        .map(|&t| (self.query_distance(row, t), t))
                        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                        .map(|(d, t)| (d, s, t))
                })
                .flatten()
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            match best {
                Some((_, s, t)) => plan.push((s, t)),
                None => skipped += 1,
            }
        }
        for (s, t) in plan {
            let mut list = self.adj.get(s as usize).to_vec();
            if !list.contains(&t) {
                list.push(t);
                self.adj.set(s as usize, &list);
                added += 1;
            }
        }
        self.repaired = true;
        let mut stats = self.connectivity();
        stats.escape_edges = added;
        stats.skipped_sinks = skipped;
        Ok(stats)
    }

    /// Components other than `main` with no edge into a different component.
    fn sink_labels(&self, components: &Components, main: u32) -> Vec<u32> {
        let mut leaves = vec![false; components.sizes.len()];
        for (v, list) in self.adj.iter().enumerate() {
            let c = components.label[v];
            if list.iter().any(|&u| components.label[u as usize] != c) {
                leaves[c as usize] = true;
            }
        }
        (0..components.sizes.len() as u32)
            .filter(|&c| c != main && !leaves[c as usize])
            .collect()
    }

    fn stats_from(&self, components: &Components) -> ConnectivityStats {
        let (main, largest) = components
            .sizes
            .iter()
            .enumerate()
            .max_by_key(|&(c, &s)| (s, std::cmp::Reverse(c)))
            .map(|(c, &s)| (c as u32, s))
            .unwrap_or((0, 0));
        let sinks = self.sink_labels(components, main);
        let mut in_degree = vec![0u32; self.n];
        for list in self.adj.iter() {
            for &u in list {
                in_degree[u as usize] += 1;
            }
        }
        let mut seen = vec![false; self.n];
        let seeds: Vec<u32> = std::iter::once(self.entry)
            .chain(self.upper_ids.iter().copied())
            .filter(|&v| (v as usize) < self.n)
            .collect();
        let reached = self.mark_reachable(&seeds, &mut seen);
        ConnectivityStats {
            vertices: self.n,
            components: components.sizes.len(),
            largest_component: largest,
            sink_components: sinks.len(),
            sink_vertices: sinks.iter().map(|&c| components.sizes[c as usize]).sum(),
            unreachable_from_entry: self.n - reached,
            zero_in_degree: in_degree.iter().filter(|&&d| d == 0).count(),
            escape_edges: 0,
            skipped_sinks: 0,
            reach_edges: 0,
            reach_swaps: 0,
            unreachable_sample: (0..self.n as u32)
                .filter(|&v| !seen[v as usize])
                .take(UNREACHABLE_SAMPLE)
                .collect(),
        }
    }

    /// Marks everything reachable from `seeds` in `seen` and returns how many were new.
    fn mark_reachable(&self, seeds: &[u32], seen: &mut [bool]) -> usize {
        let mut stack: Vec<u32> = Vec::new();
        let mut fresh = 0usize;
        for &v in seeds {
            if !seen[v as usize] {
                seen[v as usize] = true;
                fresh += 1;
                stack.push(v);
            }
        }
        while let Some(v) = stack.pop() {
            for &u in self.adj.get(v as usize) {
                if !seen[u as usize] {
                    seen[u as usize] = true;
                    fresh += 1;
                    stack.push(u);
                }
            }
        }
        fresh
    }

    /// Gives every vertex the entry cannot reach an in-edge from the nearest reachable
    /// vertex, then reports the structure after the repair. A source with no free slot
    /// swaps its farthest out-edge whose target keeps another in-edge, so no vertex is
    /// orphaned by the swap and the encoding keeps its size. Uses no queries or truth.
    pub fn add_reach_edges(&mut self) -> Result<ConnectivityStats, WarmError> {
        let mut seen = vec![false; self.n];
        let seeds: Vec<u32> = std::iter::once(self.entry)
            .chain(self.upper_ids.iter().copied())
            .filter(|&v| (v as usize) < self.n)
            .collect();
        self.mark_reachable(&seeds, &mut seen);
        let mut in_degree = vec![0u32; self.n];
        for list in self.adj.iter() {
            for &u in list {
                in_degree[u as usize] += 1;
            }
        }
        let orphans: Vec<u32> = (0..self.n as u32).filter(|&v| !seen[v as usize]).collect();
        let (mut added, mut swaps) = (0usize, 0usize);
        for v in orphans {
            if seen[v as usize] {
                continue;
            }
            let row = self.vectors[v as usize * self.dim..(v as usize + 1) * self.dim].to_vec();
            let mut near: Vec<(f32, u32)> = (0..self.n as u32)
                .into_par_iter()
                .filter(|&u| seen[u as usize])
                .map(|u| (self.query_distance(&row, u), u))
                .collect();
            let keep = SOURCE_TRIES.min(near.len());
            if keep == 0 {
                continue;
            }
            near.select_nth_unstable_by(keep - 1, |a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            near.truncate(keep);
            near.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let free = near
                .iter()
                .map(|&(_, u)| u)
                .find(|&u| self.adj.get(u as usize).len() < self.degree);
            if let Some(u) = free {
                let mut list = self.adj.get(u as usize).to_vec();
                list.push(v);
                self.adj.set(u as usize, &list);
                in_degree[v as usize] += 1;
                added += 1;
            } else {
                let mut done = false;
                for &(_, u) in &near {
                    let mut list = self.adj.get(u as usize).to_vec();
                    let urow = &self.vectors[u as usize * self.dim..(u as usize + 1) * self.dim];
                    let victim = (0..list.len())
                        .filter(|&i| in_degree[list[i] as usize] >= 2)
                        .max_by(|&a, &b| {
                            self.query_distance(urow, list[a])
                                .total_cmp(&self.query_distance(urow, list[b]))
                                .then(a.cmp(&b))
                        });
                    if let Some(i) = victim {
                        in_degree[list[i] as usize] -= 1;
                        list[i] = v;
                        in_degree[v as usize] += 1;
                        self.adj.set(u as usize, &list);
                        added += 1;
                        swaps += 1;
                        done = true;
                        break;
                    }
                }
                if !done {
                    continue;
                }
            }
            self.mark_reachable(&[v], &mut seen);
        }
        self.repaired = true;
        let mut stats = self.connectivity();
        stats.reach_edges = added;
        stats.reach_swaps = swaps;
        Ok(stats)
    }

    /// Iterative Tarjan over the bottom layer.
    fn strong_components(&self) -> Components {
        const UNSEEN: u32 = u32::MAX;
        let n = self.n;
        let mut index = vec![UNSEEN; n];
        let mut low = vec![0u32; n];
        let mut on_stack = vec![false; n];
        let mut label = vec![UNSEEN; n];
        let mut sizes: Vec<usize> = Vec::new();
        let mut stack: Vec<u32> = Vec::new();
        let mut call: Vec<(u32, usize)> = Vec::new();
        let mut next = 0u32;
        for root in 0..n as u32 {
            if index[root as usize] != UNSEEN {
                continue;
            }
            call.push((root, 0));
            while let Some(&mut (v, ref mut child)) = call.last_mut() {
                let vi = v as usize;
                if *child == 0 && index[vi] == UNSEEN {
                    index[vi] = next;
                    low[vi] = next;
                    next += 1;
                    stack.push(v);
                    on_stack[vi] = true;
                }
                let list = self.adj.get(vi);
                if *child < list.len() {
                    let u = list[*child] as usize;
                    *child += 1;
                    if index[u] == UNSEEN {
                        call.push((u as u32, 0));
                    } else if on_stack[u] {
                        low[vi] = low[vi].min(index[u]);
                    }
                    continue;
                }
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent as usize] = low[parent as usize].min(low[vi]);
                }
                if low[vi] == index[vi] {
                    let id = sizes.len() as u32;
                    let mut size = 0usize;
                    loop {
                        let w = stack.pop().expect("component root is on the stack");
                        on_stack[w as usize] = false;
                        label[w as usize] = id;
                        size += 1;
                        if w == v {
                            break;
                        }
                    }
                    sizes.push(size);
                }
            }
        }
        Components { label, sizes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::qgraph::{BuildParams, Metric};

    fn ring_pair(n_each: usize) -> (Vec<f32>, usize) {
        // Two tight clusters on a line so they are far apart; degree small enough that the
        // build keeps them as separate neighbourhoods.
        let mut data = Vec::new();
        for i in 0..n_each {
            data.extend_from_slice(&[1.0, 0.01 * i as f32]);
        }
        // The sink is the smaller cluster, so the largest component is unambiguous.
        for i in 0..n_each * 3 / 4 {
            data.extend_from_slice(&[0.01 * i as f32, 1.0]);
        }
        (data, 2)
    }

    #[test]
    fn escape_edges_remove_every_small_sink_and_keep_edges() {
        let (data, dim) = ring_pair(40);
        let params = BuildParams {
            degree: 8,
            build_ef: 32,
            metric: Metric::Angular,
            ..BuildParams::default()
        };
        let mut graph = Graph::build(&data, dim, &params).unwrap();
        // Cut every edge leaving the second cluster so it becomes a sink.
        for v in 40..70 {
            let kept: Vec<u32> = graph
                .adj
                .get(v)
                .iter()
                .copied()
                .filter(|&u| u >= 40)
                .collect();
            graph.adj.set(v, &kept);
        }
        for v in 0..40 {
            let mut list = graph.adj.get(v).to_vec();
            list.truncate(graph.degree - 1);
            if !list.iter().any(|&u| u >= 40) {
                list.push(40);
            }
            graph.adj.set(v, &list);
        }
        let before = graph.connectivity();
        assert!(before.sink_components >= 1, "{before:?}");
        let old: Vec<Vec<u32>> = graph.adj.iter().map(<[u32]>::to_vec).collect();
        let after = graph.add_escape_edges().unwrap();
        assert_eq!(after.sink_components, 0, "{after:?}");
        assert_eq!(after.skipped_sinks, 0);
        assert!(after.escape_edges >= 1);
        for (v, list) in old.iter().enumerate() {
            assert!(list.iter().all(|u| graph.adj.get(v).contains(u)));
        }
        let again = graph.add_escape_edges().unwrap();
        assert_eq!(again.escape_edges, 0);
    }

    #[test]
    fn reach_edges_connect_orphans_without_orphaning_others() {
        let (data, dim) = ring_pair(40);
        let params = BuildParams {
            degree: 8,
            build_ef: 32,
            metric: Metric::Angular,
            ..BuildParams::default()
        };
        let mut graph = Graph::build(&data, dim, &params).unwrap();
        // Drop every edge into vertices 10 and 11, and fill the lists that pointed at them.
        for v in 0..graph.n {
            let kept: Vec<u32> = graph
                .adj
                .get(v)
                .iter()
                .copied()
                .filter(|&u| u != 10 && u != 11)
                .collect();
            graph.adj.set(v, &kept);
        }
        let before = graph.connectivity();
        assert!(before.unreachable_from_entry >= 2, "{before:?}");
        let after = graph.add_reach_edges().unwrap();
        assert_eq!(after.unreachable_from_entry, 0, "{after:?}");
        assert!(after.reach_edges >= 2);
        assert!(graph.adj.iter().all(|l| l.len() <= graph.degree));
        assert_eq!(graph.add_reach_edges().unwrap().reach_edges, 0);
    }
}
