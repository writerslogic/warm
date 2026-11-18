// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// Separate test target, outside integration_tests: re-executes this binary by test name to kill a child, and arms process-wide failpoints.

//! Process death at every durability point of the experience core: the journal append and sync,
//! the snapshot write, the scrub of the files a checkpoint replaces, and the document store's
//! own points that a flush passes.
//!
//! A child process runs a fixed workload through the memory API and prints `ACK <n>` after
//! operation `n` returns. The parent counts the durability points of an uncut run, reruns the
//! workload once per point with the child ended at that point, reopens the store and checks:
//! the recovered state equals the state after exactly the acknowledged operations, or after
//! those plus the one in flight; no forgotten episode's text is in any file once the store is
//! reopened; and the store keeps working. This models a killed process, not power loss (see
//! `crash_harness.rs`).
//!
//! `WARM_CRASH_STRIDE=n` samples every n-th point; `WARM_CRASH_OFFSET=m` shifts the start.
#![cfg(feature = "failpoints")]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde_json::{json, Value};
use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::WarmCore;

const DIM: u32 = 4096;

/// Failpoint state is process-wide; the tests that use it take turns.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
const EPISODES: [&str; 3] = ["a", "b", "c"];

#[derive(Clone)]
enum Op {
    Remember(Vec<Value>),
    Flush,
    Forget(&'static str),
}

fn open(dir: &Path) -> Memory {
    let core = Arc::new(WarmCore::new(DIM, Some(dir.display().to_string()), None).unwrap());
    Memory::new(core).with_clock(Arc::new(ManualClock::new(1)))
}

fn step(ep: &str, n: u32, extra: u32, goal: Option<&str>) -> Value {
    let mut buttons: Vec<Value> = vec![
        json!({"role": "button", "name": format!("Zebrafish-{ep}"), "handle": "1"}),
        json!({"role": "button", "name": "Shared control", "handle": "2"}),
    ];
    for i in 0..extra {
        buttons.push(json!({"role": "link", "name": format!("link {ep} {i}")}));
    }
    let mut o = json!({
        "episode": ep, "step": n, "url": "http://shop/x",
        "tree": {"role": "RootWebArea", "name": format!("Page {ep}"), "children": buttons},
    });
    if n > 0 {
        o["action"] = json!({"call": "click('2')"});
    }
    if let Some(g) = goal {
        o["goal"] = json!(g);
    }
    o
}

fn plan() -> Vec<Op> {
    vec![
        Op::Remember(vec![step("a", 0, 0, None)]),
        Op::Remember(vec![step("a", 1, 1, None)]),
        Op::Remember(vec![step("b", 0, 0, None), step("b", 1, 2, None)]),
        Op::Flush,
        Op::Remember(vec![step("a", 2, 3, None)]),
        Op::Remember(vec![step("c", 0, 0, Some("buy things"))]),
        Op::Forget("b"),
        Op::Remember(vec![step("c", 1, 1, None)]),
        Op::Flush,
        Op::Forget("a"),
        Op::Remember(vec![step("c", 2, 2, None)]),
    ]
}

fn apply(m: &Memory, op: &Op) {
    match op {
        Op::Remember(steps) => {
            m.call("remember", json!({ "facts": steps })).unwrap();
        }
        Op::Flush => m.core().flush().unwrap(),
        Op::Forget(ep) => {
            m.call(
                "forget",
                json!({"scope": {"kind": "episode", "episode": ep}}),
            )
            .unwrap();
        }
    }
}

/// Everything observable about the experience state that does not depend on slot numbers.
fn fingerprint(m: &Memory) -> String {
    m.core()
        .with_experience(|s| {
            if s.stats().episodes == 0 && s.stats().regions == 0 {
                return "empty".to_string();
            }
            let mut out = format!("{:?}\n", s.stats());
            for ep in EPISODES {
                if let Some(v) = s.episode(ep) {
                    out.push_str(&format!("{ep}: {:?} {:?}\n", v.goal, v.outcome));
                    for p in &v.path {
                        out.push_str(&format!(
                            "  {} {:?} {} {} {} {}\n",
                            p.step,
                            p.action,
                            p.template.title_template,
                            p.template.url_template,
                            p.no_change,
                            p.error
                        ));
                    }
                }
            }
            let mut regions: Vec<String> = s
                .search_regions("Zebrafish Shared control link Page", 1000)
                .unwrap()
                .into_iter()
                .map(|h| format!("{}|{}|{}", h.text, h.episodes, h.steps))
                .collect();
            regions.sort();
            out.push_str(&regions.join("\n"));
            out
        })
        .unwrap_or_else(|| "empty".into())
}

fn model(ops: &[Op]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    for op in ops {
        apply(&m, op);
    }
    fingerprint(&m)
}

fn leaked(dir: &Path, needle: &str) -> bool {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        // The store's lock file is held open, and Windows refuses to read it.
        .filter(|e| e.path().extension().is_none_or(|x| x != "lock"))
        .any(|e| {
            std::fs::read(e.path())
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
        })
}

#[test]
fn crash_child() {
    let Ok(dir) = std::env::var("WARM_CRASH_CHILD_DIR") else {
        return;
    };
    let m = open(Path::new(&dir));
    for (n, op) in plan().iter().enumerate() {
        apply(&m, op);
        println!("ACK {n}");
    }
}

fn run_child(
    dir: &Path,
    failpoint: Option<&str>,
    trace: Option<&Path>,
) -> (usize, Option<i32>, String) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env("WARM_CRASH_CHILD_DIR", dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
    let mut stderr = String::new();
    let _ = std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr);
    (acked, child.wait().unwrap().code(), stderr)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[test]
fn every_acknowledged_experience_operation_survives_a_cut_at_every_durability_point() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let ops = plan();
    let models: Vec<String> = (0..=ops.len()).map(|n| model(&ops[..n])).collect();
    let work = tempfile::tempdir().unwrap();
    let trace = work.path().join("trace");
    let (acked, code, _) = run_child(&work.path().join("count"), None, Some(&trace));
    assert_eq!((acked, code), (ops.len(), Some(0)));
    let names: Vec<String> = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    for needed in [
        "experience-journal-append",
        "experience-journal-written",
        "experience-journal-synced",
        "experience-checkpoint-staged",
        "experience-checkpoint-old-removed",
        "experience-checkpoint-published",
        "experience-journal-removed",
        "scrub-overwrite",
    ] {
        assert!(
            names.iter().any(|n| n == needed),
            "{needed} is never reached"
        );
    }
    let (stride, offset) = (
        env_usize("WARM_CRASH_STRIDE", 1),
        env_usize("WARM_CRASH_OFFSET", 0),
    );
    let mut cut = 0;
    for k in (1 + offset..=names.len()).step_by(stride) {
        let dir = work.path().join(format!("run{k}"));
        let (acked, code, stderr) = run_child(&dir, Some(&format!("exit@{k}")), None);
        assert_eq!(
            code,
            Some(writerslogic_warm::core::failpoint::EXIT_CODE),
            "exit@{k} ({}) did not end the child; stderr: {stderr}",
            names[k - 1]
        );
        cut += 1;
        let context = format!("exit@{k} ({}) after {acked} acks", names[k - 1]);
        let m = open(&dir);
        let got = fingerprint(&m);
        let allowed = [&models[acked], &models[(acked + 1).min(ops.len())]];
        assert!(
            allowed.contains(&&got),
            "{context}: recovered state is neither the acknowledged state nor the one with the operation in flight\n--- got\n{got}\n--- acked\n{}\n--- in flight\n{}",
            models[acked],
            models[(acked + 1).min(ops.len())]
        );
        // A forgotten episode leaves no text behind once the store is open again.
        for ep in EPISODES {
            let present = m
                .core()
                .with_experience(|s| s.has_episode(ep))
                .unwrap_or(false);
            let ingested = plan()[..acked.min(ops.len())]
                .iter()
                .any(|op| matches!(op, Op::Remember(v) if v.iter().any(|s| s["episode"] == ep)));
            if ingested && !present {
                assert!(
                    !leaked(&dir, &format!("Zebrafish-{ep}")),
                    "{context}: text of forgotten episode {ep} remains"
                );
            }
        }
        drop(m);
        // Recovery is idempotent and the store keeps working.
        let again = open(&dir);
        assert_eq!(
            fingerprint(&again),
            got,
            "{context}: a second open changed the state"
        );
        again
            .call("remember", json!({"facts": [step("z", 0, 0, None)]}))
            .unwrap();
        drop(again);
        let again = open(&dir);
        assert!(
            again
                .core()
                .with_experience(|s| s.has_episode("z"))
                .unwrap(),
            "{context}"
        );
        drop(again);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    assert!(
        cut >= names.len() / stride / 2,
        "too few points exercised: {cut}"
    );
}

#[test]
fn a_failed_journal_write_leaves_the_state_unchanged_and_the_store_usable() {
    use writerslogic_warm::core::failpoint::{arm, disarm, Fault};
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    apply(&m, &Op::Remember(vec![step("a", 0, 0, None)]));
    let before = fingerprint(&m);
    for fault in [Fault::NoSpace, Fault::Io] {
        arm(fault, 1, false);
        let result = m.call("remember", json!({"facts": [step("a", 1, 1, None)]}));
        disarm();
        assert!(result.is_err(), "{fault:?}");
        assert_eq!(
            fingerprint(&m),
            before,
            "{fault:?}: the failed step is not visible"
        );
    }
    apply(&m, &Op::Remember(vec![step("a", 1, 1, None)]));
    drop(m);
    let m = open(dir.path());
    assert_eq!(m.core().with_experience(|s| s.stats().steps).unwrap(), 2);
}

/// A checkpoint that fails anywhere must not make a later acknowledged write unrecoverable. The
/// journal phase is the hard case: the snapshot is published and the old journal zeroed, and a
/// failed unlink once left that file in place for the next append.
#[test]
fn a_write_acknowledged_after_a_failed_checkpoint_survives_reopen() {
    use writerslogic_warm::core::experience::OwnedObservation;
    use writerslogic_warm::core::failpoint::{arm, disarm, hits, Fault};
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let obs =
        |n: u32| -> OwnedObservation { serde_json::from_value(step("a", n, 0, None)).unwrap() };
    let core = |dir: &Path| WarmCore::new(DIM, Some(dir.display().to_string()), None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    c.with_experience_mut(|s| s.ingest_many(vec![obs(0)]).unwrap())
        .unwrap();
    disarm();
    c.with_experience_mut(|s| s.checkpoint().unwrap()).unwrap();
    let points = hits();
    drop(c);
    assert!(points > 3, "{points}");
    for at in 1..=points {
        let dir = tempfile::tempdir().unwrap();
        let c = core(dir.path());
        c.with_experience_mut(|s| s.ingest_many(vec![obs(0)]).unwrap())
            .unwrap();
        arm(Fault::Io, at, false);
        let failed = c.with_experience_mut(|s| s.checkpoint()).unwrap().is_err();
        disarm();
        let second = c
            .with_experience_mut(|s| s.ingest_many(vec![obs(1)]))
            .unwrap();
        drop(c);
        let c = core(dir.path());
        let (zero, one) = c
            .with_experience(|s| (s.has_step("a", 0), s.has_step("a", 1)))
            .unwrap();
        assert!(
            zero,
            "point {at} (checkpoint failed: {failed}): step 0 lost"
        );
        if second.is_ok() {
            assert!(
                one,
                "point {at} (checkpoint failed: {failed}): acknowledged step 1 lost"
            );
        }
    }
}
