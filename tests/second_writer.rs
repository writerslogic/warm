// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The concurrency model: one engine (one process) owns a store directory and serves any
//! number of reader and writer threads; a second engine, in this or another process, is
//! refused while the first is open and never disturbs it. There is no read-only second
//! opener.

mod erase_common;

use erase_common::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::WarmCore;

fn refusal(dir: &Path) -> String {
    match WarmCore::new(DIM, Some(dir.display().to_string()), None) {
        Ok(_) => panic!("a second engine opened a store that is already open"),
        Err(error) => format!("{error:#}"),
    }
}

#[test]
fn a_second_engine_is_refused_and_leaves_the_first_intact() {
    let dir = tempfile::tempdir().unwrap();
    let memory = open(dir.path()).unwrap();
    apply(&memory, Op::Remember(0)).unwrap();
    assert!(refusal(dir.path()).contains("already open"));
    // The refused attempt wrote nothing the first engine can trip over.
    apply(&memory, Op::Remember(1)).unwrap();
    apply(&memory, Op::Forget(0)).unwrap();
    assert!(memory.get(&id(1)).unwrap().is_some());
    drop(memory);
    let memory = open(dir.path()).unwrap();
    assert!(memory.get(&id(0)).unwrap().is_none());
    assert_eq!(
        memory.get(&id(1)).unwrap().unwrap().text.as_deref(),
        Some(text(1).as_str())
    );
}

/// Child: tries to open the store the parent holds and reports the outcome.
#[test]
fn second_writer_child() {
    let Ok(dir) = std::env::var("WARM_SECOND_WRITER_DIR") else {
        return;
    };
    match WarmCore::new(DIM, Some(dir), None) {
        Ok(_) => println!("RESULT opened"),
        Err(error) => println!("RESULT refused {error:#}"),
    }
}

#[test]
fn a_second_process_is_refused_while_the_first_writes() {
    let dir = tempfile::tempdir().unwrap();
    let memory = open(dir.path()).unwrap();
    apply(&memory, Op::Remember(0)).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "second_writer_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("WARM_SECOND_WRITER_DIR", dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("RESULT refused") && stdout.contains("already open"),
        "{stdout}"
    );
    apply(&memory, Op::Remember(1)).unwrap();
    drop(memory);
    // After the owner closes, another process may take the store.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "second_writer_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("WARM_SECOND_WRITER_DIR", dir.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("RESULT opened"));
}

#[test]
fn readers_see_whole_facts_while_a_writer_commits() {
    let dir = tempfile::tempdir().unwrap();
    let memory = Arc::new(open(dir.path()).unwrap());
    const WRITTEN: usize = 60;
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (memory, done) = (Arc::clone(&memory), Arc::clone(&done));
            std::thread::spawn(move || {
                let mut seen = 0usize;
                while !done.load(Ordering::Acquire) {
                    for i in 0..WRITTEN {
                        if let Some(fact) = memory.get(&id(i)).unwrap() {
                            assert_eq!(fact.text.as_deref(), Some(text(i).as_str()));
                            seen += 1;
                        }
                    }
                    let _ = memory.recall("basalt", &RecallOptions::default()).unwrap();
                }
                seen
            })
        })
        .collect();
    for i in 0..WRITTEN {
        apply(&memory, Op::Remember(i)).unwrap();
        if i % 5 == 4 {
            apply(&memory, Op::Forget(i - 4)).unwrap();
        }
    }
    done.store(true, Ordering::Release);
    for reader in readers {
        reader.join().unwrap();
    }
    assert!(memory.get(&id(WRITTEN - 1)).unwrap().is_some());
}
