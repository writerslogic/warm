// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// Separate test target: arms process-wide failpoints that would fire in unrelated tests.

//! A batch whose commit hits ENOSPC or EIO at any durability point (see `core::failpoint`) is
//! never visible: not to the writer's memory, not to a reader refreshing during or after the
//! failure, not after a reopen. A batch that returns `Ok` is whole everywhere.
//!
//! The failpoint state is process global, so this file holds a single test.
#![cfg(feature = "failpoints")]

use std::path::Path;
use writerslogic_warm::core::failpoint::{arm, disarm, hits, Fault};
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{BatchOp, CommittedMutationError, IndeterminateMutationError, WarmCore};

fn config() -> Option<WarmConfig> {
    Some(WarmConfig::unencrypted())
}

fn writer(dir: &Path) -> anyhow::Result<WarmCore> {
    WarmCore::new(256, Some(dir.display().to_string()), config())
}

fn reader(dir: &Path) -> anyhow::Result<WarmCore> {
    WarmCore::open_read_only(256, Some(dir.display().to_string()), config())
}

fn put(id: &str, text: &str) -> BatchOp {
    BatchOp::Memorize {
        document: serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap(),
    }
}

fn batch() -> Vec<BatchOp> {
    vec![
        put("n1", "new zebra one"),
        BatchOp::Delete { id: "gone".into() },
        put("edit", "edit zebra rewritten"),
        put("n2", "new zebra two"),
    ]
}

fn seed(dir: &Path) {
    let core = writer(dir).unwrap();
    for (id, text) in [
        ("keep", "keep zebra"),
        ("edit", "edit zebra"),
        ("gone", "gone zebra"),
    ] {
        core.memorize_document(
            serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap(),
        )
        .unwrap();
    }
}

fn snapshot(core: &WarmCore) -> String {
    let infos = core.list_document_info(None, 1000).unwrap();
    let mut out = String::new();
    for info in infos {
        let text = core.get_document(&info.id).unwrap().unwrap().text;
        out += &format!("{}={:?}\n", info.id, text);
    }
    out
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".hms.lock" {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn a_batch_that_hits_a_disk_fault_is_never_visible() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("base");
    seed(&base);
    let none = snapshot(&reader(&base).unwrap());

    // Count the durability points one batch passes, and the state a whole batch leaves.
    let all = {
        let dir = root.path().join("count");
        copy_dir(&base, &dir);
        let core = writer(&dir).unwrap();
        arm(Fault::Io, u64::MAX, false);
        core.write_batch(batch()).unwrap();
        let total = hits();
        disarm();
        assert!(total >= 1, "a batch passes no durability point");
        (total, snapshot(&core))
    };
    let (total, all) = all;
    assert_ne!(none, all);

    let mut failures = 0;
    for sticky in [false, true] {
        for k in 1..=total {
            let dir = root.path().join(format!("k{k}-{sticky}"));
            copy_dir(&base, &dir);
            let core = writer(&dir).unwrap();
            let mut watcher = reader(&dir).unwrap();
            arm(Fault::Io, k, sticky);
            let result = core.write_batch(batch());
            disarm();
            // `Ok` and a durable-but-effect-failed error (`CommittedMutationError`: the frame is
            // synced, a later cleanup failed) mean the whole batch is committed. Any other
            // error means it is not, except an indeterminate rollback, whose disk state may be
            // either (never part).
            let committed = match &result {
                Ok(_) => true,
                Err(error) => error.is::<CommittedMutationError>(),
            };
            let indeterminate = result
                .as_ref()
                .err()
                .is_some_and(|error| error.is::<IndeterminateMutationError>());
            failures += usize::from(result.is_err());
            let expected = if committed { &all } else { &none };
            assert_eq!(
                &snapshot(&core),
                expected,
                "writer memory, point {k} sticky {sticky}"
            );
            // A reader may be behind, never torn.
            let _ = watcher.refresh();
            let seen = snapshot(&watcher);
            assert!(
                seen == none || seen == all,
                "reader saw a partial batch, point {k} sticky {sticky}"
            );
            if !committed && !indeterminate {
                assert_eq!(
                    seen, none,
                    "reader saw a failed batch, point {k} sticky {sticky}"
                );
            }
            drop((core, watcher));
            // The disk holds all or none, whichever the call reported.
            let reopened = snapshot(&writer(&dir).unwrap());
            assert!(
                reopened == none || reopened == all,
                "reopen, point {k} sticky {sticky}"
            );
            if committed {
                assert_eq!(reopened, all, "a committed batch was lost, point {k}");
            } else if !indeterminate {
                assert_eq!(reopened, none, "a failed batch persisted, point {k}");
            }
        }
    }
    assert!(failures > 0, "no armed point failed the batch");

    // Transient failure, then the same writer commits the batch and a reader follows.
    let dir = root.path().join("retry");
    copy_dir(&base, &dir);
    let core = writer(&dir).unwrap();
    let mut watcher = reader(&dir).unwrap();
    for k in 1..=total {
        arm(Fault::Io, k, false);
        let failed = core
            .write_batch(batch())
            .is_err_and(|error| !error.is::<CommittedMutationError>());
        disarm();
        if failed {
            assert_eq!(snapshot(&core), none);
            core.write_batch(batch()).unwrap();
            assert!(watcher.refresh().unwrap());
            assert_eq!(snapshot(&watcher), all);
            return;
        }
    }
    panic!("no armed point failed the batch");
}
