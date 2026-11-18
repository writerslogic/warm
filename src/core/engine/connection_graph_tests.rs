// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn connection_graph_open_failure_does_not_fall_back_to_volatile_state() {
    let dir = tempfile::tempdir().unwrap();
    let warm = WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap();
    let log = dir.path().join("connection_graph.log");
    std::fs::create_dir(&log).unwrap();
    assert!(warm.relate("one", "rel", "two").is_err());
    assert!(warm.query_relation("one", "rel", "two").is_err());
    std::fs::remove_dir(&log).unwrap();
    warm.relate("one", "rel", "two").unwrap();
    drop(warm);
    let reopened = WarmCore::new(256, Some(dir.path().display().to_string()), None).unwrap();
    assert!(reopened.query_relation("one", "rel", "two").unwrap() > 0.0);
}

#[test]
fn relate_and_query_through_engine() {
    let dir = tempfile::tempdir().unwrap();
    let warm = WarmCore::new(4096, Some(dir.path().to_string_lossy().to_string()), None).unwrap();

    // Query before any relation exists -> graph unused -> 0.0.
    assert_eq!(
        warm.query_relation("paris", "capital_of", "france")
            .unwrap(),
        0.0
    );

    warm.relate("paris", "capital_of", "france").unwrap();
    let present = warm
        .query_relation("paris", "capital_of", "france")
        .unwrap();
    let absent = warm
        .query_relation("berlin", "capital_of", "spain")
        .unwrap();
    assert!(
        present > absent,
        "asserted relation ({present}) must score above an absent one ({absent})"
    );
}

#[test]
fn connection_graph_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();

    {
        let warm = WarmCore::new(4096, Some(path.clone()), None).unwrap();
        warm.relate("paris", "capital_of", "france").unwrap();
    } // engine dropped -> connection graph's BufWriter flushed on drop

    // New engine over the same storage: the relation must reload from the log.
    let hms2 = WarmCore::new(4096, Some(path), None).unwrap();
    let present = hms2
        .query_relation("paris", "capital_of", "france")
        .unwrap();
    let absent = hms2
        .query_relation("madrid", "capital_of", "italy")
        .unwrap();
    assert!(
        present > absent,
        "relation must survive restart: reloaded ({present}) !> absent ({absent})"
    );
}

#[test]
fn phase_graph_relate_and_retrieve() {
    let dir = tempfile::tempdir().unwrap();
    let warm = WarmCore::new(4096, Some(dir.path().to_string_lossy().to_string()), None).unwrap();
    warm.relate_phase("paris", "capital_of", "france");
    warm.relate_phase("berlin", "capital_of", "germany");
    // forward retrieval and inverse (subject) retrieval from the same field.
    assert_eq!(
        warm.phase_retrieve_object("paris", "capital_of").as_deref(),
        Some("france")
    );
    assert_eq!(
        warm.phase_retrieve_subject("capital_of", "germany")
            .as_deref(),
        Some("berlin")
    );
    warm.relate_phase("alice", "parent_of", "bob");
    warm.relate_phase("bob", "parent_of", "carol");
    assert_eq!(
        warm.phase_retrieve_object("bob", "parent_of").as_deref(),
        Some("carol")
    );
    assert_eq!(
        warm.phase_retrieve_subject("parent_of", "bob").as_deref(),
        Some("alice")
    );
    assert_eq!(
        warm.phase_retrieve_path("alice", &["parent_of", "parent_of"])
            .as_deref(),
        Some("carol")
    );
}
