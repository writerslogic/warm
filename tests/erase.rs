// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! What `forget` leaves on disk. The default `forget` is a purge: when it returns, and after a
//! crash at any point during it followed by a reopen, no file of the store directory holds
//! the deleted text. A logical forget (`purge: false`) leaves the deleted frames in the
//! journal; the first test pins exactly which files those are.
//!
//! This scans files, not the device: see `tests/raw_image.rs` and `docs/memory-api/ERASURE.md`
//! for what that does and does not cover.

mod erase_common;

use erase_common::*;
use std::path::Path;
use writerslogic_warm::core::memory_api::*;

fn ids(list: &[usize]) -> ForgetScope {
    ForgetScope::Ids {
        ids: list.iter().map(|i| id(*i)).collect(),
    }
}

fn seed(memory: &Memory, n: usize) {
    for i in 0..n {
        apply(memory, Op::Remember(i)).unwrap();
    }
}

fn assert_clean(dir: &Path, indices: &[usize], when: &str) {
    for &i in indices {
        let found = leaks(dir, &token(i));
        assert!(found.is_empty(), "{} present {when}: {found:?}", id(i));
    }
}

#[test]
fn a_logical_forget_leaves_the_text_only_in_the_journal_segments() {
    let dir = tempfile::tempdir().unwrap();
    let memory = open(dir.path()).unwrap();
    seed(&memory, 4);
    memory
        .forget(
            &ids(&[0]),
            &ForgetOptions {
                purge: false,
                ..ForgetOptions::default()
            },
        )
        .unwrap();
    let found = leaks(dir.path(), &token(0));
    assert!(!found.is_empty(), "the journal keeps the deleted frame");
    for path in &found {
        let name = Path::new(path).file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with("seg_") && name.ends_with(".bin"),
            "unexpected remnant of a logical forget: {path}"
        );
    }
    // A later purge removes them all.
    memory.core().compact_scrubbing().unwrap();
    assert_clean(dir.path(), &[0], "after the purge");
}

#[test]
fn the_default_forget_leaves_no_trace_before_or_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let memory = open(dir.path()).unwrap();
    seed(&memory, 6);
    assert!(
        !leaks(dir.path(), &token(1)).is_empty(),
        "the fixture must write the text"
    );
    let report = memory
        .forget(&ids(&[0, 1, 2]), &ForgetOptions::default())
        .unwrap();
    assert!(report.purged && report.forgotten.len() == 3);
    assert_clean(dir.path(), &[0, 1, 2], "after forget");
    assert!(
        !leaks(dir.path(), &token(3)).is_empty(),
        "survivors stay readable"
    );
    assert!(!dir.path().join("purge.pending").exists());
    drop(memory);
    assert_clean(dir.path(), &[0, 1, 2], "after reopen");
    let memory = open(dir.path()).unwrap();
    assert!(memory.get(&id(3)).unwrap().is_some());
    assert!(memory.get(&id(0)).unwrap().is_none());
}

#[test]
fn forgetting_a_superseded_chain_and_replaced_text_erases_every_version() {
    let dir = tempfile::tempdir().unwrap();
    let memory = open(dir.path()).unwrap();
    for (i, supersedes) in [(0usize, None), (1, Some(0usize))] {
        memory
            .remember(RememberInput {
                id: Some(id(i)),
                text: text(i),
                supersedes: supersedes.map(|s| vec![id(s)]),
                ..RememberInput::default()
            })
            .unwrap();
    }
    memory
        .forget(
            &ids(&[1]),
            &ForgetOptions {
                include_chain: true,
                ..ForgetOptions::default()
            },
        )
        .unwrap();
    assert_clean(dir.path(), &[0, 1], "after forgetting the chain");
}

/// An interrupted purge is finished by the next open.
#[test]
fn an_interrupted_purge_is_finished_by_the_next_open() {
    let dir = tempfile::tempdir().unwrap();
    let memory = open(dir.path()).unwrap();
    seed(&memory, 4);
    // The first half of a purge: marked and committed, never compacted.
    memory.core().mark_purge_pending().unwrap();
    memory
        .forget(
            &ids(&[0]),
            &ForgetOptions {
                purge: false,
                ..ForgetOptions::default()
            },
        )
        .unwrap();
    assert!(!leaks(dir.path(), &token(0)).is_empty());
    drop(memory);
    let memory = open(dir.path()).unwrap();
    assert_clean(dir.path(), &[0], "after the open that finishes the purge");
    assert!(!dir.path().join("purge.pending").exists());
    assert!(memory.get(&id(1)).unwrap().is_some());
}

#[cfg(feature = "failpoints")]
mod crash {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    const SEEDED: usize = 4;

    /// Child: seeds, then forgets the first three documents (the default, purging forget).
    #[test]
    fn erase_child() {
        let Ok(dir) = std::env::var("WARM_ERASE_CHILD_DIR") else {
            return;
        };
        let memory = open(Path::new(&dir)).unwrap();
        seed(&memory, SEEDED);
        println!("ACK seeded");
        memory
            .forget(&ids(&[0, 1, 2]), &ForgetOptions::default())
            .unwrap();
        println!("ACK forgotten");
    }

    fn run_child(
        dir: &Path,
        failpoint: Option<&str>,
        trace: Option<&Path>,
    ) -> (Vec<String>, Option<i32>) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "crash::erase_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("WARM_ERASE_CHILD_DIR", dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(spec) = failpoint {
            command.env("WARM_FAILPOINT", spec);
        }
        if let Some(path) = trace {
            command.env("WARM_FAILPOINT_TRACE", path);
        }
        let mut child = command.spawn().unwrap();
        let acks = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| line.find("ACK ").map(|at| line[at + 4..].to_owned()))
            .collect();
        let status = child.wait().unwrap();
        (acks, status.code())
    }

    /// Cuts the process at every durability point of seeding and forgetting, reopens, and
    /// requires: a document the store no longer holds is in no file; an acknowledged forget
    /// removed all three.
    #[test]
    fn a_crash_at_every_point_of_a_purge_leaves_nothing_after_reopen() {
        let work = tempfile::tempdir().unwrap();
        let trace = work.path().join("trace");
        let (acks, code) = run_child(&work.path().join("count"), None, Some(&trace));
        assert_eq!(code, Some(0));
        assert_eq!(acks, ["seeded", "forgotten"]);
        let total = std::fs::read_to_string(&trace).unwrap().lines().count();
        assert!(
            total > 20,
            "the durability points were not reached: {total}"
        );
        let stride: usize = std::env::var("WARM_CRASH_STRIDE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);
        let offset: usize = std::env::var("WARM_CRASH_OFFSET")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut cut = 0;
        for k in (1 + offset..=total).step_by(stride) {
            let dir = work.path().join(format!("run{k}"));
            let (acks, code) = run_child(&dir, Some(&format!("exit@{k}")), None);
            if code == Some(0) {
                continue;
            }
            assert_eq!(
                code,
                Some(writerslogic_warm::core::failpoint::EXIT_CODE),
                "exit@{k}"
            );
            cut += 1;
            let context = format!("exit@{k} after {acks:?}");
            let memory = open(&dir).unwrap_or_else(|e| panic!("{context}: reopen: {e:#}"));
            let held = |i: usize| memory.get(&id(i)).unwrap().is_some();
            if acks.iter().any(|a| a == "forgotten") {
                assert!(
                    !held(0) && !held(1) && !held(2),
                    "{context}: acknowledged forget"
                );
                assert!(held(3), "{context}: survivor");
            }
            for i in 0..SEEDED {
                if !held(i) && acks.iter().any(|a| a == "seeded") {
                    let found = leaks(&dir, &token(i));
                    assert!(
                        found.is_empty(),
                        "{context}: {} remains in {found:?}",
                        id(i)
                    );
                }
            }
            assert!(
                !dir.join("purge.pending").exists(),
                "{context}: purge still pending"
            );
            drop(memory);
            for i in 0..SEEDED {
                if open(&dir).unwrap().get(&id(i)).unwrap().is_none() {
                    assert!(
                        leaks(&dir, &token(i)).is_empty(),
                        "{context}: second reopen"
                    );
                }
            }
        }
        assert!(cut > 0);
    }
}
