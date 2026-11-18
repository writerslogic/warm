// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// How a batch's Merkle tree hashes its nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MerkleHashing {
    /// Batch records signed before the tree was versioned: the leaf is the content hash itself
    /// and a parent is SHA-256(left || right). A parent's two children are a 64-byte string
    /// whose SHA-256 is a node of the tree, so those 64 bytes verify as content of the batch.
    /// Their envelopes sign the bare 32-byte root.
    Legacy,
    /// RFC 6962 domain separation: a leaf is SHA-256(0x00 || content hash) and a parent is
    /// SHA-256(0x01 || left || right), so no leaf hash can equal a parent hash. Envelopes sign
    /// a [`batch_root_claim`] naming the version and the number of leaves.
    Rfc6962,
}

impl MerkleHashing {
    pub(super) fn leaf(self, content_hash: &[u8; 32]) -> [u8; 32] {
        match self {
            Self::Legacy => *content_hash,
            Self::Rfc6962 => Sha256::new()
                .chain_update([0x00])
                .chain_update(content_hash)
                .finalize()
                .into(),
        }
    }

    pub(super) fn node(self, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let hasher = match self {
            Self::Legacy => Sha256::new(),
            Self::Rfc6962 => Sha256::new().chain_update([0x01]),
        };
        hasher
            .chain_update(left)
            .chain_update(right)
            .finalize()
            .into()
    }
}

/// The `type` of a signed batch root claim.
pub(super) const BATCH_ROOT_TYPE: &str = "hms-batch-root";
/// The batch root claim version signed with [`MerkleHashing::Rfc6962`] trees.
pub(super) const BATCH_ROOT_VERSION: u64 = 2;

/// The CBOR payload a batch envelope signs: `{type, v, size, root}`, where `size` is the number
/// of leaves, which fixes the tree's shape.
pub(super) fn batch_root_claim(root: &[u8; 32], size: usize) -> Result<Vec<u8>> {
    let claim = ciborium::Value::Map(vec![
        (
            ciborium::Value::Text("type".to_string()),
            ciborium::Value::Text(BATCH_ROOT_TYPE.to_string()),
        ),
        (
            ciborium::Value::Text("v".to_string()),
            ciborium::Value::Integer(BATCH_ROOT_VERSION.into()),
        ),
        (
            ciborium::Value::Text("size".to_string()),
            ciborium::Value::Integer((size as u64).into()),
        ),
        (
            ciborium::Value::Text("root".to_string()),
            ciborium::Value::Bytes(root.to_vec()),
        ),
    ]);
    let mut payload = Vec::new();
    ciborium::into_writer(&claim, &mut payload)
        .map_err(|e| anyhow::anyhow!("CBOR encoding failed: {e}"))?;
    Ok(payload)
}

/// Decodes a [`batch_root_claim`] into its root and size, or `None` when the payload is not
/// one (so it can be tried as a fact claim). A payload typed as a batch root with any other
/// version, key or field type is an error.
pub(super) fn parse_batch_root_claim(payload: &[u8]) -> Result<Option<([u8; 32], u64)>> {
    let Ok(ciborium::Value::Map(entries)) = ciborium::from_reader(payload) else {
        return Ok(None);
    };
    let typed = entries
        .iter()
        .any(|(k, v)| k.as_text() == Some("type") && v.as_text() == Some(BATCH_ROOT_TYPE));
    if !typed {
        return Ok(None);
    }
    let malformed = || anyhow::anyhow!("malformed batch root claim");
    if entries.len() != 4 {
        return Err(malformed());
    }
    let (mut version, mut size, mut root) = (None, None, None);
    for (key, value) in entries {
        match (key.as_text().ok_or_else(malformed)?, value) {
            ("type", _) => {}
            ("v", ciborium::Value::Integer(v)) => version = u64::try_from(v).ok(),
            ("size", ciborium::Value::Integer(v)) => size = u64::try_from(v).ok(),
            ("root", ciborium::Value::Bytes(v)) => root = <[u8; 32]>::try_from(v.as_slice()).ok(),
            _ => return Err(malformed()),
        }
    }
    if version != Some(BATCH_ROOT_VERSION) {
        return Err(anyhow::anyhow!("unsupported batch root claim version"));
    }
    Ok(Some((
        root.ok_or_else(malformed)?,
        size.ok_or_else(malformed)?,
    )))
}

/// The root that a leaf node and its inclusion proof hash up to, as
/// [`ProvenanceManager::merkle_proof`] lays proofs out: each sibling flagged when it sits to
/// the left.
pub(super) fn merkle_root_from_proof(
    hashing: MerkleHashing,
    leaf_node: [u8; 32],
    proof: &[(bool, [u8; 32])],
) -> [u8; 32] {
    proof
        .iter()
        .fold(leaf_node, |node, (sibling_on_left, sibling)| {
            if *sibling_on_left {
                hashing.node(sibling, &node)
            } else {
                hashing.node(&node, sibling)
            }
        })
}

/// Checks that the content hash of a batch record hashes up to `root` through `proof`. For a
/// versioned root the proof may hold at most one sibling per level of a `size`-leaf tree.
pub(super) fn check_inclusion(
    hashing: MerkleHashing,
    content_hash: &[u8; 32],
    proof: Option<&[(bool, [u8; 32])]>,
    root: &[u8],
    size: Option<u64>,
) -> Result<()> {
    let proof = proof.ok_or_else(|| anyhow::anyhow!("batch record has no inclusion proof"))?;
    if let Some(size) = size {
        let depth = size
            .checked_next_power_of_two()
            .map_or(64, u64::trailing_zeros);
        if size == 0 || proof.len() > depth as usize {
            return Err(anyhow::anyhow!(
                "inclusion proof does not fit a tree of {size} leaves"
            ));
        }
    }
    if merkle_root_from_proof(hashing, hashing.leaf(content_hash), proof).as_slice() != root {
        return Err(anyhow::anyhow!(
            "content hash does not hash up to the signed batch root"
        ));
    }
    Ok(())
}
