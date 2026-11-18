// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Decompose a product vector into factors from domain codebooks using diffusion.
    pub fn factorize_diffusion(
        &self,
        product: &EntangledHVec,
        domain_codebooks: &[Vec<EntangledHVec>],
        max_iter: usize,
    ) -> Vec<Option<EntangledHVec>> {
        DiffusionFactorizer::factorize(&self.config.diffusion, product, domain_codebooks, max_iter)
    }

    /// Lock the connection graph, lazily opening it over its durable append-only
    /// event log at `{storage}/connection_graph.log` on first use so plastic
    /// state survives restart.
    #[cfg(feature = "experimental")]
    pub(super) fn connection_graph_lazy(
        &self,
    ) -> Result<
        crate::core::sync::MutexGuard<'_, Option<super::super::connection_graph::ConnectionGraph>>,
    > {
        let mut guard = self.connection_graph.lock();
        if guard.is_none() {
            let path = self.storage_path.join("connection_graph.log");
            let graph = super::super::connection_graph::ConnectionGraph::open(
                self.dimensions,
                super::super::connection_graph::GraphConfig::default(),
                &path,
            )?;
            *guard = Some(graph);
        }
        Ok(guard)
    }

    /// Assert a `(subject, relation, object)` edge into the experimental plastic
    /// connection graph. Opt-in and additive: independent of the sparse-binary
    /// store. Durable — the edge is appended to the graph's event log.
    #[cfg(feature = "experimental")]
    pub fn relate(&self, subject: &str, relation: &str, object: &str) -> Result<()> {
        let mut guard = self.connection_graph_lazy()?;
        guard
            .as_mut()
            .expect("initialized connection graph")
            .store(subject, relation, object)?;
        Ok(())
    }

    /// Query a relation in the plastic connection graph, returning its presence
    /// score. This is a *mutating* read (the observer effect): it reinforces the
    /// queried relation and lets the untouched background decay, all recorded as
    /// verifiable, durable events.
    #[cfg(feature = "experimental")]
    pub fn query_relation(&self, subject: &str, relation: &str, object: &str) -> Result<f64> {
        let mut guard = self.connection_graph_lazy()?;
        guard
            .as_mut()
            .expect("initialized connection graph")
            .query(subject, relation, object)
            .map_err(Into::into)
    }

    /// Dimension and phase-quantization of the experimental phasor memory. A
    /// modest dim keeps the (dim * n_phases) histogram small; retrieval works
    /// well past this substrate's load in the validation experiments.
    #[cfg(feature = "experimental")]
    pub(super) fn phase_graph_lazy(
        &self,
    ) -> crate::core::sync::MutexGuard<'_, Option<super::super::phase_graph::PhaseGraph>> {
        let mut guard = self.phase_graph.lock();
        if guard.is_none() {
            *guard = Some(super::super::phase_graph::PhaseGraph::new(2048, 256));
        }
        guard
    }

    /// Assert `(subject, relation, object)` into the experimental phasor memory,
    /// which supports relation algebra (rotation binding) and associative
    /// retrieval the sparse-binary store cannot. Opt-in and additive.
    #[cfg(feature = "experimental")]
    pub fn relate_phase(&self, subject: &str, relation: &str, object: &str) {
        self.phase_graph_lazy()
            .as_mut()
            .expect("lazily initialized")
            .relate(subject, relation, object);
    }

    /// Recover the object of `(subject, relation)` from the phasor memory.
    #[cfg(feature = "experimental")]
    pub fn phase_retrieve_object(&self, subject: &str, relation: &str) -> Option<String> {
        self.phase_graph_lazy()
            .as_ref()
            .and_then(|g| g.retrieve_object(subject, relation).map(String::from))
    }

    /// Recover the subject of `(relation, object)` — the inverse query — from the
    /// phasor memory.
    #[cfg(feature = "experimental")]
    pub fn phase_retrieve_subject(&self, relation: &str, object: &str) -> Option<String> {
        self.phase_graph_lazy()
            .as_ref()
            .and_then(|g| g.retrieve_subject(relation, object).map(String::from))
    }

    /// Multi-hop reasoning over the phasor memory: follow `relations` from
    /// `start`, retrieving at each hop, returning the final entity.
    #[cfg(feature = "experimental")]
    pub fn phase_retrieve_path(&self, start: &str, relations: &[&str]) -> Option<String> {
        self.phase_graph_lazy()
            .as_ref()
            .and_then(|g| g.retrieve_path(start, relations))
    }
}
