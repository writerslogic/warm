// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! ENOSPC and EIO at every durability point (see `core::failpoint`), in process.
//!
//! The same workload as `tests/crash_harness.rs` runs with the n-th durability point failing,
//! once (a transient error) or from then on (a disk that stays full or broken). The
//! operation that hit the error may fail; every operation that returned `Ok` must be on disk
//! after the store is reopened, and the failed one must be whole or absent.
//!
//! The failpoint state is process global, so this file holds a single test.
#![cfg(feature = "failpoints")]

mod erase_common;

use erase_common::*;
use writerslogic_warm::core::failpoint::{arm, disarm, hits, Fault};

const OPS: usize = 14;

/// Runs the plan with the current failpoint arming; returns the acknowledged prefix and the
/// failed operations.
fn run(dir: &std::path::Path, stop_at_first_error: bool) -> (Vec<Op>, Vec<Op>) {
    let (mut acked, mut failed) = (Vec::new(), Vec::new());
    let Ok(memory) = open(dir) else {
        return (acked, failed);
    };
    for op in plan(OPS) {
        match apply(&memory, op) {
            Ok(()) => acked.push(op),
            Err(_) => {
                failed.push(op);
                if stop_at_first_error {
                    break;
                }
            }
        }
    }
    (acked, failed)
}

#[test]
fn acknowledged_operations_survive_enospc_and_eio_at_every_point() {
    let work = tempfile::tempdir().unwrap();
    // Count the points with a fault that never fires.
    arm(Fault::Io, u64::MAX, false);
    let (acked, failed) = run(&work.path().join("count"), true);
    assert!(failed.is_empty() && acked.len() == plan(OPS).len());
    let total = hits();
    disarm();
    assert!(total > 50, "{total}");
    let stride: u64 = std::env::var("WARM_FAULT_STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let mut injected = 0;
    for (fault, label) in [(Fault::NoSpace, "enospc"), (Fault::Io, "eio")] {
        for sticky in [false, true] {
            for k in (1..=total).step_by(stride as usize) {
                let dir = work.path().join(format!("{label}-{sticky}-{k}"));
                arm(fault, k, sticky);
                let (acked, failed) = run(&dir, sticky);
                // The disk stays broken through the close, as it would in life.
                let seen = hits();
                disarm();
                if failed.is_empty() && seen < k {
                    continue;
                }
                injected += 1;
                let context = format!(
                    "{label} sticky={sticky} at point {k}, acked {}",
                    acked.len()
                );
                check_reopened(&dir, &acked, &failed, &context);
            }
        }
    }
    assert!(injected as u64 >= total / stride, "{injected}");
}
