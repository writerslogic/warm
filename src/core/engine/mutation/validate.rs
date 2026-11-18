// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    pub(in super::super) fn validate_mutation(&self, mutation: &Mutation) -> Result<()> {
        match mutation {
            Mutation::AuditPending(intent) => {
                ensure!(
                    self.schema >= crate::core::format::AUDIT_EFFECTS_SCHEMA_VERSION,
                    "audit effects require schema 6"
                );
                intent.id()?;
            }
            Mutation::AuditAcknowledged { .. } => {
                ensure!(
                    self.schema >= crate::core::format::AUDIT_EFFECTS_SCHEMA_VERSION,
                    "audit effects require schema 6"
                );
            }
            Mutation::Vector { id, vector }
            | Mutation::Atom { id, vector }
            | Mutation::Composite { id, vector } => {
                validate_id(id)?;
                ensure!(
                    valid_vector(vector, self.dimensions),
                    "invalid vector dimensions or indices"
                );
            }
            Mutation::Document(document) => {
                validate_id(&document.id)?;
                if crate::core::vector_records::is_record(document) {
                    ensure!(
                        self.schema >= crate::core::format::VECTOR_RECORDS_SCHEMA_VERSION,
                        "vector records require store schema 7"
                    );
                    let embedding_dimensions = self
                        .config
                        .embedding_space
                        .as_ref()
                        .ok_or_else(|| {
                            anyhow::anyhow!("vector records require an embedding space")
                        })?
                        .dimensions;
                    return crate::core::vector_records::validate(
                        document,
                        self.dimensions,
                        Some(embedding_dimensions),
                    );
                }
                ensure!(
                    !document.chunks.is_empty() && document.chunks.len() <= 4096,
                    "invalid document chunk count"
                );
                for chunk in &document.chunks {
                    validate_id(&chunk.id)?;
                    // A chunk's terms match only queries analyzed as they were.
                    ensure!(
                        chunk.analyzed != self.analyzer.is_legacy(),
                        "document {:?} was analyzed {} the store's {} lexical analyzer; reopen it with the release and store.json it was written with",
                        document.id,
                        if chunk.analyzed { "by a non-legacy analyzer, not" } else { "by the legacy analyzer, not" },
                        self.analyzer
                    );
                    let chunk_limit = crate::core::format::scan::chunk_byte_limit(
                        chunk.terms.is_whole_document(),
                        document.chunks.len(),
                        u64::from(chunk.start_byte),
                        chunk.vector_state == crate::core::documents::VectorState::Lexical,
                    );
                    let Some(chunk_limit) = chunk_limit else {
                        anyhow::bail!("whole-document postings require one lexical chunk");
                    };
                    ensure!(
                        valid_vector(&chunk.vector, self.dimensions)
                            && chunk.start_byte < chunk.end_byte
                            && chunk.end_byte as usize
                                <= crate::core::documents::MAX_DOCUMENT_BYTES
                            && u64::from(chunk.end_byte - chunk.start_byte) <= chunk_limit
                            && chunk
                                .text
                                .as_ref()
                                .is_none_or(|text| text.len()
                                    == (chunk.end_byte - chunk.start_byte) as usize),
                        "document vector dimension mismatch"
                    );
                    if let Some(embedding) = &chunk.embedding {
                        let dim = self
                            .config
                            .embedding_space
                            .as_ref()
                            .map_or(0, |s| s.dimensions);
                        crate::core::documents::normalize_embedding(embedding, dim)?;
                    }
                }
            }
            Mutation::EncodeDocument {
                id,
                version,
                vectors,
                ..
            } => {
                validate_id(id)?;
                ensure!(
                    !version.is_empty()
                        && version.len() <= 1024
                        && !vectors.is_empty()
                        && vectors.len() <= 4096
                        && vectors
                            .iter()
                            .all(|vector| valid_vector(vector, self.dimensions)),
                    "invalid deferred document encoding"
                );
            }
            Mutation::Derived { id, children } => {
                validate_public_id(id)?;
                let prefix = format!("{MEANING_PREFIX}{}:{id}:", id.len());
                ensure!(
                    children.len() <= 4094
                        && children.iter().all(|child| child.starts_with(&prefix)),
                    "invalid derived meaning ownership"
                );
                for child in children {
                    validate_id(child)?;
                }
            }
            Mutation::DeleteDocument { id } => validate_id(id)?,
            Mutation::DeleteAtom { id } | Mutation::DeleteComposite { id } => {
                validate_id(id)?;
                ensure!(
                    self.schema >= crate::core::format::MEANING_MAINTENANCE_SCHEMA_VERSION,
                    "scoped meaning deletion requires schema 5; use warm-format upgrade"
                );
                ensure!(
                    self.meaning_enabled(),
                    "scoped meaning deletion requires meaning memory"
                );
            }
            Mutation::RefreshMeaningIndices {} => {
                ensure!(
                    self.schema >= crate::core::format::MEANING_MAINTENANCE_SCHEMA_VERSION,
                    "meaning index refresh requires schema 5; use warm-format upgrade"
                );
                ensure!(
                    self.meaning_enabled(),
                    "meaning index refresh requires meaning memory"
                );
            }
            Mutation::StoredTriple(t) => {
                ensure!(
                    self.meaning_enabled(),
                    "stored triple requires meaning memory"
                );
                for id in [&t.subject_id, &t.relation_id, &t.object_id, &t.composite_id] {
                    validate_id(id)?;
                }
            }
            Mutation::Delete { id } => validate_id(id)?,
            Mutation::Triplet {
                id,
                subject,
                relation,
                object,
            } => {
                for id in [id, subject, relation, object] {
                    validate_id(id)?;
                }
                ensure!(
                    self.meaning_enabled(),
                    "triplet ingestion requires meaningEnabled"
                );
            }
            Mutation::Relation(r) => {
                for id in [&r.source_id, &r.relation_type, &r.target_id] {
                    validate_id(id)?;
                }
                ensure!(
                    r.valid_from.is_finite()
                        && r.valid_to.is_finite()
                        && r.valid_from >= 0.0
                        && r.valid_to >= 0.0
                        && (r.valid_to == 0.0 || r.valid_to >= r.valid_from),
                    "invalid relation validity interval"
                );
            }
            Mutation::RemoveRelation {
                source,
                relation,
                target,
            } => {
                for id in [source, relation, target] {
                    validate_id(id)?;
                }
            }
            Mutation::RelationType(r) => validate_id(&r.name)?,
            Mutation::Rule(r) => {
                ensure!(
                    self.meaning_enabled(),
                    "composition rules require meaningEnabled"
                );
                validate_id(&r.name)?;
                validate_id(&r.output_relation)?;
                ensure!(
                    !r.input_relations.is_empty(),
                    "rule requires input relations"
                );
                for id in &r.input_relations {
                    validate_id(id)?;
                }
            }
        }
        Ok(())
    }
}
