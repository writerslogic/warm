// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Process death at every durability point of the commit, compaction and purge paths.
//!
//! A child process runs a fixed workload (remember, forget, compaction) against a store and
//! prints `ACK <n>` after operation `n` returns. The parent first counts the durability
//! points the workload passes (every segment flush, file sync, directory sync, atomic write,
//! rename and scrub, see `core::failpoint`), then reruns the workload once per point with the
//! child ended at that point, reopens the store and checks: every acknowledged operation took
//! effect, the one in flight is whole or absent, every present document is searchable, and
//! no deleted text is in any file.
//!
//! This models a killed process, in which the kernel keeps every write. It does not model
//! power loss, where unsynced writes and unsynced renames may be lost: that is
//! `benchmarks/public/durability_matrix.py` (Linux, write-barrier shim) and the platform
//! sync calls documented in `docs/file-format/CRASH-CONSISTENCY.md`.
//!
//! `WARM_CRASH_STRIDE=n` samples every n-th point; `WARM_CRASH_OFFSET=m` shifts the start.
#![cfg(feature = "failpoints")]

mod erase_common;

use erase_common::*;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

const OPS: usize = 14;

#[test]
fn crash_child() {
    let Ok(dir) = std::env::var("WARM_CRASH_CHILD_DIR") else {
        return;
    };
    let memory = open(Path::new(&dir)).unwrap();
    for (n, op) in plan(OPS).into_iter().enumerate() {
        apply(&memory, op).unwrap();
        println!("ACK {n}");
    }
}

fn run_child(dir: &Path, failpoint: Option<&str>, trace: Option<&Path>) -> (usize, Option<i32>) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env("WARM_CRASH_CHILD_DIR", dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(spec) = failpoint {
        command.env("WARM_FAILPOINT", spec);
    }
    if let Some(path) = trace {
        command.env("WARM_FAILPOINT_TRACE", path);
    }
    let mut child = command.spawn().unwrap();
    let acked = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.contains("ACK "))
        .count();
    (acked, child.wait().unwrap().code())
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[test]
fn every_acknowledged_operation_survives_a_cut_at_every_durability_point() {
    let work = tempfile::tempdir().unwrap();
    let trace = work.path().join("trace");
    let (acked, code) = run_child(&work.path().join("count"), None, Some(&trace));
    let ops = plan(OPS);
    assert_eq!((acked, code), (ops.len(), Some(0)));
    let names: Vec<String> = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    for needed in [
        "segment-flush",
        "file-sync",
        "dir-sync",
        "atomic-write",
        "atomic-rename",
        "atomic-renamed",
        "scrub-overwrite",
        "scrub-unlink",
        "text-sync",
    ] {
        assert!(
            names.iter().any(|n| n == needed),
            "{needed} is never reached"
        );
    }
    let (stride, offset) = (
        env_usize("WARM_CRASH_STRIDE", 5),
        env_usize("WARM_CRASH_OFFSET", 0),
    );
    let mut cut = 0;
    for k in (1 + offset..=names.len()).step_by(stride) {
        let dir = work.path().join(format!("run{k}"));
        let (acked, code) = run_child(&dir, Some(&format!("exit@{k}")), None);
        assert_eq!(
            code,
            Some(writerslogic_warm::core::failpoint::EXIT_CODE),
            "exit@{k} ({}) did not end the child",
            names[k - 1]
        );
        cut += 1;
        let context = format!("exit@{k} ({}) after {acked} acks", names[k - 1]);
        check_reopened(
            &dir,
            &ops[..acked],
            &ops[acked..(acked + 1).min(ops.len())],
            &context,
        );
        // Recovery is idempotent and the store keeps working.
        let memory = open(&dir).unwrap();
        apply(&memory, Op::Remember(OPS + 1)).unwrap();
    }
    assert!(
        cut >= names.len() / stride / 2,
        "too few points exercised: {cut}"
    );
}
