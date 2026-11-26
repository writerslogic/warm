// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Best-first backward-chaining planner over causal triples.
//!
//! Starting from a goal state, finds triples where the object matches the goal
//! and recursively plans to achieve the subject. Produces an ordered list of
//! actions (triples to make true), expanding the lowest-cost subgoal first.
//!
//! Cost estimation: each action's cost is `1.0 / (1 + evidence_count)` where
//! `evidence_count` is the number of triples supporting the action.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::core::hash::{FastMap, FastSet};

use crate::core::triple_store::TripleStore;

/// A single planned action: a triple that needs to be made true.
#[derive(Clone, Debug)]
pub struct PlannedAction {
    pub subject: String,
    pub relation: String,
    pub object: String,
    /// Depth in the backward chain (0 = directly achieves goal).
    pub depth: usize,
    /// Estimated cost. Lower means more evidence supports this action.
    /// Computed as `1.0 / (1 + evidence_count)`.
    pub cost: f64,
}

/// A complete plan: an ordered sequence of actions to achieve a goal.
#[derive(Clone, Debug)]
pub struct Plan {
    pub goal: String,
    pub actions: Vec<PlannedAction>,
    /// Whether the plan reaches known facts for every leaf subgoal.
    pub complete: bool,
    /// Total cost of all actions in the plan.
    pub total_cost: f64,
}

/// A subgoal waiting to be expanded in the best-first search.
struct Subgoal {
    target: String,
    depth: usize,
    /// Cumulative cost to reach this subgoal.
    cost: f64,
}

/// Wrapper for min-heap ordering (BinaryHeap is max-heap by default).
impl Eq for Subgoal {}

impl PartialEq for Subgoal {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

impl Ord for Subgoal {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering so BinaryHeap acts as a min-heap on cost.
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for Subgoal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Backward-chaining planner over the triple store.
pub struct Planner;

impl Planner {
    /// Backward-chains from `goal`: triples with `object == goal` make their subject a
    /// subgoal, expanded lowest-cost first up to `max_depth`.
    ///
    /// The plan is complete when every leaf subgoal is already a known subject or has no
    /// further chain; otherwise the best partial plan is returned with `complete == false`.
    /// `causal_relations` restricts followed relations; empty follows all.
    pub fn backward_chain(
        triple_store: &TripleStore,
        goal: &str,
        causal_relations: &[&str],
        max_depth: usize,
    ) -> Plan {
        let mut actions: Vec<PlannedAction> = Vec::new();
        let mut distances: FastMap<String, Vec<(usize, f64)>> = FastMap::default();
        distances.insert(goal.to_string(), vec![(0, 0.0)]);
        let mut expanded: FastMap<String, std::ops::Range<usize>> = FastMap::default();
        let mut grounded = FastSet::default();

        let mut heap = BinaryHeap::new();
        heap.push(Subgoal {
            target: goal.to_string(),
            depth: 0,
            cost: 0.0,
        });

        while let Some(subgoal) = heap.pop() {
            if !distances[&subgoal.target].contains(&(subgoal.depth, subgoal.cost))
                || subgoal.depth >= max_depth
            {
                continue;
            }

            let candidates = triple_store.query(None, None, Some(&subgoal.target));
            let filtered: Vec<_> = if causal_relations.is_empty() {
                candidates
            } else {
                candidates
                    .into_iter()
                    .filter(|t| causal_relations.contains(&t.relation_id.as_str()))
                    .collect()
            };

            if filtered.is_empty() {
                if triple_store.has(Some(&subgoal.target), None, None) {
                    grounded.insert(subgoal.target.clone());
                }
                continue;
            }

            let evidence_count = filtered.len();
            let action_cost = 1.0 / (1.0 + evidence_count as f64);

            if let Some(range) = expanded.get(&subgoal.target) {
                for action in &mut actions[range.clone()] {
                    action.depth = action.depth.min(subgoal.depth);
                }
            } else {
                let start = actions.len();
                actions.extend(filtered.iter().map(|t| PlannedAction {
                    subject: t.subject_id.clone(),
                    relation: t.relation_id.clone(),
                    object: t.object_id.clone(),
                    depth: subgoal.depth,
                    cost: action_cost,
                }));
                expanded.insert(subgoal.target.clone(), start..actions.len());
            }
            for t in &filtered {
                let depth = subgoal.depth + 1;
                let cost = subgoal.cost + action_cost;
                let frontier = distances.entry(t.subject_id.clone()).or_default();
                // A cheaper but deeper route cannot dominate a shallower route under a depth limit.
                if frontier.iter().any(|&(d, c)| d <= depth && c <= cost) {
                    continue;
                }
                frontier.retain(|&(d, c)| !(depth <= d && cost <= c));
                frontier.push((depth, cost));
                heap.push(Subgoal {
                    target: t.subject_id.clone(),
                    depth,
                    cost,
                });
            }
        }
        let all_complete = distances
            .keys()
            .all(|target| expanded.contains_key(target) || grounded.contains(target));

        actions.sort_by_key(|b| std::cmp::Reverse(b.depth));

        let total_cost: f64 = actions.iter().map(|a| a.cost).sum();

        Plan {
            goal: goal.to_string(),
            actions,
            complete: all_complete,
            total_cost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shallower_route_reopens_a_subgoal_at_the_depth_boundary() {
        let store = TripleStore::new();
        store.add("a", "causes", "goal", "g1");
        store.add("b", "causes", "goal", "g2");
        for i in 0..9 {
            store.add("x", "causes", "a", &format!("a{i}"));
            store.add("shared", "causes", "x", &format!("x{i}"));
        }
        store.add("shared", "causes", "b", "b1");
        store.add("leaf", "causes", "shared", "leaf");
        for limit in [3, 4, 5] {
            let plan = Planner::backward_chain(&store, "goal", &["causes"], limit);
            let leaf: Vec<_> = plan
                .actions
                .iter()
                .filter(|a| a.subject == "leaf")
                .collect();
            assert_eq!(leaf.len(), 1);
            assert_eq!(leaf[0].depth, 2);
            assert_eq!(plan.complete, limit >= 4);
        }
    }

    #[test]
    fn test_simple_backward_chain() {
        let store = TripleStore::new();
        store.add("action_a", "causes", "state_b", "c1");
        store.add("state_b", "causes", "goal_c", "c2");

        let plan = Planner::backward_chain(&store, "goal_c", &["causes"], 5);
        assert!(!plan.actions.is_empty());
        assert_eq!(plan.actions[0].subject, "action_a");
        assert_eq!(plan.actions[0].object, "state_b");
        assert_eq!(plan.actions[1].subject, "state_b");
        assert_eq!(plan.actions[1].object, "goal_c");
    }

    #[test]
    fn test_no_causal_chain() {
        let unrelated = TripleStore::new();
        unrelated.add("x", "likes", "y", "c1");
        let empty = TripleStore::new();
        for (store, relations) in [(&unrelated, &["causes"][..]), (&empty, &[][..])] {
            let plan = Planner::backward_chain(store, "goal", relations, 5);
            assert!(plan.actions.is_empty());
            assert!(!plan.complete);
        }
    }

    #[test]
    fn test_cycle_prevention() {
        let store = TripleStore::new();
        store.add("a", "causes", "b", "c1");
        store.add("b", "causes", "a", "c2");

        let plan = Planner::backward_chain(&store, "a", &["causes"], 10);
        assert!(plan.actions.len() <= 4);
    }

    #[test]
    fn test_all_relations() {
        let store = TripleStore::new();
        store.add("heat", "produces", "steam", "c1");
        store.add("steam", "enables", "power", "c2");

        let plan = Planner::backward_chain(&store, "power", &[], 5);
        assert_eq!(plan.actions.len(), 2);
    }

    #[test]
    fn test_cost_estimation() {
        let store = TripleStore::new();
        store.add("x", "causes", "goal", "c1");
        store.add("y", "causes", "goal", "c2");

        let plan = Planner::backward_chain(&store, "goal", &["causes"], 5);
        assert_eq!(plan.actions.len(), 2);

        let expected_cost = 1.0 / 3.0;
        for action in &plan.actions {
            assert!(
                (action.cost - expected_cost).abs() < 1e-9,
                "expected cost {expected_cost}, got {}",
                action.cost
            );
        }

        let expected_total = 2.0 * expected_cost;
        assert!(
            (plan.total_cost - expected_total).abs() < 1e-9,
            "expected total_cost {expected_total}, got {}",
            plan.total_cost
        );

        let store2 = TripleStore::new();
        store2.add("z", "causes", "goal2", "c3");
        let plan2 = Planner::backward_chain(&store2, "goal2", &["causes"], 5);
        assert_eq!(plan2.actions.len(), 1);
        let single_cost = 1.0 / 2.0;
        assert!(
            (plan2.actions[0].cost - single_cost).abs() < 1e-9,
            "expected cost {single_cost}, got {}",
            plan2.actions[0].cost
        );
    }

    #[test]
    fn test_best_first_ordering() {
        let store = TripleStore::new();
        store.add("step1a", "causes", "mid", "c1");
        store.add("alt1", "causes", "goal", "c2");
        store.add("alt2", "causes", "goal", "c3");
        store.add("mid", "causes", "goal", "c4");

        store.add("step1b", "causes", "side", "c5");
        store.add("side", "causes", "goal", "c6");

        let plan = Planner::backward_chain(&store, "goal", &["causes"], 5);

        let depth0: Vec<_> = plan.actions.iter().filter(|a| a.depth == 0).collect();
        let depth1: Vec<_> = plan.actions.iter().filter(|a| a.depth == 1).collect();

        assert!(!depth0.is_empty(), "should have depth-0 actions");

        let expected_d0_cost = 1.0 / 5.0;
        for a in &depth0 {
            assert!(
                (a.cost - expected_d0_cost).abs() < 1e-9,
                "depth-0 cost should be {expected_d0_cost}, got {}",
                a.cost
            );
        }

        assert!(!depth1.is_empty(), "should have depth-1 actions");
        for a in &depth1 {
            assert!(
                a.cost > expected_d0_cost,
                "depth-1 cost {} should exceed depth-0 cost {expected_d0_cost}",
                a.cost
            );
        }

        let sum: f64 = plan.actions.iter().map(|a| a.cost).sum();
        assert!(
            (plan.total_cost - sum).abs() < 1e-9,
            "total_cost {} should equal sum of action costs {sum}",
            plan.total_cost
        );
    }
}
