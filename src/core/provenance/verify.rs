// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Verify a provenance record without a ProvenanceManager instance.
///
/// Verifies the record's signatures and requires its signing identity —
/// resolved from the COSE key_id, or the record's issuer DID for a vc-only
/// record — to be present in `trust`. As with
/// [`ProvenanceManager::verify_fact_provenance`], a record whose signing key is
/// not trusted verifies as `valid == false`.
pub fn verify_record(record: &ProvenanceRecord, trust: &TrustStore) -> Result<VerificationResult> {
    let mut details = Vec::new();

    let mut signed_payload = None;
    if let Some(ref envelope) = record.cose_envelope {
        let kid = cose::extract_key_id(envelope)?;
        let pk: [u8; 32] = kid
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("COSE key_id is not 32 bytes"))?;
        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pk)
            .map_err(|e| anyhow::anyhow!("invalid public key in COSE: {e}"))?;
        let outcome = cose::verify_and_extract(&verifying_key, envelope);
        details.push(outcome_detail("cose_signature", &outcome));
        signed_payload = outcome.ok().map(|payload| (payload, verifying_key));
    }

    let mut credential = None;
    if let Some(ref vc_json) = record.vc_json {
        details.push(outcome_detail(
            "vc_proof",
            &vc::verify_credential_json(vc_json),
        ));
        match serde_json::from_str::<vc::FactCredential>(vc_json) {
            Ok(parsed) => {
                details.push(outcome_detail(
                    "vc_semantics",
                    &vc::validate_fact_credential(&parsed),
                ));
                details.push(revocation_detail(
                    &parsed,
                    trust.requires_status_lists(),
                    |url, index| trust.status_lookup(url, index),
                ));
                credential = Some(parsed);
            }
            Err(e) => details.push(VerificationDetail {
                check: "vc_parse".to_string(),
                passed: false,
                message: Some(e.to_string()),
            }),
        }
    }
    if !details.is_empty() {
        details.push(outcome_detail(
            "record_binding",
            &check_binding(
                record,
                signed_payload.as_ref().map(|(p, k)| (p.as_slice(), k)),
                credential.as_ref(),
            ),
        ));
    }

    // Trust anchoring applies to the whole record, not just its COSE envelope:
    // resolve the signing identity from the COSE key_id, falling back to the
    // record's issuer DID (so a vc-only record is anchored too), and require it
    // to be trusted. Without the fallback, a record carrying only a self-signed
    // VC would bypass the trust check entirely.
    if !details.is_empty() {
        let signer_key = record
            .cose_envelope
            .as_ref()
            .and_then(|e| cose::extract_key_id(e).ok())
            .and_then(|kid| <[u8; 32]>::try_from(kid.as_slice()).ok())
            .or_else(|| {
                record
                    .issuer_did
                    .as_deref()
                    .and_then(|d| did::ed25519_from_did_key(d).ok())
            })
            .and_then(|pk| ed25519_dalek::VerifyingKey::from_bytes(&pk).ok());
        let trusted = signer_key.as_ref().is_some_and(|k| trust.is_trusted(k));
        details.push(trust_anchor_detail(trusted));
    }

    let all_passed = details.iter().all(|d| d.passed);
    Ok(VerificationResult {
        valid: all_passed && !details.is_empty(),
        issuer_did: record.issuer_did.clone(),
        timestamp_ms: record.timestamp_ms,
        details,
    })
}

/// The `revocation_status` detail of a credential. `lookup(status list URL, index)` answers
/// whether the index is revoked, missing, or outside the known bitstring.
pub(super) fn revocation_detail(
    credential: &vc::FactCredential,
    require_list: bool,
    lookup: impl Fn(&str, u64) -> StatusLookup,
) -> VerificationDetail {
    let (passed, message) = match credential.credential_status {
        None => (true, None),
        Some(ref status)
            if status.status_type != "BitstringStatusListEntry"
                || status.status_purpose != "revocation" =>
        {
            (
                false,
                Some("unsupported credential status type or purpose".to_string()),
            )
        }
        Some(ref status) => match status.status_list_index.parse::<u64>() {
            Err(_) => (
                false,
                Some(format!(
                    "status list index {:?} is not a number",
                    status.status_list_index
                )),
            ),
            Ok(index) => match lookup(&status.status_list_credential, index) {
                StatusLookup::Value(true) => {
                    (false, Some("credential has been revoked".to_string()))
                }
                StatusLookup::Value(false) => (true, None),
                StatusLookup::OutOfRange => {
                    (false, Some("status list index is out of range".to_string()))
                }
                StatusLookup::Stale => (
                    false,
                    Some("trusted status list is stale or has no signed timestamp".to_string()),
                ),
                StatusLookup::Missing => (
                    !require_list,
                    Some(format!(
                        "no trusted status list for {}; revocation not checked",
                        status.status_list_credential
                    )),
                ),
            },
        },
    };
    VerificationDetail {
        check: "revocation_status".to_string(),
        passed,
        message,
    }
}

/// The fields of a fact claim signed by [`cose::sign_fact_claim`].
pub(super) struct FactClaim {
    pub(super) id: String,
    pub(super) content_hash: [u8; 32],
    pub(super) timestamp_ms: u64,
    pub(super) source_uri: Option<String>,
}

/// Decodes a fact claim, rejecting any other signed payload (deletions, key rotations, heads
/// and manifests): unknown or repeated keys, missing fields and wrong types are errors.
pub(super) fn parse_fact_claim(payload: &[u8]) -> Result<FactClaim> {
    let not_a_claim = || anyhow::anyhow!("signed payload is not a fact claim");
    let value: ciborium::Value = ciborium::from_reader(payload).map_err(|_| not_a_claim())?;
    let ciborium::Value::Map(entries) = value else {
        return Err(not_a_claim());
    };
    let (mut id, mut content_hash, mut timestamp_ms, mut source_uri) = (None, None, None, None);
    for (key, value) in entries {
        let ciborium::Value::Text(key) = key else {
            return Err(not_a_claim());
        };
        let duplicate = match (key.as_str(), value) {
            ("id", ciborium::Value::Text(v)) => id.replace(v).is_some(),
            ("content_hash", ciborium::Value::Bytes(v)) => {
                let hash: [u8; 32] = v.as_slice().try_into().map_err(|_| not_a_claim())?;
                content_hash.replace(hash).is_some()
            }
            ("timestamp_ms", ciborium::Value::Integer(v)) => {
                let ms = u64::try_from(v).map_err(|_| not_a_claim())?;
                timestamp_ms.replace(ms).is_some()
            }
            ("source_uri", ciborium::Value::Text(v)) => source_uri.replace(v).is_some(),
            _ => return Err(not_a_claim()),
        };
        if duplicate {
            return Err(not_a_claim());
        }
    }
    Ok(FactClaim {
        id: id.ok_or_else(not_a_claim)?,
        content_hash: content_hash.ok_or_else(not_a_claim)?,
        timestamp_ms: timestamp_ms.ok_or_else(not_a_claim)?,
        source_uri,
    })
}

/// Checks that a record's signed parts speak about this record and agree with each other.
/// Each signature on its own only proves that the key signed *some* payload, so without this a
/// valid envelope or credential signed for another fact (or a head, deletion or batch root)
/// verifies when pasted into a record.
///
/// - A fact-claim envelope must carry the record's content hash, timestamp and source.
/// - A batch envelope signs a bare 32-byte Merkle root: the record's content hash must hash up
///   to it through the record's inclusion proof, and the record must carry its credential,
///   since a bare hash alone cannot be told apart from a signed log head.
/// - The credential must name the record's content hash, source and issuer, the envelope's
///   signer and the fact id of the envelope's claim.
///
/// `ProvenanceRecord` carries no fact id, so the id is bound across envelope and credential;
/// checking it against the caller's id is up to the caller (`urn:hms:fact:<id>` in the
/// credential subject).
pub(super) fn check_binding(
    record: &ProvenanceRecord,
    envelope: Option<(&[u8], &ed25519_dalek::VerifyingKey)>,
    credential: Option<&vc::FactCredential>,
) -> Result<()> {
    let source_hash = record
        .source_hash
        .ok_or_else(|| anyhow::anyhow!("record has no content hash"))?;
    let record_uri = record.source_uri.as_deref().unwrap_or("");
    let mut claimed_id = None;
    if let Some((payload, _)) = envelope {
        if payload.len() == 32 {
            check_inclusion(
                MerkleHashing::Legacy,
                &source_hash,
                record.merkle_proof.as_deref(),
                payload,
                None,
            )?;
            if credential.is_none() {
                return Err(anyhow::anyhow!("batch record has no credential"));
            }
        } else if let Some((root, size)) = parse_batch_root_claim(payload)? {
            check_inclusion(
                MerkleHashing::Rfc6962,
                &source_hash,
                record.merkle_proof.as_deref(),
                &root,
                Some(size),
            )?;
        } else {
            let claim = parse_fact_claim(payload)?;
            if record.merkle_proof.is_some() {
                return Err(anyhow::anyhow!(
                    "record carries an inclusion proof but its envelope signs a single fact"
                ));
            }
            if claim.content_hash != source_hash {
                return Err(anyhow::anyhow!(
                    "signed content hash differs from the record's"
                ));
            }
            if claim.timestamp_ms != record.timestamp_ms {
                return Err(anyhow::anyhow!(
                    "signed timestamp differs from the record's"
                ));
            }
            // Records written without a source hold an empty one; their claims omit it.
            if claim.source_uri.as_deref().unwrap_or("") != record_uri {
                return Err(anyhow::anyhow!("signed source differs from the record's"));
            }
            claimed_id = Some(claim.id);
        }
    }
    if let Some(credential) = credential {
        let subject = &credential.credential_subject;
        if subject.content_hash != hex_encode(&source_hash) {
            return Err(anyhow::anyhow!(
                "credential content hash differs from the record's"
            ));
        }
        if subject.source_uri.as_deref().unwrap_or("") != record_uri {
            return Err(anyhow::anyhow!(
                "credential source differs from the record's"
            ));
        }
        if record
            .issuer_did
            .as_deref()
            .is_some_and(|issuer| issuer != credential.issuer)
        {
            return Err(anyhow::anyhow!(
                "credential issuer differs from the record's"
            ));
        }
        if let Some((_, signer)) = envelope {
            if did::did_key_from_ed25519(&signer.to_bytes()) != credential.issuer {
                return Err(anyhow::anyhow!(
                    "envelope and credential are signed by different keys"
                ));
            }
        }
        if let Some(id) = claimed_id {
            if subject.id != format!("urn:hms:fact:{id}") {
                return Err(anyhow::anyhow!(
                    "credential and envelope name different facts"
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn outcome_detail<T, E: std::fmt::Display>(
    check: &str,
    outcome: &Result<T, E>,
) -> VerificationDetail {
    VerificationDetail {
        check: check.to_string(),
        passed: outcome.is_ok(),
        message: outcome.as_ref().err().map(ToString::to_string),
    }
}

pub(super) fn trust_anchor_detail(trusted: bool) -> VerificationDetail {
    VerificationDetail {
        check: "trust_anchor".to_string(),
        passed: trusted,
        message: (!trusted).then(|| "signing key is not in the trust store".to_string()),
    }
}
