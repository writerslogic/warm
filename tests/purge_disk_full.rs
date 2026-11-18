// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// Separate test target, outside integration_tests: arms process-wide failpoints that would fire in unrelated tests sharing the process.

//! A purge interrupted by a crash must not make the store unopenable when the disk is full:
//! the store opens, reports the purge as pending (never as complete), keeps every acknowledged
//! document, and a later open with space finishes the purge.
//!
//! The failpoint state is process global, so this file holds a single test.
#![cfg(feature = "failpoints")]

mod erase_common;

use erase_common::*;
use std::path::Path;
use writerslogic_warm::core::failpoint::{arm, disarm, hits, Fault};
use writerslogic_warm::core::memory_api::*;

/// A store with documents 0..4 where document 0 was deleted by a purge that died before its
/// scrub (`pending`), or an identical store with no purge owed.
fn build(dir: &Path, pending: bool) {
    let memory = open(dir).unwrap();
    for i in 0..4 {
        apply(&memory, Op::Remember(i)).unwrap();
    }
    let ticket = pending.then(|| memory.core().mark_purge_pending().unwrap());
    memory
        .forget(
            &ForgetScope::Ids { ids: vec![id(0)] },
            &ForgetOptions {
                purge: false,
                ..ForgetOptions::default()
            },
        )
        .unwrap();
    drop(ticket);
}

#[test]
fn a_full_disk_does_not_block_open_and_a_later_open_finishes_the_purge() {
    let work = tempfile::tempdir().unwrap();
    // The durability points an open passes before it reaches the purge.
    let reference = work.path().join("reference");
    build(&reference, false);
    arm(Fault::Io, u64::MAX, false);
    drop(open(&reference).unwrap());
    let before_purge = hits();
    disarm();
    assert!(before_purge > 0);

    let dir = work.path().join("store");
    build(&dir, true);
    assert!(dir.join("purge.pending").exists());
    arm(Fault::NoSpace, before_purge + 1, true);
    let memory = open(&dir).expect("a pending purge must not block open on a full disk");
    let status = memory.core().purge_status();
    disarm();
    assert_eq!(status.pending, 1, "the purge is reported pending");
    assert!(status.last_error.is_some(), "and the reason is reported");
    assert!(
        dir.join("purge.pending").exists(),
        "the obligation stays on disk"
    );
    assert!(
        !leaks(&dir, &token(0)).is_empty(),
        "the text is still in the journal, which the pending purge says"
    );
    for i in 1..4 {
        assert_eq!(
            memory.get(&id(i)).unwrap().unwrap().text.as_deref(),
            Some(text(i).as_str()),
            "acknowledged {} survives",
            id(i)
        );
    }
    assert!(memory.get(&id(0)).unwrap().is_none());
    drop(memory);

    let memory = open(&dir).unwrap();
    assert_eq!(memory.core().purge_status().pending, 0);
    assert!(!dir.join("purge.pending").exists());
    assert!(
        leaks(&dir, &token(0)).is_empty(),
        "the later open erased it"
    );
    assert!(memory.get(&id(3)).unwrap().is_some());
}
