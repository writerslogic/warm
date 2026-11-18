// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Everything applying a batch needs that can be computed before its exclusive window:
    /// the clones handed to the shards, each replaced document and the chunk ids it loses.
    /// Validation is the caller's ([`Self::commit`] and replay validate first).
    pub(in super::super) fn prepare_mutations<'a>(
        &self,
        mutations: &'a [Mutation],
    ) -> PreparedBatch<'a> {
        // One read lock for the batch's lookups.
        let store = self.documents.read();
        let prepared = mutations
            .iter()
            .map(|mutation| match mutation {
                Mutation::Vector { id, vector } | Mutation::Composite { id, vector } => {
                    Prepared::Vector {
                        id: id.clone(),
                        vector: vector.clone(),
                    }
                }
                Mutation::Document(document) => {
                    let previous = store.get(&document.id).cloned();
                    let stale = stale_chunks(previous.as_deref(), document);
                    // Chunks awaiting deferred encoding or stored lexical-only have no
                    // vector to index; `stale` drops any earlier vector under their ids.
                    let chunks = document
                        .chunks
                        .iter()
                        .filter(|chunk| chunk.vector_state.is_encoded())
                        .map(|chunk| (chunk.id.clone(), chunk.vector.clone()))
                        .collect();
                    Prepared::Document {
                        document: Arc::clone(document),
                        previous,
                        stale,
                        chunks,
                    }
                }
                _ => Prepared::None,
            })
            .collect();
        drop(store);
        PreparedBatch {
            mutations,
            prepared: std::cell::RefCell::new(prepared),
        }
    }

    /// Installs the documents a batch has accumulated so far in one store write.
    pub(super) fn install_documents(&self, pending: &mut Vec<Arc<StoredDocument>>) -> Result<()> {
        if !pending.is_empty() {
            self.documents
                .write()
                .insert_batch(std::mem::take(pending))?;
        }
        Ok(())
    }

    /// Installs a prepared batch into the live state (once: the prepared values are taken).
    /// A document whose stored predecessor is no longer the one the batch was prepared
    /// against (another writer replaced it in between) has its stale chunks re-diffed here.
    /// Consecutive documents install together in one store write; the run breaks at a
    /// repeated id, so the second copy diffs against the first as it would one at a time.
    pub(in super::super) fn apply_mutations(
        &self,
        batch: &PreparedBatch<'_>,
        shards: &ShardSet,
    ) -> Result<()> {
        let prepared = std::mem::take(&mut *batch.prepared.borrow_mut());
        ensure!(
            prepared.len() == batch.len(),
            "prepared batch already installed"
        );
        let mut pending: Vec<Arc<StoredDocument>> = Vec::new();
        let mut pending_ids: HashSet<String> = HashSet::new();
        let mut failure = None;
        for (mutation, prepared) in batch.mutations.iter().zip(prepared) {
            if !matches!(
                mutation,
                Mutation::Document(_)
                    | Mutation::DeleteDocument { .. }
                    | Mutation::EncodeDocument { .. }
                    | Mutation::AuditPending(_)
                    | Mutation::AuditAcknowledged { .. }
            ) {
                self.image
                    .other_state
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            let document_id = match (mutation, &prepared) {
                (Mutation::Document(_), Prepared::Document { document, .. }) => {
                    Some(document.id.clone())
                }
                _ => None,
            };
            if document_id
                .as_ref()
                .is_none_or(|id| pending_ids.contains(id))
            {
                if let Err(error) = self.install_documents(&mut pending) {
                    failure.get_or_insert(error);
                }
                pending_ids.clear();
            }
            if let Some(id) = document_id {
                pending_ids.insert(id);
            }
            if let Err(error) = self.apply_mutation(mutation, prepared, shards, &mut pending) {
                failure.get_or_insert(error);
            }
        }
        if let Err(error) = self.install_documents(&mut pending) {
            failure.get_or_insert(error);
        }
        failure.map_or(Ok(()), Err)
    }

    /// Applies one mutation of a batch. The batch is already in the journal, so a failure
    /// here must not stop the rest of it: [`Self::apply_mutations`] reports the first one
    /// once every mutation has been attempted.
    pub(super) fn apply_mutation(
        &self,
        mutation: &Mutation,
        prepared: Prepared,
        shards: &ShardSet,
        pending: &mut Vec<Arc<StoredDocument>>,
    ) -> Result<()> {
        match (mutation, prepared) {
            (Mutation::AuditPending(intent), _) => {
                self.audit_pending.write().push_back(intent.clone());
            }
            (Mutation::AuditAcknowledged { id }, _) => {
                let mut pending = self.audit_pending.write();
                ensure!(
                    pending
                        .front()
                        .is_some_and(|intent| intent.0.effect_id == Some(*id)),
                    "audit acknowledgement is out of order"
                );
                pending.pop_front();
            }
            (
                Mutation::Vector { id, vector },
                Prepared::Vector {
                    id: key,
                    vector: value,
                },
            ) => {
                self.remove_derived_from_memory(id, shards)?;
                if let Some(composites) = &self.composite_memory {
                    composites.delete(id);
                }
                if let Some(triples) = &self.triple_store {
                    triples.remove_composite(id);
                }
                shards.insert(key, value, self.dimensions)?;
                if let Some(atoms) = &self.atom_memory {
                    atoms.insert_with_vec(id, vector);
                }
            }
            (Mutation::Atom { id, vector }, _) => {
                if let Some(atoms) = &self.atom_memory {
                    atoms.insert_with_vec(id, vector);
                }
            }
            (Mutation::DeleteAtom { id }, _) => {
                self.atom_memory
                    .as_ref()
                    .expect("validated meaning configuration")
                    .delete(id);
            }
            (Mutation::DeleteComposite { id }, _) => {
                self.composite_memory
                    .as_ref()
                    .expect("validated meaning configuration")
                    .delete(id);
            }
            (Mutation::RefreshMeaningIndices {}, _) => {
                self.atom_memory
                    .as_ref()
                    .expect("validated meaning configuration")
                    .rebuild_indices();
                self.composite_memory
                    .as_ref()
                    .expect("validated meaning configuration")
                    .rebuild_indices();
            }
            (
                Mutation::Composite { id, vector },
                Prepared::Vector {
                    id: key,
                    vector: value,
                },
            ) => {
                if let Some(composites) = &self.composite_memory {
                    composites.insert(id.clone(), vector.clone());
                }
                shards.insert(key, value, self.dimensions)?;
            }
            (Mutation::StoredTriple(record), _) => {
                if let Some(triples) = &self.triple_store {
                    triples.load_triple(record.clone());
                }
            }
            (
                Mutation::Document(_),
                Prepared::Document {
                    document,
                    previous,
                    mut stale,
                    chunks,
                },
            ) => {
                let current = self.documents.read().get(&document.id).cloned();
                let unchanged = match (&previous, &current) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                };
                if !unchanged {
                    stale = stale_chunks(current.as_deref(), &document);
                }
                for chunk in &stale {
                    shards.remove(chunk, self.dimensions)?;
                }
                for (chunk, vector) in chunks {
                    shards.insert(chunk, vector, self.dimensions)?;
                }
                pending.push(document);
            }
            (Mutation::Vector { .. } | Mutation::Composite { .. } | Mutation::Document(_), _) => {
                unreachable!("mutation prepared with the wrong variant")
            }
            (
                Mutation::EncodeDocument {
                    id,
                    version,
                    fingerprint,
                    vectors,
                },
                _,
            ) => {
                self.install_deferred_encoding(id, version, *fingerprint, vectors, shards)?;
            }
            (Mutation::Derived { id, children }, _) => {
                self.derived.write().insert(id.clone(), children.clone());
            }
            (Mutation::DeleteDocument { id }, _) => {
                self.remove_document_from_memory(id, shards)?;
            }
            (Mutation::Delete { id }, _) => {
                self.remove_derived_from_memory(id, shards)?;
                self.remove_memory_id(id, shards)?;
            }
            (
                Mutation::Triplet {
                    id,
                    subject,
                    relation,
                    object,
                },
                _,
            ) => {
                self.remove_derived_from_memory(id, shards)?;
                let atoms = self
                    .atom_memory
                    .as_ref()
                    .expect("validated meaning configuration");
                let composites = self
                    .composite_memory
                    .as_ref()
                    .expect("validated meaning configuration");
                let triples = self
                    .triple_store
                    .as_ref()
                    .expect("validated meaning configuration");
                let roles = self
                    .role_registry
                    .as_ref()
                    .expect("validated meaning configuration");
                let (_, s) = atoms.get_or_insert(subject);
                let (_, r) = atoms.get_or_insert(relation);
                let (_, o) = atoms.get_or_insert(object);
                let vector = roles.compose_triple(&s, &r, &o);
                composites.insert(id.clone(), vector.clone());
                triples.remove_composite(id);
                triples.add(subject, relation, object, id);
                shards.insert(id.clone(), vector, self.dimensions)?;
            }
            (Mutation::Relation(r), _) => {
                self.graph
                    .remove(&r.source_id, &r.relation_type, &r.target_id);
                self.graph.add(r);
            }
            (
                Mutation::RemoveRelation {
                    source,
                    relation,
                    target,
                },
                _,
            ) => {
                self.graph.remove(source, relation, target);
            }
            (Mutation::RelationType(r), _) => self.graph.declare_type(r.clone()),
            (Mutation::Rule(r), _) => {
                self.rule_store
                    .as_ref()
                    .expect("validated meaning configuration")
                    .add_rule(r.clone());
            }
        }
        Ok(())
    }

    /// Installs `vectors` on the pending chunks of document `id` if it is still the revision
    /// they were computed for (same version, chunk count and content fingerprint); any other
    /// state, including an already installed encoding, leaves the store untouched. The decision
    /// reads only the live state, so a replay of the journal reaches the same one.
    pub(super) fn install_deferred_encoding(
        &self,
        id: &str,
        version: &str,
        fingerprint: u64,
        vectors: &[EntangledHVec],
        shards: &ShardSet,
    ) -> Result<()> {
        use crate::core::documents::{document_fingerprint, StoredDocument, VectorState};
        let current = {
            let store = self.documents.read();
            let Some(document) = store.get(id).cloned() else {
                return Ok(());
            };
            let applicable = document.version == version
                && document.chunks.len() == vectors.len()
                && document
                    .chunks
                    .iter()
                    .any(|chunk| chunk.vector_state == VectorState::Pending)
                && document_fingerprint(&store, &document)? == Some(fingerprint);
            if !applicable {
                return Ok(());
            }
            document
        };
        let mut encoded = StoredDocument::clone(&current);
        for (chunk, vector) in encoded.chunks.iter_mut().zip(vectors) {
            if chunk.vector_state != VectorState::Pending {
                continue;
            }
            chunk.vector = vector.clone();
            chunk.vector_state = VectorState::Encoded;
            shards.insert(chunk.id.clone(), vector.clone(), self.dimensions)?;
        }
        self.documents.write().insert_batch(vec![Arc::new(encoded)])
    }

    pub(super) fn remove_derived_from_memory(&self, id: &str, shards: &ShardSet) -> Result<()> {
        if let Some(children) = self.derived.write().remove(id) {
            for child in children {
                self.remove_memory_id(&child, shards)?;
            }
        }
        Ok(())
    }

    pub(super) fn remove_memory_id(&self, id: &str, shards: &ShardSet) -> Result<()> {
        shards.remove(id, self.dimensions)?;
        if let Some(atoms) = &self.atom_memory {
            atoms.delete(id);
        }
        if let Some(composites) = &self.composite_memory {
            composites.delete(id);
        }
        if let Some(triples) = &self.triple_store {
            triples.remove_composite(id);
        }
        Ok(())
    }
}
