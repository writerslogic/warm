// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// The `HMS-TXN-2` journal payload of a transaction: the magic, then the batch as one JSON
/// array. A large batch serializes its elements in parallel and joins them with the separators
/// `serde_json` writes for a slice, so the bytes are exactly those of serializing the slice.
pub(in super::super) fn transaction_payload(mutations: &[Mutation]) -> Result<Vec<u8>> {
    let mut payload = Vec::with_capacity(TRANSACTION_MAGIC.len() + 128);
    payload.extend_from_slice(TRANSACTION_MAGIC);
    if mutations.len() < PARALLEL_SERIALIZATION {
        serde_json::to_writer(&mut payload, mutations)?;
        return Ok(payload);
    }
    let parts = mutations
        .par_iter()
        .map(serde_json::to_vec)
        .collect::<Result<Vec<Vec<u8>>, serde_json::Error>>()?;
    payload.reserve(parts.iter().map(Vec::len).sum::<usize>() + parts.len() + 1);
    payload.push(b'[');
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            payload.push(b',');
        }
        payload.extend_from_slice(part);
    }
    payload.push(b']');
    Ok(payload)
}

#[derive(Clone, Serialize, Deserialize)]
pub(in super::super) enum Mutation {
    AuditPending(crate::core::audit_effects::AuditIntent),
    AuditAcknowledged {
        id: [u8; 16],
    },
    Vector {
        id: String,
        vector: EntangledHVec,
    },
    Atom {
        id: String,
        vector: EntangledHVec,
    },
    Composite {
        id: String,
        vector: EntangledHVec,
    },
    DeleteAtom {
        id: String,
    },
    DeleteComposite {
        id: String,
    },
    RefreshMeaningIndices {},
    StoredTriple(crate::core::triple_store::TripleRecord),
    Document(Arc<crate::core::documents::StoredDocument>),
    /// The hypervectors of a document memorized with deferred encoding, one per chunk, computed
    /// later from its stored text. Applies only while the document `id` is still the revision
    /// (`version`, chunk count and content `fingerprint`) the vectors were computed for and has
    /// chunks awaiting encoding; otherwise a no-op, so replay and concurrent encoders agree.
    EncodeDocument {
        id: String,
        version: String,
        fingerprint: u64,
        vectors: Vec<EntangledHVec>,
    },
    Derived {
        id: String,
        children: Vec<String>,
    },
    DeleteDocument {
        id: String,
    },
    Delete {
        id: String,
    },
    Triplet {
        id: String,
        subject: String,
        relation: String,
        object: String,
    },
    Relation(Relation),
    RemoveRelation {
        source: String,
        relation: String,
        target: String,
    },
    RelationType(RelationType),
    Rule(CompositionRule),
}

/// A batch of mutations with what [`WarmCore::prepare_mutations`] computed for each of them.
/// Installation takes the prepared values out, so the batch is passed by reference and its
/// length stays readable afterwards.
pub(in super::super) struct PreparedBatch<'a> {
    pub(super) mutations: &'a [Mutation],
    pub(super) prepared: std::cell::RefCell<Vec<Prepared>>,
}

impl PreparedBatch<'_> {
    pub(in super::super) fn len(&self) -> usize {
        self.mutations.len()
    }

    pub(super) fn stage_text(&self, segment: &crate::core::documents::TextSegment) -> Result<()> {
        for prepared in self.prepared.borrow_mut().iter_mut() {
            if let Prepared::Document { document, .. } = prepared {
                *document = segment.append_document(Arc::clone(document))?;
            }
        }
        Ok(())
    }
}

pub(super) enum Prepared {
    None,
    Vector {
        id: String,
        vector: EntangledHVec,
    },
    Document {
        document: Arc<crate::core::documents::StoredDocument>,
        /// The stored document the batch was prepared against, if any.
        previous: Option<Arc<crate::core::documents::StoredDocument>>,
        /// Chunk ids of `previous` that `document` no longer has.
        stale: Vec<String>,
        chunks: Vec<(String, EntangledHVec)>,
    },
}

/// Chunk ids of `previous` that `document` does not carry an encoded vector for: absent from
/// it, or replaced by a chunk awaiting deferred encoding or stored lexical-only. Their shard
/// entries go, so no stale vector outlives the text it encoded.
pub(super) fn stale_chunks(
    previous: Option<&crate::core::documents::StoredDocument>,
    document: &crate::core::documents::StoredDocument,
) -> Vec<String> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let encoded: std::collections::BTreeSet<_> = document
        .chunks
        .iter()
        .filter(|chunk| chunk.vector_state.is_encoded())
        .map(|chunk| chunk.id.as_str())
        .collect();
    previous
        .chunks
        .iter()
        .filter(|chunk| !encoded.contains(chunk.id.as_str()))
        .map(|chunk| chunk.id.clone())
        .collect()
}

/// A vector of the store's dimensions with sorted, in-range indices.
pub(super) fn valid_vector(vector: &EntangledHVec, dimensions: usize) -> bool {
    vector.dim == dimensions
        && vector
            .indices()
            .last()
            .is_none_or(|&i| (i as usize) < dimensions)
        && vector.indices().windows(2).all(|p| p[0] < p[1])
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= u16::MAX as usize && !id.contains('\0'),
        "IDs must contain 1..=65535 UTF-8 bytes and no null characters"
    );
    Ok(())
}

pub(in super::super) fn validate_public_id(id: &str) -> Result<()> {
    validate_id(id)?;
    ensure!(
        !id.starts_with(MEANING_PREFIX) && !id.starts_with(crate::core::documents::CHUNK_PREFIX),
        "hms:chunk: and hms:meaning: IDs are reserved"
    );
    Ok(())
}
