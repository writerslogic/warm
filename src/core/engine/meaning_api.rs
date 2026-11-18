// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Query across local instance and remote peers in parallel.
    ///
    /// Improvements over naive collect-sort-truncate:
    /// - **BinaryHeap merge**: O(N log k) top-k selection instead of O(N log N) sort.
    /// - **ID deduplication**: same document appearing in multiple peers is counted once
    ///   (highest similarity wins).
    /// - **Partial failure tolerance**: a failed peer logs a warning and is skipped
    ///   rather than aborting the entire query.
    pub fn federated_query(
        &self,
        peer_paths: &[String],
        query_vec: &EntangledHVec,
        k: u32,
    ) -> Result<Vec<super::super::types::RetrievalResult>> {
        use rayon::prelude::*;
        use std::collections::BinaryHeap;

        let k = k as usize;

        let local_results = self.query(query_vec, k as u32)?;

        let peer_outcomes: Vec<(String, Result<Vec<super::super::types::RetrievalResult>>)> =
            peer_paths
                .par_iter()
                .map(|path| {
                    let outcome = WarmCore::new(
                        self.dimensions as u32,
                        Some(path.clone()),
                        Some(self.config.clone()),
                    )
                    .and_then(|peer| peer.query(query_vec, k as u32));
                    (path.clone(), outcome)
                })
                .collect();

        let mut seen: crate::core::hash::FastMap<String, f64> =
            crate::core::hash::FastMap::with_capacity_and_hasher(k * 2, Default::default());

        // BinaryHeap with RetrievalResult's Ord (min-heap by similarity):
        // pop() removes the lowest-similarity item, so we maintain top-k.
        let mut heap: BinaryHeap<super::super::types::RetrievalResult> =
            BinaryHeap::with_capacity(k + 1);

        let mut insert = |r: super::super::types::RetrievalResult| {
            let dominated = seen
                .get(&r.id)
                .is_some_and(|&prev_sim| prev_sim >= r.similarity);
            if dominated {
                return;
            }
            seen.insert(r.id.clone(), r.similarity);
            heap.push(r);
            if heap.len() > k {
                if let Some(evicted) = heap.pop() {
                    seen.remove(&evicted.id);
                }
            }
        };

        for r in local_results {
            insert(r);
        }

        for (path, outcome) in peer_outcomes {
            match outcome {
                Ok(results) => {
                    for r in results {
                        insert(r);
                    }
                }
                Err(e) => {
                    tracing::warn!("federated query: peer {:?} failed, skipping: {}", path, e);
                }
            }
        }

        // `RetrievalResult`'s reversed `Ord` makes ascending order highest-similarity first.
        Ok(heap.into_sorted_vec())
    }

    /// Build the shared meaning-memory context, if all required subsystems are
    /// present. Returns `None` when meaning memory is not fully initialized.
    pub(super) fn meaning_ctx(&self) -> Option<structural::MeaningContext<'_>> {
        let (atom_mem, comp_mem, tri, roles, adm) = match (
            &self.atom_memory,
            &self.composite_memory,
            &self.triple_store,
            &self.role_registry,
            &self.admission,
        ) {
            (Some(a), Some(c), Some(t), Some(r), Some(ad)) => (a, c, t, r, ad),
            _ => return None,
        };
        let mc = &self.config.meaning;
        Some(structural::MeaningContext {
            atom_memory: atom_mem,
            composite_memory: comp_mem,
            triple_store: tri,
            roles,
            admission: adm,
            beta: mc.beta,
            k: 64,
            max_iter: 3,
        })
    }

    pub fn structural_query(
        &self,
        known: &[(&str, &EntangledHVec)],
        target_role: &str,
    ) -> Vec<structural::StructuralResult> {
        let _transaction = self.mutation_gate.read();
        let Some(ctx) = self.meaning_ctx() else {
            return Vec::new();
        };
        structural::fuzzy_structural_query(&ctx, known, target_role)
    }

    /// Uses default QueryLimits; panics if a work, frontier or byte limit is exhausted.
    /// Use [`Self::multi_hop_with_limits`] to handle budget errors.
    pub fn multi_hop(&self, start: &str, relations: &[&str]) -> Vec<multi_hop::MultiHopResult> {
        let _transaction = self.mutation_gate.read();
        let (ctx, rules) = match (self.meaning_ctx(), &self.rule_store) {
            (Some(ctx), Some(rules)) => (ctx, rules),
            _ => return Vec::new(),
        };
        let mc = &self.config.meaning;
        multi_hop::multi_hop_query(start, relations, &ctx, rules, mc.max_hop_depth)
    }

    pub fn meaning_cleanup(&self, noisy: &EntangledHVec) -> Option<(String, f64)> {
        let atom_mem = self.atom_memory.as_ref()?;
        let mc = &self.config.meaning;
        let result = atom_mem.cleanup(noisy, mc.beta, 64, 3);
        if result.found {
            Some((result.id, result.confidence))
        } else {
            None
        }
    }

    pub fn declare_rule(
        &self,
        name: &str,
        input_relations: Vec<String>,
        output_relation: String,
    ) -> Result<()> {
        self.commit(&[Mutation::Rule(super::super::rules::CompositionRule {
            name: name.to_string(),
            input_relations,
            output_relation,
        })])
    }

    pub fn meaning_enabled(&self) -> bool {
        self.config.meaning.enabled
    }

    pub fn meaning_atom_count(&self) -> usize {
        self.atom_memory.as_ref().map_or(0, |m| m.count())
    }

    pub fn meaning_composite_count(&self) -> usize {
        self.composite_memory.as_ref().map_or(0, |m| m.count())
    }

    pub fn meaning_triple_count(&self) -> usize {
        self.triple_store.as_ref().map_or(0, |t| t.count())
    }

    pub fn meaning_rule_count(&self) -> usize {
        self.rule_store.as_ref().map_or(0, |r| r.count())
    }

    pub fn register_role(&mut self, name: &str, shift: usize) -> anyhow::Result<()> {
        if let Some(ref mut roles) = self.role_registry {
            roles.register(name, shift)
        } else {
            Err(anyhow::anyhow!("meaning memory not enabled"))
        }
    }
}
