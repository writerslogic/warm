// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

fn manager() -> (TempDir, ProvenanceManager) {
    let dir = tempdir().unwrap();
    let mgr = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    (dir, mgr)
}

fn assert_rejected(mgr: &ProvenanceManager, record: &ProvenanceRecord, what: &str) {
    let trust = mgr.self_trust();
    let own = mgr.verify_fact_provenance(record, &trust).unwrap();
    assert!(!own.valid, "manager accepted {what}: {:?}", own.details);
    let free = verify_record(record, &trust).unwrap();
    assert!(
        !free.valid,
        "verify_record accepted {what}: {:?}",
        free.details
    );
}

fn assert_accepted(mgr: &ProvenanceManager, record: &ProvenanceRecord, what: &str) {
    let trust = mgr.self_trust();
    let own = mgr.verify_fact_provenance(record, &trust).unwrap();
    assert!(own.valid, "manager rejected {what}: {:?}", own.details);
    let free = verify_record(record, &trust).unwrap();
    assert!(
        free.valid,
        "verify_record rejected {what}: {:?}",
        free.details
    );
}

#[test]
fn envelopes_signed_for_something_else_do_not_verify() {
    let (dir, mgr) = manager();
    let a = mgr.create_fact_provenance("a", b"alpha", None).unwrap();
    let b = mgr
        .create_fact_provenance("b", b"beta", Some("https://example.com/b"))
        .unwrap();
    assert_accepted(&mgr, &a, "an untouched fact record");
    assert_accepted(&mgr, &b, "an untouched fact record with a source");

    let deletion = mgr.record_deletion("gone", None).unwrap();
    let head = std::fs::read(dir.path().join("provenance_log.head")).unwrap();
    let items: Vec<(&str, &[u8], Option<&str>)> = vec![("x", b"x", None), ("y", b"y", None)];
    let batch = mgr.create_batch_provenance(&items).unwrap();

    let foreign = [
        ("another fact's envelope", b.cose_envelope.clone().unwrap()),
        ("a deletion envelope", deletion.cose_envelope.unwrap()),
        ("the signed log head", head),
        (
            "a batch root envelope",
            batch[0].cose_envelope.clone().unwrap(),
        ),
    ];
    for (what, envelope) in foreign {
        let mut forged = a.clone();
        forged.cose_envelope = Some(envelope);
        assert_rejected(&mgr, &forged, what);
    }

    let mut forged = a.clone();
    forged.source_hash = Some([0x55; 32]);
    assert_rejected(&mgr, &forged, "a record whose content hash was replaced");

    let mut forged = a.clone();
    forged.vc_json = b.vc_json.clone();
    assert_rejected(&mgr, &forged, "another fact's credential");

    let mut forged = a.clone();
    forged.timestamp_ms += 1;
    assert_rejected(&mgr, &forged, "a record whose timestamp was changed");

    let mut forged = b.clone();
    forged.source_uri = Some("https://example.com/other".to_string());
    assert_rejected(&mgr, &forged, "a record whose source was changed");
}

#[test]
fn manifests_verify_only_with_their_own_signed_manifest() {
    let (dir, mgr) = manager();
    let trust = mgr.self_trust();
    let manifest = mgr
        .create_store_manifest("s1", b"store one", 1, 1024, None)
        .unwrap();
    let other = mgr
        .create_store_manifest("s2", b"store two", 1, 1024, None)
        .unwrap();
    assert!(
        mgr.verify_store_manifest(&manifest, None, &trust)
            .unwrap()
            .valid
    );
    assert!(
        mgr.verify_store_manifest(&manifest, Some(b"store one"), &trust)
            .unwrap()
            .valid
    );

    let fact = mgr.create_fact_provenance("f", b"fact", None).unwrap();
    let head = std::fs::read(dir.path().join("provenance_log.head")).unwrap();
    let foreign = [
        (
            "another store's manifest envelope",
            other.cose_envelope.clone().unwrap(),
        ),
        ("a fact envelope", fact.cose_envelope.unwrap()),
        ("the signed log head", head),
    ];
    for (what, envelope) in foreign {
        let mut forged = manifest.clone();
        forged.cose_envelope = Some(envelope);
        let result = mgr.verify_store_manifest(&forged, None, &trust).unwrap();
        assert!(!result.valid, "accepted {what}: {:?}", result.details);
    }

    let mut forged = manifest.clone();
    forged.store_hash = other.store_hash;
    assert!(
        !mgr.verify_store_manifest(&forged, None, &trust)
            .unwrap()
            .valid
    );
}

#[test]
fn batch_records_verify_only_with_their_own_inclusion_proof() {
    let (_dir, mgr) = manager();
    let items: Vec<(&str, &[u8], Option<&str>)> = vec![
        ("b1", b"alpha", None),
        ("b2", b"beta", None),
        ("b3", b"gamma", Some("https://example.com")),
        ("b4", b"delta", None),
        ("b5", b"epsilon", None),
    ];
    let records = mgr.create_batch_provenance(&items).unwrap();
    for r in &records {
        assert_accepted(&mgr, r, "an untouched batch record");
    }

    let mut forged = records[0].clone();
    forged.merkle_proof = records[1].merkle_proof.clone();
    assert_rejected(&mgr, &forged, "a batch record with a sibling's proof");

    let mut forged = records[0].clone();
    forged.merkle_proof = None;
    assert_rejected(&mgr, &forged, "a batch record without a proof");

    let mut forged = records[0].clone();
    forged.merkle_proof.as_mut().unwrap().pop();
    assert_rejected(&mgr, &forged, "a batch record with a truncated proof");

    let mut forged = records[2].clone();
    forged.source_hash = Some(Sha256::digest(b"not in the batch").into());
    assert_rejected(
        &mgr,
        &forged,
        "a batch record whose content is not in the tree",
    );

    // A versioned root cannot be mistaken for anything else the key signs, so the envelope
    // and proof alone prove the content is in the batch.
    let mut cose_only = records[0].clone();
    cose_only.vc_json = None;
    assert_accepted(
        &mgr,
        &cose_only,
        "a batch record with only its envelope and proof",
    );
}

#[test]
fn verify_record_honors_the_trusted_status_list() {
    let (_dir, mgr) = manager();
    let record = mgr.create_fact_provenance("r", b"revocable", None).unwrap();
    assert!(verify_record(&record, &mgr.self_trust()).unwrap().valid);

    let credential: vc::FactCredential =
        serde_json::from_str(record.vc_json.as_ref().unwrap()).unwrap();
    let index: u64 = credential
        .credential_status
        .unwrap()
        .status_list_index
        .parse()
        .unwrap();
    mgr.revoke_credential(index).unwrap();

    let result = verify_record(&record, &mgr.self_trust()).unwrap();
    assert!(!result.valid);
    assert!(result
        .details
        .iter()
        .any(|d| d.check == "revocation_status" && !d.passed));
}

#[test]
fn published_status_list_covers_issued_credentials_and_requires_trust() {
    let key = SigningKey::from_bytes(&[31u8; 32]);
    let mgr = ProvenanceManager::from_signing_key(key)
        .with_status_list_url("https://example.com/credentials/status/1".into())
        .unwrap();
    let record = mgr
        .create_fact_provenance("fact", b"revocable", None)
        .unwrap();
    let credential: vc::FactCredential =
        serde_json::from_str(record.vc_json.as_ref().unwrap()).unwrap();
    let status = credential.credential_status.unwrap();
    assert_eq!(status.status_list_credential, mgr.status_list_url);
    let index: u64 = status.status_list_index.parse().unwrap();
    mgr.revoke_credential(index).unwrap();

    let published = mgr.status_list_credential().unwrap();
    let mut untrusted = TrustStore::new();
    assert!(untrusted.trust_status_list_credential(&published).is_err());
    let mut trust = TrustStore::trusting_did(mgr.issuer_did()).unwrap();
    assert!(trust.trust_status_list_credential(&published).is_err());
    trust
        .authorize_status_list_publisher(&published.id, &mgr.signing_key.verifying_key())
        .unwrap();
    trust.trust_status_list_credential(&published).unwrap();
    assert_eq!(
        trust.is_revoked(&status.status_list_credential, index),
        Some(true)
    );
    assert!(!verify_record(&record, &trust).unwrap().valid);
}

#[test]
fn reopening_rejects_a_different_published_status_url() {
    let (dir, mgr) = manager();
    let mgr = mgr
        .with_status_list_url("https://example.com/status/1".into())
        .unwrap();
    mgr.create_fact_provenance("fact", b"content", None)
        .unwrap();
    drop(mgr);
    let reopened = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    assert!(reopened
        .with_status_list_url("https://example.com/status/2".into())
        .is_err());
    let reopened = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    assert!(reopened
        .with_status_list_url("https://example.com/status/1".into())
        .is_ok());
}

/// Without a trusted status list a credential passes its revocation check unchecked by
/// default, and fails it when the trust store requires status lists.
#[test]
fn strict_trust_store_fails_unknown_revocation_status() {
    let (_dir, mgr) = manager();
    let record = mgr.create_fact_provenance("r", b"strict", None).unwrap();
    let mut trust = TrustStore::trusting_did(mgr.issuer_did()).unwrap();
    assert!(verify_record(&record, &trust).unwrap().valid);
    trust.require_status_lists(true);
    let result = verify_record(&record, &trust).unwrap();
    assert!(!result.valid);
    assert!(result
        .details
        .iter()
        .any(|d| d.check == "revocation_status" && !d.passed));
    assert!(
        verify_record(&record, mgr.self_trust().require_status_lists(true))
            .unwrap()
            .valid
    );
}

#[test]
fn status_lookup_rejects_indices_outside_the_published_list() {
    let key = SigningKey::from_bytes(&[43u8; 32]);
    let did = did::did_key_from_ed25519(&key.verifying_key().to_bytes());
    let url = "https://example.com/status/1";
    let mut credential = vc::create_fact_credential(&did, "fact", &[0u8; 32], 1, None, None, 0)
        .with_status_list_url(url);
    let mut trust = TrustStore::new();
    trust.trust_status_list(url, vec![0; status_list::MIN_BYTES]);
    let last = (status_list::MIN_BYTES * 8 - 1) as u64;
    for (index, expected) in [(last - 1, true), (last, true), (last + 1, false)] {
        credential
            .credential_status
            .as_mut()
            .unwrap()
            .status_list_index = index.to_string();
        let result = revocation_detail(&credential, false, |url, index| {
            trust.status_lookup(url, index)
        });
        assert_eq!(result.passed, expected, "index {index}");
    }
    credential
        .credential_status
        .as_mut()
        .unwrap()
        .status_list_index = last.to_string();
    credential
        .credential_status
        .as_mut()
        .unwrap()
        .status_purpose = "suspension".into();
    assert!(
        !revocation_detail(&credential, false, |url, index| {
            trust.status_lookup(url, index)
        })
        .passed
    );
}

#[test]
fn record_verification_rejects_unsigned_credential_extensions() {
    let (_dir, mgr) = manager();
    let mut record = mgr
        .create_fact_provenance("fact", b"content", None)
        .unwrap();
    let mut document: serde_json::Value =
        serde_json::from_str(record.vc_json.as_ref().unwrap()).unwrap();
    document["credentialSubject"]["admin"] = serde_json::Value::Bool(true);
    record.vc_json = Some(serde_json::to_string(&document).unwrap());
    let result = verify_record(&record, &mgr.self_trust()).unwrap();
    assert!(!result.valid);
    assert!(result
        .details
        .iter()
        .any(|detail| detail.check == "vc_proof" && !detail.passed));
}

#[test]
fn signed_credential_semantics_and_status_snapshot_freshness_are_reported() {
    let (_dir, mgr) = manager();
    let mgr = mgr
        .with_status_list_url("https://example.com/status/1".into())
        .unwrap();
    let mut record = mgr
        .create_fact_provenance("fact", b"content", None)
        .unwrap();
    let mut credential: vc::FactCredential =
        serde_json::from_str(record.vc_json.as_ref().unwrap()).unwrap();
    credential.extra.insert(
        "validUntil".into(),
        serde_json::json!("2020-01-01T00:00:00Z"),
    );
    let credential = vc::sign_credential(&mgr.signing_key, credential).unwrap();
    record.vc_json = Some(serde_json::to_string(&credential).unwrap());
    let checked = verify_record(&record, &mgr.self_trust()).unwrap();
    assert!(checked
        .details
        .iter()
        .any(|d| d.check == "vc_proof" && d.passed));
    assert!(checked
        .details
        .iter()
        .any(|d| d.check == "vc_semantics" && !d.passed));

    let original = mgr
        .create_fact_provenance("freshness", b"content", None)
        .unwrap();
    let mut list = mgr.status_list_credential().unwrap();
    list.valid_from = "2020-01-01T00:00:00Z".into();
    list.proof = None;
    list.proof = Some(vc::sign_document(&mgr.signing_key, &list).unwrap());
    let mut trust = TrustStore::trusting_did(mgr.issuer_did()).unwrap();
    trust
        .authorize_status_list_publisher(&list.id, &mgr.signing_key.verifying_key())
        .unwrap();
    trust.trust_status_list_credential(&list).unwrap();
    trust.require_recent_status_lists(60);
    let checked = verify_record(&original, &trust).unwrap();
    assert!(checked
        .details
        .iter()
        .any(|d| d.check == "revocation_status"
            && !d.passed
            && d.message.as_deref().unwrap_or_default().contains("stale")));
}

#[test]
fn concurrent_writers_keep_one_chain_and_the_last_record() {
    let (dir, mgr) = manager();
    let mgr = Arc::new(mgr);
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let mgr = Arc::clone(&mgr);
            std::thread::spawn(move || {
                for i in 0..20 {
                    let id = format!("shared-{}", (t + i) % 4);
                    match i % 7 {
                        6 => drop(mgr.record_deletion(&id, None).unwrap()),
                        _ => drop(
                            mgr.create_fact_provenance(&id, format!("{t}/{i}").as_bytes(), None)
                                .unwrap(),
                        ),
                    }
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert!(mgr.verify_log_integrity().unwrap());
    let live: Vec<Option<u64>> = (0..4)
        .map(|k| mgr.get_record(&format!("shared-{k}")).map(|r| r.sequence))
        .collect();
    drop(Arc::try_unwrap(mgr).ok().unwrap());

    let reopened = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    assert!(reopened.verify_log_integrity().unwrap());
    let replayed: Vec<Option<u64>> = (0..4)
        .map(|k| {
            reopened
                .get_record(&format!("shared-{k}"))
                .map(|r| r.sequence)
        })
        .collect();
    assert_eq!(live, replayed);
}

#[test]
fn log_and_key_event_log_verify_right_after_rotation_and_after_reopen() {
    let dir = tempdir().unwrap();
    let mut mgr = ProvenanceManager::new(&dir.path().join("old.key"), Some(dir.path())).unwrap();
    mgr.create_fact_provenance("before", b"old key", None)
        .unwrap();
    let new_key = dir.path().join("new.key");
    mgr.rotate_key(&new_key).unwrap();
    assert!(mgr.verify_log_integrity().unwrap());
    mgr.verify_keri_log().unwrap();
    drop(mgr);

    let reopened = ProvenanceManager::new(&new_key, Some(dir.path())).unwrap();
    assert!(reopened.verify_log_integrity().unwrap());
    reopened.verify_keri_log().unwrap();
    reopened
        .create_fact_provenance("after", b"new key", None)
        .unwrap();
    assert!(reopened.verify_log_integrity().unwrap());
    reopened.verify_keri_log().unwrap();
}

#[test]
fn interior_nodes_verify_as_content_only_in_legacy_trees() {
    let content: Vec<[u8; 32]> = (0..4u32)
        .map(|i| Sha256::digest(i.to_le_bytes()).into())
        .collect();
    for hashing in [MerkleHashing::Legacy, MerkleHashing::Rfc6962] {
        let leaves: Vec<[u8; 32]> = content.iter().map(|h| hashing.leaf(h)).collect();
        let levels = ProvenanceManager::merkle_levels(&leaves, hashing);
        let root = levels[levels.len() - 1][0];
        // The 64 bytes of leaves 0 and 1 claimed as content, with leaf 0's proof minus its
        // first sibling.
        let mut forged_content = Vec::with_capacity(64);
        forged_content.extend_from_slice(&leaves[0]);
        forged_content.extend_from_slice(&leaves[1]);
        let forged_hash: [u8; 32] = Sha256::digest(&forged_content).into();
        let proof = ProvenanceManager::merkle_proof(&levels, 0);
        let accepted =
            check_inclusion(hashing, &forged_hash, Some(&proof[1..]), &root, Some(4)).is_ok();
        assert_eq!(accepted, hashing == MerkleHashing::Legacy, "{hashing:?}");
        for (i, h) in content.iter().enumerate() {
            let proof = ProvenanceManager::merkle_proof(&levels, i);
            check_inclusion(hashing, h, Some(&proof), &root, Some(4)).unwrap();
        }
    }
}

#[test]
fn new_batches_sign_a_versioned_root_claim() {
    let (_dir, mgr) = manager();
    let items: Vec<(&str, &[u8], Option<&str>)> = vec![
        ("v1", b"one", None),
        ("v2", b"two", None),
        ("v3", b"three", None),
    ];
    let records = mgr.create_batch_provenance(&items).unwrap();
    let payload = cose::verify_and_extract(
        &mgr.signing_key.verifying_key(),
        records[0].cose_envelope.as_ref().unwrap(),
    )
    .unwrap();
    let (_, size) = parse_batch_root_claim(&payload).unwrap().unwrap();
    assert_eq!(size, 3);

    let mut forged = records[0].clone();
    forged
        .merkle_proof
        .as_mut()
        .unwrap()
        .push((false, [0u8; 32]));
    assert_rejected(&mgr, &forged, "a proof longer than the signed tree allows");

    let mut tampered = ciborium::Value::Map(vec![]);
    if let Ok(ciborium::Value::Map(mut entries)) = ciborium::from_reader(payload.as_slice()) {
        for (k, v) in &mut entries {
            if k.as_text() == Some("v") {
                *v = ciborium::Value::Integer(3u64.into());
            }
        }
        tampered = ciborium::Value::Map(entries);
    }
    let mut bytes = Vec::new();
    ciborium::into_writer(&tampered, &mut bytes).unwrap();
    assert!(parse_batch_root_claim(&bytes).is_err());
}

#[test]
fn records_written_before_the_binding_checks_still_verify() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("testdata/legacy_v1_records.json")).unwrap();
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let trust = TrustStore::trusting_key(&key.verifying_key());
    let mgr = ProvenanceManager::from_signing_key(key);
    let mut records: Vec<ProvenanceRecord> = ["fact", "fact_with_uri", "triple"]
        .iter()
        .map(|k| serde_json::from_value(fixture[k].clone()).unwrap())
        .collect();
    records
        .extend(serde_json::from_value::<Vec<ProvenanceRecord>>(fixture["batch"].clone()).unwrap());
    assert_eq!(records.len(), 6);
    for r in &records {
        let free = verify_record(r, &trust).unwrap();
        assert!(free.valid, "{:?}", free.details);
        let own = mgr.verify_fact_provenance(r, &trust).unwrap();
        assert!(own.valid, "{:?}", own.details);
    }

    // A legacy batch envelope signs a bare hash, as the log head does: without the
    // credential the record proves nothing about a batch.
    let mut forged = records[3].clone();
    forged.vc_json = None;
    assert!(!verify_record(&forged, &trust).unwrap().valid);
    assert!(!mgr.verify_fact_provenance(&forged, &trust).unwrap().valid);
}

/// A Sigstore bundle over caller-chosen bytes must not be a signature a fact claim can
/// reuse: the bundle signs under its own domain, not the raw bytes.
#[test]
fn sigstore_bundle_signature_does_not_forge_a_fact_claim() {
    use base64::Engine;
    use coset::{CborSerializable, CoseSign1Builder, HeaderBuilder};

    let (_dir, mgr) = manager();
    let genuine = mgr.create_fact_provenance("f", b"genuine", None).unwrap();
    let forged_hash: [u8; 32] = Sha256::digest(b"forged").into();
    let claim = ciborium::Value::Map(vec![
        ("id".into(), "f".into()),
        (
            "content_hash".into(),
            ciborium::Value::Bytes(forged_hash.to_vec()),
        ),
        ("timestamp_ms".into(), genuine.timestamp_ms.into()),
    ]);
    let mut payload = Vec::new();
    ciborium::into_writer(&claim, &mut payload).unwrap();
    let mut sign1 = CoseSign1Builder::new()
        .protected(
            HeaderBuilder::new()
                .algorithm(coset::iana::Algorithm::EdDSA)
                .key_id(mgr.verifying_key().to_bytes().to_vec())
                .build(),
        )
        .payload(payload)
        .build();
    let bundle = mgr
        .create_sigstore_bundle(&sign1.tbs_data(b""), None)
        .unwrap();
    sign1.signature = base64::engine::general_purpose::STANDARD
        .decode(&bundle.message_signature.signature)
        .unwrap();

    let mut forged = genuine;
    forged.source_hash = Some(forged_hash);
    forged.vc_json = None;
    forged.cose_envelope = Some(sign1.to_vec().unwrap());
    assert_rejected(
        &mgr,
        &forged,
        "a fact claim signed through a Sigstore bundle",
    );
}

/// A credential from another trusted issuer is checked against that issuer's status list,
/// not this manager's own list.
#[test]
fn foreign_revoked_credential_fails_the_manager_check() {
    let (_dir, issuer) = manager();
    let (_dir2, verifier) = manager();
    let record = issuer
        .create_fact_provenance("x", b"foreign", None)
        .unwrap();
    let credential: vc::FactCredential =
        serde_json::from_str(record.vc_json.as_deref().unwrap()).unwrap();
    let index: u64 = credential
        .credential_status
        .as_ref()
        .unwrap()
        .status_list_index
        .parse()
        .unwrap();
    issuer.revoke_credential(index).unwrap();

    let trust = issuer.self_trust();
    assert!(!verify_record(&record, &trust).unwrap().valid);
    let result = verifier.verify_fact_provenance(&record, &trust).unwrap();
    assert!(
        result
            .details
            .iter()
            .any(|d| d.check == "revocation_status" && !d.passed),
        "{:?}",
        result.details
    );
    assert!(!result.valid);

    // The verifier's own list does not revoke a foreign credential either.
    let other = issuer.create_fact_provenance("y", b"other", None).unwrap();
    let other_index: u64 =
        serde_json::from_str::<vc::FactCredential>(other.vc_json.as_deref().unwrap())
            .unwrap()
            .credential_status
            .unwrap()
            .status_list_index
            .parse()
            .unwrap();
    verifier.revoke_credential(other_index).unwrap();
    assert!(
        verifier
            .verify_fact_provenance(&other, &issuer.self_trust())
            .unwrap()
            .valid
    );
}
