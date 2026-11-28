// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Atomic mixed batches: document upserts and deletes committed as one journal frame.
//!
//! The frame carries the operations in order and is covered by one checksum, so a reopened
//! store and every read-only reader (which stops at the last checksum-valid frame) see all of
//! the batch or none of it. The frame is the existing `HMS-TXN-2`/`HMS-TXN-3` payload, which
//! already holds any mix of document and delete mutations: no new frame type, no format
//! change, and every release that reads those frames reads these.

use std::collections::HashMap;

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

use super::mutation::Mutation;
use super::WarmCore;
use crate::core::documents::{BatchTooLarge, DocumentInput, IngestOptions, MAX_BATCH_ITEMS};

/// One operation of [`WarmCore::write_batch`].
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum BatchOp {
    /// Adds or replaces a document, as [`WarmCore::memorize_document`].
    Memorize { document: DocumentInput },
    /// Removes a document, as [`WarmCore::delete_document`].
    Delete { id: String },
}

/// What one operation did, in the order of the batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum BatchResult {
    /// The document was stored as this many chunks.
    Memorize { chunks: u32 },
    /// Whether a document with the id existed when the operation applied.
    Delete { existed: bool },
}

impl WarmCore {
    /// [`Self::write_batch_with`] with default options.
    pub fn write_batch(&self, ops: Vec<BatchOp>) -> Result<Vec<BatchResult>> {
        self.write_batch_with(ops, &IngestOptions::default())
    }

    /// Applies `ops` as one unit. Every operation is validated and prepared first; if any is
    /// invalid nothing is applied. The batch is then one journal frame: it is durable before
    /// it is applied in memory (one sync for the whole batch), and a crash at any byte of the
    /// append leaves the store with all of it or none of it.
    ///
    /// Operations apply in order and the last write to an id wins, as in any write batch:
    /// delete then memorize of an id leaves the new document, memorize then delete leaves
    /// none, and memorizing an id twice leaves the second. Each operation still reports its
    /// own result.
    ///
    /// A batch holds 1..=4096 operations and its journal frame at most
    /// [`MAX_BATCH_BYTES`] encoded bytes (the document batch limits also apply); a larger one
    /// fails with [`BatchTooLarge`] naming the limit and writes nothing.
    pub fn write_batch_with(
        &self,
        ops: Vec<BatchOp>,
        options: &IngestOptions,
    ) -> Result<Vec<BatchResult>> {
        self.ensure_writable()?;
        const OPS: &str = "write batch requires 1..=4096 operations";
        ensure!(!ops.is_empty(), OPS);
        if ops.len() > MAX_BATCH_ITEMS {
            return Err(BatchTooLarge(OPS).into());
        }
        // Bound the bytes before preparing or serializing anything: ids are the only part of a
        // delete, and a document's source is bounded by the document batch limits.
        let delete_bytes: usize = ops
            .iter()
            .map(|op| match op {
                BatchOp::Delete { id } => id.len(),
                BatchOp::Memorize { .. } => 0,
            })
            .sum();
        if delete_bytes > MAX_BATCH_BYTES {
            return Err(BatchTooLarge(BYTES).into());
        }
        // Documents prepare in rounds so that no round repeats an id (a document batch needs
        // unique ids); round r holds each id's r-th memorize. The frame keeps them in op order.
        let mut rounds: Vec<Vec<(usize, DocumentInput)>> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut kinds = Vec::with_capacity(ops.len());
        for (index, op) in ops.into_iter().enumerate() {
            match op {
                BatchOp::Memorize { document } => {
                    let round = seen.entry(document.id.clone()).or_default();
                    if rounds.len() <= *round {
                        rounds.push(Vec::new());
                    }
                    rounds[*round].push((index, document));
                    *round += 1;
                    kinds.push(None);
                }
                BatchOp::Delete { id } => kinds.push(Some(id)),
            }
        }
        let mut prepared: Vec<Option<Mutation>> = (0..kinds.len()).map(|_| None).collect();
        for round in rounds {
            let (indices, inputs): (Vec<_>, Vec<_>) = round.into_iter().unzip();
            for (index, mutation) in indices
                .into_iter()
                .zip(self.prepare_documents(inputs, options)?)
            {
                prepared[index] = Some(mutation);
            }
        }
        let mut present: HashMap<String, bool> = HashMap::new();
        let mut mutations = Vec::with_capacity(kinds.len());
        let mut results = Vec::with_capacity(kinds.len());
        for (kind, prepared) in kinds.into_iter().zip(prepared) {
            match (kind, prepared) {
                (None, Some(mutation)) => {
                    let Mutation::Document(document) = &mutation else {
                        unreachable!("document preparation produced a non-document mutation");
                    };
                    results.push(BatchResult::Memorize {
                        chunks: document.chunks.len() as u32,
                    });
                    present.insert(document.id.clone(), true);
                    mutations.push(mutation);
                }
                (Some(id), None) => {
                    let existed = match present.get(&id) {
                        Some(&existed) => existed,
                        None => self.documents.read().contains_key(&id),
                    };
                    results.push(BatchResult::Delete { existed });
                    present.insert(id.clone(), false);
                    mutations.push(Mutation::DeleteDocument { id });
                }
                _ => unreachable!("every memorize operation was prepared"),
            }
        }
        let payload = self.transaction_payload(&mutations)?;
        // Both journal magics are the same length.
        if payload.len() - super::mutation::TRANSACTION_MAGIC.len() > MAX_BATCH_BYTES {
            return Err(BatchTooLarge(BYTES).into());
        }
        self.commit_serialized(&mutations, &payload)?;
        drop((mutations, payload));
        self.fold_if_over_budget();
        Ok(results)
    }
}

/// The most encoded bytes one batch may journal (32 MiB, the document batch transaction
/// limit; the journal frame itself holds up to 50 MB).
pub const MAX_BATCH_BYTES: usize = 32 * 1024 * 1024;
const BYTES: &str = "write batch exceeds 33554432 encoded bytes (32 MiB)";

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: usize, len: usize) -> String {
        format!("{index:06}{}", "x".repeat(len - 6))
    }

    fn deletes(last_len: usize) -> Vec<BatchOp> {
        let mut ops: Vec<_> = (0..MAX_BATCH_ITEMS - 1)
            .map(|i| BatchOp::Delete { id: id(i, 8190) })
            .collect();
        ops.push(BatchOp::Delete {
            id: id(MAX_BATCH_ITEMS - 1, last_len),
        });
        ops
    }

    fn encoded_len(core: &WarmCore, ops: &[BatchOp]) -> usize {
        let mutations: Vec<_> = ops
            .iter()
            .map(|op| match op {
                BatchOp::Delete { id } => Mutation::DeleteDocument { id: id.clone() },
                BatchOp::Memorize { .. } => unreachable!(),
            })
            .collect();
        core.transaction_payload(&mutations).unwrap().len()
            - crate::core::format::TRANSACTION_MAGIC.len()
    }

    /// The byte cap, not the operation count, is what stops this batch: 4096 deletes sized
    /// so the encoded frame is exactly the cap, one byte under, and one byte over.
    #[test]
    fn the_byte_cap_holds_at_limit_minus_one_limit_and_limit_plus_one() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let core = WarmCore::new(
            256,
            Some(dir.path().display().to_string()),
            Some(crate::core::WarmConfig::unencrypted()),
        )?;
        let mut len = 6;
        for step in [
            1 << 15,
            1 << 14,
            1 << 13,
            1 << 12,
            1 << 11,
            1 << 10,
            512,
            256,
            128,
            64,
            32,
            16,
            8,
            4,
            2,
            1,
        ] {
            if len + step <= u16::MAX as usize
                && encoded_len(&core, &deletes(len + step)) <= MAX_BATCH_BYTES
            {
                len += step;
            }
        }
        assert_eq!(
            encoded_len(&core, &deletes(len)),
            MAX_BATCH_BYTES,
            "calibration"
        );
        assert!(core.write_batch(deletes(len - 1))?.len() == MAX_BATCH_ITEMS);
        assert!(core.write_batch(deletes(len))?.len() == MAX_BATCH_ITEMS);
        let error = core.write_batch(deletes(len + 1)).unwrap_err();
        assert!(error.is::<BatchTooLarge>(), "{error:#}");
        assert!(error.to_string().contains("33554432"), "{error:#}");
        Ok(())
    }
}
