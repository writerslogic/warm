// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    pub(in super::super) fn commit(&self, mutations: &[Mutation]) -> Result<()> {
        if !self.checkpoint_accepts(mutations) {
            self.hydrate_documents()?;
        }
        self.commit_with_append(mutations, |payload| self.arena_write(payload).map(|_| ()))
    }

    /// [`Self::commit`] of a batch whose journal payload the caller built with
    /// [`WarmCore::transaction_payload`] already (a document batch serializes once, where its
    /// size is checked).
    pub(in super::super) fn commit_serialized(
        &self,
        mutations: &[Mutation],
        payload: &[u8],
    ) -> Result<()> {
        debug_assert!(payload.starts_with(crate::core::format::TRANSACTION_MAGIC_PREFIX));
        if !self.checkpoint_accepts(mutations) {
            self.hydrate_documents()?;
        }
        self.validate_transaction(mutations)?;
        self.commit_payload(mutations, payload, |payload| {
            self.arena_write(payload).map(|_| ())
        })
    }

    pub(in super::super) fn commit_serialized_if_revision(
        &self,
        mutations: &[Mutation],
        payload: &[u8],
        revision: u64,
    ) -> Result<bool> {
        if !self.checkpoint_accepts(mutations) {
            self.hydrate_documents()?;
        }
        self.validate_transaction(mutations)?;
        self.commit_payload_at_revision(mutations, payload, Some(revision), |payload| {
            self.arena_write(payload).map(|_| ())
        })
    }

    pub(super) fn commit_with_append(
        &self,
        mutations: &[Mutation],
        append: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<()> {
        self.validate_transaction(mutations)?;
        let payload = self.transaction_payload(mutations)?;
        self.commit_payload(mutations, &payload, append)
    }

    /// The bounds and per-mutation checks every transaction passes before it is serialized.
    /// A large batch validates in parallel and still reports the first failure in batch
    /// order.
    pub(super) fn validate_transaction(&self, mutations: &[Mutation]) -> Result<()> {
        ensure!(
            !mutations.is_empty() && mutations.len() <= 4096,
            "transaction requires 1..=4096 operations"
        );
        if mutations.len() < PARALLEL_VALIDATION {
            return mutations
                .iter()
                .try_for_each(|mutation| self.validate_mutation(mutation));
        }
        let failures: Vec<Option<anyhow::Error>> = mutations
            .par_iter()
            .map(|mutation| self.validate_mutation(mutation).err())
            .collect();
        failures.into_iter().flatten().next().map_or(Ok(()), Err)
    }

    pub(super) fn commit_payload(
        &self,
        mutations: &[Mutation],
        payload: &[u8],
        append: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<()> {
        self.commit_payload_at_revision(mutations, payload, None, append)
            .map(|_| ())
    }

    pub(super) fn commit_payload_at_revision(
        &self,
        mutations: &[Mutation],
        payload: &[u8],
        revision: Option<u64>,
        append: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<bool> {
        self.commit_payload_checked(mutations, payload, revision, append, || Ok(()))
            .map(|result| result.is_some())
    }

    pub(in super::super) fn commit_serialized_checked<T>(
        &self,
        mutations: &[Mutation],
        payload: &[u8],
        check: impl FnOnce() -> Result<T>,
    ) -> Result<(u64, T)> {
        if !self.checkpoint_accepts(mutations) {
            self.hydrate_documents()?;
        }
        self.validate_transaction(mutations)?;
        self.commit_payload_checked(
            mutations,
            payload,
            None,
            |payload| self.arena_write(payload).map(|_| ()),
            check,
        )?
        .ok_or_else(|| anyhow::anyhow!("unconditional transaction rejected"))
    }

    pub(super) fn commit_payload_checked<T>(
        &self,
        mutations: &[Mutation],
        payload: &[u8],
        revision: Option<u64>,
        append: impl FnOnce(&[u8]) -> Result<()>,
        check: impl FnOnce() -> Result<T>,
    ) -> Result<Option<(u64, T)>> {
        // Lookups, clones and the chunk diff of every mutation are prepared while readers are
        // still served and before this writer queues; the exclusive window only installs them
        // (a document replaced by another writer in between is re-diffed there).
        let submitted = mutations;
        let mutations = &self.prepare_mutations(mutations);
        let transaction = self.mutation_gate.begin_write();
        if revision.is_some_and(|expected| expected != self.revision()) {
            return Ok(None);
        }
        let checked = check()?;
        let next_revision = self
            .revision()
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("store revision exhausted"))?;
        self.validate_audit_transitions(mutations.mutations)?;
        let segment = self.documents.read().text_segment();
        let mut staged = segment
            .as_ref()
            .map(|segment| segment.stage())
            .transpose()?;
        let prepared = segment
            .as_ref()
            .map_or(Ok(()), |segment| mutations.stage_text(segment));
        if let Err(error) = prepared.and_then(|()| append(payload)) {
            if let Some(staged) = &mut staged {
                if let Err(cleanup) = staged.rollback() {
                    return Err(
                        error.context(format!("retained-text rollback failed: {cleanup:#}"))
                    );
                }
            }
            return Err(error);
        }
        if let Some(staged) = &mut staged {
            staged.committed();
        }
        let applied = {
            let _transaction = transaction.publish();
            let shards = self.shards.write();
            let applied = self.apply_mutations(mutations, &shards);
            self.revision
                .store(next_revision, std::sync::atomic::Ordering::Release);
            applied
        };
        let cleaned = segment
            .as_ref()
            .map_or(Ok(()), |segment| segment.drain_scrubs());
        for mutation in submitted {
            if let Mutation::Vector { id, .. } | Mutation::Delete { id } = mutation {
                self.forget_dense(id);
            }
            match mutation {
                Mutation::DeleteDocument { id } => self.forget_sparse(id),
                Mutation::Document(document) => self.forget_sparse(&document.id),
                _ => {}
            }
        }
        crate::core::error::after_commit("live publication", applied)?;
        crate::core::error::after_commit("retained-text cleanup", cleaned)?;
        Ok(Some((next_revision, checked)))
    }
}
