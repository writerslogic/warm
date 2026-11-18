// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Returns the dimensionality of the hypervector space.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Encode text; panics on model/resource failure. Use `try_encode_text` for fallible models.
    pub fn encode_text(&self, text: &str) -> EntangledHVec {
        self.try_encode_text(text)
            .expect("text encoding failed; use try_encode_text to handle model or resource errors")
    }

    pub fn try_encode_text(&self, text: &str) -> Result<EntangledHVec> {
        #[cfg(feature = "local-embed")]
        if self.config.text_encoder == super::super::encoding::TextEncoder::LocalEmbedV1 {
            return Ok(self.encode_texts_with_embedder(&[text])?.remove(0));
        }
        Ok(self.config.text_encoder.encode(text, self.dimensions))
    }

    pub(crate) fn try_encode_texts(&self, texts: &[&str]) -> Result<Vec<EntangledHVec>> {
        use rayon::prelude::*;
        #[cfg(feature = "local-embed")]
        if self.config.text_encoder == super::super::encoding::TextEncoder::LocalEmbedV1 {
            return self.encode_texts_with_embedder(texts);
        }
        texts
            .par_iter()
            .map(|text| self.try_encode_text(text))
            .collect()
    }

    /// Compute word, sentence, syllable, and character-class counts for text.
    pub fn analyze_text(&self, text: &str) -> TextMetrics {
        TextProcessor::analyze(text)
    }

    /// Compute Flesch Reading Ease score from text metrics.
    pub fn calculate_readability(&self, metrics: &TextMetrics) -> f64 {
        TextProcessor::calculate_readability(metrics)
    }

    /// Delete a vector by ID. Returns true if it existed. Crash-safe: tombstone is persisted first.
    pub fn delete(&self, id: &str) -> Result<bool> {
        self.delete_with_reason(id, None)
    }

    pub fn delete_with_reason(
        &self,
        id: &str,
        #[cfg_attr(
            not(feature = "provenance"),
            expect(unused_variables, reason = "recorded only by provenance")
        )]
        reason: Option<&str>,
    ) -> Result<bool> {
        mutation::validate_public_id(id)?;
        anyhow::ensure!(
            !id.starts_with(super::super::documents::CHUNK_PREFIX),
            "use deleteDocument to delete document chunks"
        );
        let existed = self.shards.read().get_vector(id).is_some();
        self.commit_audited(
            vec![Mutation::Delete { id: id.to_owned() }],
            AuditOp::Delete,
            id,
        )?;
        #[cfg(feature = "provenance")]
        if let Some(ref mgr) = self.provenance {
            if self.config.provenance.auto_sign {
                super::super::error::after_commit(
                    "deletion provenance",
                    mgr.record_deletion(id, reason),
                )?;
            }
        }
        Ok(existed)
    }

    pub fn memorize_meaning(&self, id: &str, text: &str) -> Result<()> {
        self.memorize_meaning_with_source(id, text, None)
    }

    pub fn memorize_meaning_with_source(
        &self,
        id: &str,
        text: &str,
        #[cfg_attr(
            not(feature = "provenance"),
            expect(unused_variables, reason = "recorded only by provenance")
        )]
        source_uri: Option<&str>,
    ) -> Result<()> {
        mutation::validate_public_id(id)?;
        let mut mutations = vec![Mutation::Vector {
            id: id.to_owned(),
            vector: self.try_encode_text(text)?,
        }];
        let mut children = Vec::new();
        if self.config.meaning.auto_decompose {
            if let Some(decomposer) = &self.decomposer {
                for (i, unit) in decomposer.decompose(text).into_iter().enumerate() {
                    let child = format!("{}{}:{id}:{i}", mutation::MEANING_PREFIX, id.len());
                    children.push(child.clone());
                    mutations.push(Mutation::Triplet {
                        id: child,
                        subject: unit.subject,
                        relation: unit.relation,
                        object: unit.object,
                    });
                }
            }
        }
        if !children.is_empty() {
            mutations.push(Mutation::Derived {
                id: id.to_owned(),
                children,
            });
        }
        self.commit(&mutations)?;
        #[cfg(feature = "provenance")]
        if let Some(ref mgr) = self.provenance {
            if self.config.provenance.auto_sign {
                super::super::error::after_commit(
                    "fact provenance",
                    mgr.create_fact_provenance(id, text.as_bytes(), source_uri),
                )?;
            }
        }
        Ok(())
    }

    /// Store a vector with the given ID. Persists to the arena log and updates all indices.
    pub fn memorize(&self, id: String, vector: EntangledHVec) -> Result<()> {
        mutation::validate_public_id(&id)?;
        anyhow::ensure!(
            !id.starts_with(super::super::documents::CHUNK_PREFIX),
            "chunk IDs are reserved for document ingestion"
        );

        self.commit_audited(
            vec![Mutation::Vector {
                id: id.clone(),
                vector,
            }],
            AuditOp::Memorize,
            &id,
        )?;
        Ok(())
    }

    pub(super) fn maybe_auto_shard(&self, count: u64) -> Result<()> {
        let cfg = &self.config.shard;
        if !cfg.enabled
            || cfg.shard_count > 0
            || cfg.auto_threshold == 0
            || count < cfg.auto_threshold as u64
        {
            return Ok(());
        }
        let (revision, snapshot) = {
            let _transaction = self.mutation_gate.read();
            let shards = self.shards.read();
            if shards.shard_count() > 1 {
                return Ok(());
            }
            (
                self.revision.load(std::sync::atomic::Ordering::Acquire),
                shards.collect_all_patterns(),
            )
        };
        let count = (count as usize / cfg.target_shard_size).clamp(2, 1024);
        let new_set = ShardSet::Multi(ShardManager::new(count, self.dimensions));
        for (id, vector) in snapshot {
            new_set.insert(id, vector, self.dimensions)?;
        }
        let _transaction = self.mutation_gate.write();
        anyhow::ensure!(
            revision == self.revision.load(std::sync::atomic::Ordering::Acquire),
            "store changed during sharding; retry maintenance"
        );
        self.save_shard_meta(count)?;
        *self.shards.write() = new_set;
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        self.persist_indices(&self.shards.read())
    }

    /// Encode a bounded scalar value as a hypervector and memorize it.
    pub fn memorize_scalar(&self, id: String, value: f64, min: f64, max: f64) -> Result<()> {
        let vector = EntangledHVec::from_scalar(value, min, max, self.dimensions);
        self.memorize(id, vector)
    }

    /// Returns the total number of stored vectors across all shards.
    pub fn vector_count(&self) -> u64 {
        self.shards.read().count()
    }

    pub fn add_relation(&self, rel: &Relation) -> Result<()> {
        self.commit(&[Mutation::Relation(rel.clone())])
    }

    pub fn remove_relation(
        &self,
        source_id: &str,
        relation_type: &str,
        target_id: &str,
    ) -> Result<bool> {
        let existed = self
            .graph
            .outgoing(source_id, Some(relation_type), 0.0)
            .iter()
            .any(|r| r.target_id == target_id);
        self.commit(&[Mutation::RemoveRelation {
            source: source_id.into(),
            relation: relation_type.into(),
            target: target_id.into(),
        }])?;
        Ok(existed)
    }

    pub fn declare_relation_type(&self, rel_type: RelationType) -> Result<()> {
        self.commit(&[Mutation::RelationType(rel_type)])
    }

    pub fn traverse(
        &self,
        start_id: &str,
        relation_type: Option<&str>,
        max_depth: u32,
        at_time: f64,
    ) -> Vec<GraphPath> {
        let _transaction = self.mutation_gate.read();
        let shards = self.shards.read();
        self.graph
            .traverse(start_id, relation_type, max_depth, at_time, &|a, b| {
                let vec_a = shards.get_vector(a);
                let vec_b = shards.get_vector(b);
                match (vec_a, vec_b) {
                    (Some(va), Some(vb)) => va.similarity(&vb),
                    _ => 0.0,
                }
            })
    }

    pub fn outgoing_relations(
        &self,
        source_id: &str,
        relation_type: Option<&str>,
        at_time: f64,
    ) -> Vec<Relation> {
        let _transaction = self.mutation_gate.read();
        self.graph.outgoing(source_id, relation_type, at_time)
    }

    pub fn incoming_relations(
        &self,
        target_id: &str,
        relation_type: Option<&str>,
        at_time: f64,
    ) -> Vec<Relation> {
        let _transaction = self.mutation_gate.read();
        self.graph.incoming(target_id, relation_type, at_time)
    }

    pub fn relation_count(&self) -> usize {
        let _transaction = self.mutation_gate.read();
        self.graph.count()
    }
}
