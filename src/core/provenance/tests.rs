// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use tempfile::{tempdir, TempDir};

/// A manager with a fresh key, logging under its own temporary directory.
fn manager() -> (TempDir, ProvenanceManager) {
    let dir = tempdir().unwrap();
    let mgr = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    (dir, mgr)
}

#[test]
fn full_fact_provenance_flow() {
    let (_dir, mgr) = manager();

    let record = mgr
        .create_fact_provenance(
            "fact-001",
            b"Paris is the capital of France",
            Some("https://example.com"),
        )
        .unwrap();

    assert!(record.cose_envelope.is_some());
    assert!(record.vc_json.is_some());
    assert!(record.issuer_did.is_some());
    assert!(record.issuer_did.as_ref().unwrap().starts_with("did:key:z"));

    let verification = mgr
        .verify_fact_provenance(&record, &mgr.self_trust())
        .unwrap();
    assert!(verification.valid);
    // cose_signature, vc_proof, vc_semantics, revocation_status, record_binding, trust_anchor
    assert_eq!(verification.details.len(), 6);

    assert!(mgr.get_record("fact-001").is_some());
    assert_eq!(mgr.record_count(), 1);
}

#[test]
fn full_triple_provenance_flow() {
    let (_dir, mgr) = manager();

    let record = mgr
        .create_triple_provenance(&TripleProvenanceParams {
            fact_id: "triple-001",
            content: b"paris capital_of france",
            dimensions: 16384,
            subject: "paris",
            relation: "capital_of",
            object: "france",
            source_uri: None,
        })
        .unwrap();

    let vc: vc::FactCredential = serde_json::from_str(record.vc_json.as_ref().unwrap()).unwrap();
    assert_eq!(vc.credential_subject.subject_id.as_deref(), Some("paris"));

    let verification = mgr
        .verify_fact_provenance(&record, &mgr.self_trust())
        .unwrap();
    assert!(verification.valid);
    assert!(mgr.get_record("triple-001").is_some());
}

#[test]
fn store_manifest_flow() {
    let (_dir, mgr) = manager();

    let store_data = b"serialized store contents here";
    let manifest = mgr
        .create_store_manifest("store-001", store_data, 42, 16384, Some("Test Store"))
        .unwrap();

    assert!(manifest.cose_envelope.is_some());

    let verification = mgr
        .verify_store_manifest(&manifest, Some(store_data), &mgr.self_trust())
        .unwrap();
    assert!(verification.valid);
    assert!(verification
        .details
        .iter()
        .any(|d| d.check == "store_hash" && d.passed));

    let bad_verification = mgr
        .verify_store_manifest(&manifest, Some(b"wrong"), &mgr.self_trust())
        .unwrap();
    assert!(!bad_verification.valid);
}

#[test]
fn audit_deleted_issuances_never_reuse_a_status_slot_after_reopen() {
    let dir = tempdir().unwrap();
    let key = dir.path().join("test.key");
    let last = {
        let mgr = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
        let records = mgr
            .create_batch_provenance(&[("a", b"a", None), ("b", b"b", None)])
            .unwrap();
        for (id, record) in ["a", "b"].iter().zip(&records) {
            mgr.revoke_credential(record.sequence).unwrap();
            mgr.record_deletion(id, None).unwrap();
        }
        records[1].sequence
    };
    let mgr = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
    let record = mgr.create_fact_provenance("c", b"c", None).unwrap();
    assert!(record.sequence > last);
    assert!(!mgr.is_revoked(record.sequence));
    assert!(verify_record(&record, &mgr.self_trust()).unwrap().valid);
}

#[test]
fn audit_sequence_reservation_fails_without_wrapping() {
    let (_, mgr) = manager();
    mgr.logical_clock
        .store(u64::MAX, std::sync::atomic::Ordering::SeqCst);
    assert!(mgr.create_fact_provenance("a", b"a", None).is_err());
    assert!(mgr
        .create_batch_provenance(&[("b", b"b", None), ("c", b"c", None)])
        .is_err());
}

#[test]
fn records_persist_to_disk() {
    let dir = tempdir().unwrap();
    let key_path = dir.path().join("test.key");

    let issuer = {
        let mgr = ProvenanceManager::new(&key_path, Some(dir.path())).unwrap();
        mgr.create_fact_provenance("f1", b"content one", None)
            .unwrap();
        mgr.create_fact_provenance("f2", b"content two", None)
            .unwrap();
        assert_eq!(mgr.record_count(), 2);
        mgr.issuer_did().to_string()
    };

    let mgr2 = ProvenanceManager::new(&key_path, Some(dir.path())).unwrap();
    assert_eq!(mgr2.issuer_did(), issuer);
    assert_eq!(mgr2.record_count(), 2);
    assert!(mgr2.get_record("f1").is_some());
    assert!(mgr2.get_record("f2").is_some());
}

#[test]
fn standalone_verify_without_manager() {
    let (_dir, mgr) = manager();
    let record = mgr
        .create_fact_provenance("standalone-001", b"verify without manager", None)
        .unwrap();
    let issuer = record.issuer_did.clone().unwrap();
    drop(mgr);

    let trust = trust::TrustStore::trusting_did(&issuer).unwrap();
    let result = verify_record(&record, &trust).unwrap();
    assert!(result.valid);
    // cose_signature, vc_proof, vc_semantics, revocation_status (unchecked: no list trusted),
    // record_binding, trust_anchor
    assert_eq!(result.details.len(), 6);
}

#[test]
fn tampered_log_detected() {
    let (dir, mgr) = manager();
    mgr.create_fact_provenance("t1", b"one", None).unwrap();
    mgr.create_fact_provenance("t2", b"two", None).unwrap();
    assert!(mgr.verify_log_integrity().unwrap());
    drop(mgr);

    let log_path = dir.path().join("provenance_log.jsonl");
    let contents = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = contents.lines().collect();
    let tampered = lines[1].replace("\"t2\"", "\"TAMPERED\"");
    std::fs::write(&log_path, format!("{}\n{tampered}\n", lines[0])).unwrap();

    let mgr2 = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    assert!(!mgr2.verify_log_integrity().unwrap());
}

#[test]
fn absent_history_is_not_authenticated_history() {
    let (dir, mgr) = manager();
    assert_eq!(mgr.log_integrity().unwrap(), LogIntegrity::Empty);
    assert!(!mgr.verify_log_integrity().unwrap());
    std::fs::write(dir.path().join("provenance_log.jsonl"), "").unwrap();
    assert_eq!(mgr.log_integrity().unwrap(), LogIntegrity::Empty);
    let volatile = ProvenanceManager::from_signing_key(mgr.signing_key.clone());
    assert_eq!(volatile.log_integrity().unwrap(), LogIntegrity::Volatile);
    assert!(!volatile.verify_log_integrity().unwrap());
}

#[test]
fn missing_truncated_or_reordered_evidence_never_verifies() {
    for damage in [
        "empty",
        "missing",
        "head",
        "both",
        "truncate",
        "reorder",
        "signature",
    ] {
        let (dir, mgr) = manager();
        mgr.create_fact_provenance("one", b"one", None).unwrap();
        mgr.create_fact_provenance("two", b"two", None).unwrap();
        assert_eq!(mgr.log_integrity().unwrap(), LogIntegrity::Verified);
        let log = dir.path().join("provenance_log.jsonl");
        let head = log.with_extension("head");
        let contents = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<_> = contents.lines().collect();
        match damage {
            "empty" => std::fs::write(&log, "").unwrap(),
            "missing" => std::fs::remove_file(&log).unwrap(),
            "head" => std::fs::remove_file(&head).unwrap(),
            "both" => {
                std::fs::remove_file(&log).unwrap();
                std::fs::remove_file(&head).unwrap();
            }
            "truncate" => std::fs::write(&log, format!("{}\n", lines[0])).unwrap(),
            "reorder" => std::fs::write(&log, format!("{}\n{}\n", lines[1], lines[0])).unwrap(),
            "signature" => std::fs::write(&head, b"invalid signature").unwrap(),
            _ => unreachable!(),
        }
        assert!(!mgr.verify_log_integrity().unwrap(), "{damage}");
        drop(mgr);
        let reopened =
            ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
        assert!(
            !reopened.verify_log_integrity().unwrap(),
            "reopened {damage}"
        );
    }
}

#[test]
fn unreadable_log_or_head_returns_an_error() {
    for filename in ["provenance_log.jsonl", "provenance_log.head"] {
        let (dir, mgr) = manager();
        mgr.create_fact_provenance("one", b"one", None).unwrap();
        let path = dir.path().join(filename);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(mgr.verify_log_integrity().is_err(), "{filename}");
    }
}

#[test]
fn failed_signed_head_reports_committed_state_and_preserves_replay() {
    for operation in ["create", "delete", "batch", "rotate"] {
        let (dir, mut mgr) = manager();
        mgr.create_fact_provenance("one", b"one", None).unwrap();
        let head = dir.path().join("provenance_log.head");
        std::fs::remove_file(&head).unwrap();
        std::fs::create_dir(&head).unwrap();
        let new_key = dir.path().join("rotated.key");
        let error = match operation {
            "create" => mgr.create_fact_provenance("two", b"two", None).unwrap_err(),
            "delete" => mgr.record_deletion("one", None).unwrap_err(),
            "batch" => mgr
                .create_batch_provenance(&[("two", b"two", None)])
                .unwrap_err(),
            "rotate" => mgr.rotate_key(&new_key).unwrap_err(),
            _ => unreachable!(),
        };
        assert!(
            error.is::<crate::CommittedMutationError>(),
            "{operation}: {error:#}"
        );
        assert!(mgr.verify_log_integrity().is_err());
        let before = mgr.record_count();
        let issuer = mgr.issuer_did().to_string();
        assert_eq!(mgr.get_record("one").is_some(), operation != "delete");
        assert_eq!(
            mgr.get_record("two").is_some(),
            matches!(operation, "create" | "batch")
        );
        drop(mgr);
        let key = if operation == "rotate" {
            new_key
        } else {
            dir.path().join("test.key")
        };
        let reopened = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
        assert_eq!(reopened.record_count(), before);
        assert_eq!(reopened.issuer_did(), issuer);
        std::fs::remove_dir(&head).unwrap();
        assert_eq!(reopened.log_integrity().unwrap(), LogIntegrity::Unanchored);
        reopened
            .create_fact_provenance("next", b"next", None)
            .unwrap();
        assert!(reopened.verify_log_integrity().unwrap());
    }
}

#[test]
fn key_rotation_preserves_old_records() {
    let dir = tempdir().unwrap();
    let key_path = dir.path().join("orig.key");
    let mut mgr = ProvenanceManager::new(&key_path, Some(dir.path())).unwrap();
    let old_did = mgr.issuer_did().to_string();

    let record = mgr
        .create_fact_provenance("before-rotate", b"old key", None)
        .unwrap();

    let new_key_path = dir.path().join("rotated.key");
    let new_did = mgr.rotate_key(&new_key_path).unwrap();
    assert_ne!(old_did, new_did);
    assert_eq!(mgr.issuer_did(), new_did);

    let post_record = mgr
        .create_fact_provenance("after-rotate", b"new key", None)
        .unwrap();
    assert_eq!(post_record.issuer_did.as_deref(), Some(new_did.as_str()));

    let mut trust = trust::TrustStore::new();
    trust.trust_did(&old_did).unwrap();
    trust.trust_did(&new_did).unwrap();

    let old_result = verify_record(&record, &trust).unwrap();
    assert!(old_result.valid);

    let new_result = verify_record(&post_record, &trust).unwrap();
    assert!(new_result.valid);

    assert!(mgr.verify_log_integrity().unwrap());
}

#[test]
fn batch_provenance_merkle() {
    let (_dir, mgr) = manager();

    let items: Vec<(&str, &[u8], Option<&str>)> = vec![
        ("b1", b"alpha", None),
        ("b2", b"beta", None),
        ("b3", b"gamma", Some("https://example.com")),
        ("b4", b"delta", None),
    ];

    let records = mgr.create_batch_provenance(&items).unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(mgr.record_count(), 4);

    let envelope = records[0].cose_envelope.as_ref().unwrap();
    for r in &records {
        assert_eq!(r.cose_envelope.as_ref().unwrap(), envelope);
        assert!(r.merkle_proof.is_some());
        assert!(r.vc_json.is_some());
    }

    let trust = mgr.self_trust();
    for r in &records {
        let result = verify_record(r, &trust).unwrap();
        assert!(result
            .details
            .iter()
            .any(|d| d.check == "vc_proof" && d.passed));
    }
}

#[test]
fn credential_revocation() {
    let (_dir, mgr) = manager();

    let record = mgr
        .create_fact_provenance("fact-rev", b"revocable content", None)
        .unwrap();

    let verification = mgr
        .verify_fact_provenance(&record, &mgr.self_trust())
        .unwrap();
    assert!(verification.valid);
    assert!(verification
        .details
        .iter()
        .any(|d| d.check == "revocation_status" && d.passed));

    let vc: vc::FactCredential = serde_json::from_str(record.vc_json.as_ref().unwrap()).unwrap();
    let status_index: u64 = vc
        .credential_status
        .as_ref()
        .unwrap()
        .status_list_index
        .parse()
        .unwrap();
    mgr.revoke_credential(status_index).unwrap();
    assert!(mgr.is_revoked(status_index));

    let verification = mgr
        .verify_fact_provenance(&record, &mgr.self_trust())
        .unwrap();
    assert!(!verification.valid);
    assert!(verification
        .details
        .iter()
        .any(|d| d.check == "revocation_status" && !d.passed));
}

#[test]
fn unreadable_provenance_state_fails_to_open() {
    for name in ["provenance_log.jsonl", "provenance_log.statuslist"] {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(name)).unwrap();
        assert!(
            ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).is_err(),
            "{name}"
        );
    }
}

#[test]
fn failed_log_append_leaves_head_and_records_unchanged() {
    let (dir, mgr) = manager();
    mgr.create_fact_provenance("a", b"first", None).unwrap();
    let log = dir.path().join("provenance_log.jsonl");
    let head_file = log.with_extension("head");
    let head = *mgr.chain_head.lock();
    let signed = std::fs::read(&head_file).unwrap();

    let aside = dir.path().join("aside.jsonl");
    std::fs::rename(&log, &aside).unwrap();
    std::fs::create_dir(&log).unwrap();
    assert!(mgr.create_fact_provenance("b", b"second", None).is_err());
    assert!(mgr.record_deletion("a", None).is_err());
    assert_eq!(*mgr.chain_head.lock(), head);
    assert_eq!(std::fs::read(&head_file).unwrap(), signed);
    assert!(mgr.get_record("b").is_none());
    assert!(mgr.get_record("a").is_some());

    std::fs::remove_dir(&log).unwrap();
    std::fs::rename(&aside, &log).unwrap();
    mgr.create_fact_provenance("c", b"third", None).unwrap();
    assert!(mgr.verify_log_integrity().unwrap());
    drop(mgr);
    let reopened = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    assert!(reopened.verify_log_integrity().unwrap());
    assert!(reopened.get_record("a").is_some());
    assert!(reopened.get_record("b").is_none());
    assert!(reopened.get_record("c").is_some());
}

#[test]
fn batch_log_chains_and_reloads() {
    let (dir, mgr) = manager();
    mgr.create_fact_provenance("single", b"before", None)
        .unwrap();
    let items: Vec<(&str, &[u8], Option<&str>)> = vec![
        ("x", b"one", None),
        ("y", b"two", Some("u")),
        ("z", b"three", None),
    ];
    mgr.create_batch_provenance(&items).unwrap();
    assert!(mgr.verify_log_integrity().unwrap());
    let log = std::fs::read_to_string(dir.path().join("provenance_log.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 4);
    drop(mgr);
    let reopened = ProvenanceManager::new(&dir.path().join("test.key"), Some(dir.path())).unwrap();
    assert!(reopened.verify_log_integrity().unwrap());
    assert_eq!(reopened.record_count(), 4);
    let two: [u8; 32] = Sha256::digest(b"two").into();
    assert_eq!(reopened.get_record("y").unwrap().source_hash, Some(two));
}

/// The per-leaf tree rebuild the shared-levels proofs replaced.
fn reference_proof(leaves: &[[u8; 32]], index: usize) -> Vec<(bool, [u8; 32])> {
    let mut proof = Vec::new();
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    let mut idx = index;
    while level.len() > 1 {
        let sibling_idx = if idx.is_multiple_of(2) {
            idx + 1
        } else {
            idx - 1
        };
        if sibling_idx < level.len() {
            proof.push((idx % 2 == 1, level[sibling_idx]));
        }
        level = ProvenanceManager::merkle_parents(&level, MerkleHashing::Rfc6962);
        idx /= 2;
    }
    proof
}

#[test]
fn merkle_proofs_match_per_leaf_rebuild() {
    for n in 1..=65u32 {
        let leaves: Vec<[u8; 32]> = (0..n)
            .map(|i| Sha256::digest(i.to_le_bytes()).into())
            .collect();
        let levels = ProvenanceManager::merkle_levels(&leaves, MerkleHashing::Rfc6962);
        let mut root = leaves.clone();
        while root.len() > 1 {
            root = ProvenanceManager::merkle_parents(&root, MerkleHashing::Rfc6962);
        }
        assert_eq!(levels[levels.len() - 1], root, "n={n}");
        for index in 0..leaves.len() {
            let proof = ProvenanceManager::merkle_proof(&levels, index);
            assert_eq!(
                proof,
                reference_proof(&leaves, index),
                "n={n} index={index}"
            );
            assert_eq!(
                merkle_root_from_proof(MerkleHashing::Rfc6962, leaves[index], &proof),
                root[0],
                "n={n} index={index}"
            );
        }
    }
}

#[test]
fn revocation_survives_reopen() {
    let dir = tempdir().unwrap();
    let key = dir.path().join("test.key");
    let mgr = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
    mgr.revoke_credential(13).unwrap();
    drop(mgr);
    let reopened = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
    assert!(reopened.is_revoked(13));
    assert!(!reopened.is_revoked(12));
    assert!(!reopened.is_revoked(14));
}

/// A revocation that cannot be persisted is not applied, so the live list never claims
/// a revocation a restart would undo.
#[test]
fn failed_revocation_write_leaves_the_credential_live() {
    let dir = tempdir().unwrap();
    let key = dir.path().join("test.key");
    let mgr = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
    mgr.revoke_credential(1).unwrap();
    let list = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "statuslist"))
        .unwrap();
    std::fs::remove_file(&list).unwrap();
    std::fs::create_dir(&list).unwrap();
    std::fs::write(list.join("occupied"), b"x").unwrap();
    assert!(mgr.revoke_credential(13).is_err());
    assert!(!mgr.is_revoked(13));
    assert!(mgr.is_revoked(1));
}

/// Concurrent revocations all reach the persisted list: none is overwritten by a write of
/// an older list.
#[test]
fn concurrent_revocations_all_persist() {
    let dir = tempdir().unwrap();
    let key = dir.path().join("test.key");
    let mgr = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
    std::thread::scope(|scope| {
        for thread in 0..8u64 {
            let mgr = &mgr;
            scope.spawn(move || {
                for i in 0..32 {
                    mgr.revoke_credential(thread * 1000 + i).unwrap();
                }
            });
        }
    });
    drop(mgr);
    let reopened = ProvenanceManager::new(&key, Some(dir.path())).unwrap();
    for thread in 0..8u64 {
        for i in 0..32 {
            assert!(reopened.is_revoked(thread * 1000 + i));
        }
    }
}

#[test]
fn unallocatable_status_index_is_an_error() {
    let (_dir, mgr) = manager();
    assert!(mgr.revoke_credential(u64::MAX).is_err());
    assert!(!mgr.is_revoked(u64::MAX));
    mgr.revoke_credential(9).unwrap();
    assert!(mgr.is_revoked(9));
    assert!(!mgr.is_revoked(8));
}

#[test]
fn untrusted_key_rejected() {
    let (_dir, mgr) = manager();
    let record = mgr
        .create_fact_provenance("fact-trust", b"authentic content", None)
        .unwrap();

    // Signed by the manager and trusting the manager: authentic.
    let ok = mgr
        .verify_fact_provenance(&record, &mgr.self_trust())
        .unwrap();
    assert!(ok.valid);

    // The record's signature is internally valid, but a trust store that
    // does not contain the signing key must reject it as non-authentic,
    // even though every signature check passes.
    let stranger = SigningKey::from_bytes(&rand::random());
    let foreign_trust = trust::TrustStore::trusting_key(&stranger.verifying_key());
    let rejected = mgr.verify_fact_provenance(&record, &foreign_trust).unwrap();
    assert!(!rejected.valid);
    assert!(rejected
        .details
        .iter()
        .any(|d| d.check == "trust_anchor" && !d.passed));

    // Self-consistency, by contrast, still passes: the record is intact.
    let self_consistent = mgr
        .verify_fact_provenance_self_consistency(&record)
        .unwrap();
    assert!(self_consistent.valid);
}

#[test]
fn vc_only_record_requires_trust() {
    // A record carrying only a self-signed VC (no COSE envelope) must still
    // be trust-anchored via its issuer DID, not treated as authentic just
    // because the VC verifies against its own embedded issuer.
    let (_dir, mgr) = manager();
    let record = mgr
        .create_fact_provenance("vc-only", b"content", None)
        .unwrap();

    let mut vc_only = record.clone();
    vc_only.cose_envelope = None;

    let ok = verify_record(&vc_only, &mgr.self_trust()).unwrap();
    assert!(ok.valid);
    assert!(ok
        .details
        .iter()
        .any(|d| d.check == "trust_anchor" && d.passed));

    let stranger = SigningKey::from_bytes(&rand::random());
    let foreign = trust::TrustStore::trusting_key(&stranger.verifying_key());
    let rejected = verify_record(&vc_only, &foreign).unwrap();
    assert!(!rejected.valid);
    assert!(rejected
        .details
        .iter()
        .any(|d| d.check == "trust_anchor" && !d.passed));
}

#[test]
fn cawg_untrusted_issuer_rejected() {
    let (_dir, mgr) = manager();

    let referenced = vec![(
        "self#jumbf=c2pa.assertions/c2pa.hash.data".to_string(),
        "sha256".to_string(),
        vec![0x11u8; 32],
    )];
    let assertion = mgr
        .create_cawg_assertion(&referenced, "warm agent")
        .unwrap();

    // The manager's own CAWG issuer (did:jwk of its key) is trusted by self_trust.
    assert!(mgr
        .verify_cawg_assertion(&assertion, &mgr.self_trust())
        .is_ok());

    // A trust store that does not contain the issuer must reject it, even
    // though the assertion is internally self-consistent.
    let stranger = SigningKey::from_bytes(&rand::random());
    let foreign = trust::TrustStore::trusting_key(&stranger.verifying_key());
    assert!(mgr.verify_cawg_assertion(&assertion, &foreign).is_err());
}
