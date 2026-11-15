// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Delete latency of `Memory::forget` by erasure mode and store size.
//!
//! `cargo run --release --example forget_cost -- <work-dir> <docs> <deletes> <mode>[,<mode>]`
//! with mode `logical` (`purge: false`), `deferred` (the default), `sync` (`synchronous: true`),
//! `batch100` (the default, 100 ids in one call, repeated `deletes / 100` times). Prints one JSON object
//! per mode with the median, p99 and total of the per-call latency in microseconds.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::WarmCore;

fn text(i: usize) -> String {
    format!(
        "note {i} about topic{} with detail{} and ordinary words for the index",
        i % 997,
        i % 31
    )
}

fn build(dir: &Path, docs: usize) -> Memory {
    let core = Arc::new(WarmCore::new(4096, Some(dir.display().to_string()), None).unwrap());
    let memory = Memory::new(core).with_clock(Arc::new(ManualClock::new(1)));
    for start in (0..docs).step_by(1000) {
        let inputs = (start..(start + 1000).min(docs))
            .map(|i| RememberInput {
                id: Some(format!("doc{i}")),
                text: text(i),
                ..RememberInput::default()
            })
            .collect();
        memory.remember_many(inputs).unwrap();
    }
    memory
}

fn forget(memory: &Memory, ids: Vec<String>, options: &ForgetOptions) -> f64 {
    let started = Instant::now();
    let report = memory.forget(&ForgetScope::Ids { ids }, options).unwrap();
    let micros = started.elapsed().as_secs_f64() * 1e6;
    assert!(!report.forgotten.is_empty());
    micros
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let work = Path::new(&args[1]);
    let docs: usize = args[2].parse().unwrap();
    let deletes: usize = args[3].parse().unwrap();
    for mode in args[4].split(',') {
        let dir = work.join(format!("{mode}-{docs}"));
        let _ = std::fs::remove_dir_all(&dir);
        let memory = build(&dir, docs);
        let options = match mode {
            "logical" => ForgetOptions {
                purge: false,
                ..ForgetOptions::default()
            },
            "sync" => ForgetOptions {
                synchronous: true,
                ..ForgetOptions::default()
            },
            _ => ForgetOptions::default(),
        };
        let mut samples = Vec::new();
        if mode == "batch100" {
            for call in 0..deletes / 100 {
                let ids = (0..100).map(|k| format!("doc{}", call * 100 + k)).collect();
                samples.push(forget(&memory, ids, &options));
            }
        } else {
            for i in 0..deletes {
                samples.push(forget(&memory, vec![format!("doc{i}")], &options));
            }
        }
        let total: f64 = samples.iter().sum();
        samples.sort_by(f64::total_cmp);
        let at = |q: f64| samples[((samples.len() as f64 * q) as usize).min(samples.len() - 1)];
        println!(
            "{{\"mode\":\"{mode}\",\"docs\":{docs},\"calls\":{},\"median_us\":{:.0},\"p99_us\":{:.0},\"max_us\":{:.0},\"total_ms\":{:.0}}}",
            samples.len(),
            at(0.5),
            at(0.99),
            samples[samples.len() - 1],
            total / 1000.0
        );
        drop(memory);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
