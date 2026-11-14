// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Shared workload and scanners for the erase, crash and disk-fault tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::WarmCore;

pub const DIM: u32 = 4096;

pub fn open(path: &Path) -> anyhow::Result<Memory> {
    let core = Arc::new(WarmCore::new(DIM, Some(path.display().to_string()), None)?);
    Ok(Memory::new(core).with_clock(Arc::new(ManualClock::new(1))))
}

pub fn files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Files under `dir` containing the first ten bytes of `needle` (a dictionary or delta list
/// that stored a term or its prefix in the clear would match the fragment too).
pub fn leaks(dir: &Path, needle: &str) -> Vec<String> {
    let fragment = &needle.as_bytes()[..needle.len().min(10)];
    files(dir)
        .into_iter()
        .filter(|path| {
            std::fs::read(path)
                .map(|bytes| bytes.windows(fragment.len()).any(|w| w == fragment))
                .unwrap_or(false)
        })
        .map(|path| path.display().to_string())
        .collect()
}

fn mix(i: u64) -> u64 {
    let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A letters-only token unique to document `i`: it survives tokenisation and stemming, so a
/// recall for it finds exactly that document.
pub fn token(i: usize) -> String {
    let mut z = mix(i as u64 + 1);
    let mut word = String::from("zq");
    for _ in 0..12 {
        word.push((b'a' + (z % 26) as u8) as char);
        z /= 26;
        if z == 0 {
            z = mix(i as u64 + 77);
        }
    }
    word
}

pub fn text(i: usize) -> String {
    format!("note about {} and basalt gabbro schist", token(i))
}

pub fn id(i: usize) -> String {
    format!("doc{i}")
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Remember(usize),
    Forget(usize),
    Compact,
}

/// Remember `n` documents; every third remember is followed by a forget of the document two
/// back, and a compaction runs after the fifth.
pub fn plan(n: usize) -> Vec<Op> {
    let mut ops = Vec::new();
    for i in 0..n {
        ops.push(Op::Remember(i));
        if i == 4 {
            ops.push(Op::Compact);
        }
        if i % 3 == 2 {
            ops.push(Op::Forget(i - 2));
        }
    }
    ops
}

pub fn apply(memory: &Memory, op: Op) -> anyhow::Result<()> {
    match op {
        Op::Remember(i) => {
            memory.remember(RememberInput {
                id: Some(id(i)),
                text: text(i),
                ..RememberInput::default()
            })?;
        }
        Op::Forget(i) => {
            let report = memory.forget(
                &ForgetScope::Ids { ids: vec![id(i)] },
                &ForgetOptions::default(),
            )?;
            anyhow::ensure!(report.purged, "forget did not purge");
        }
        Op::Compact => memory.core().compact()?,
    }
    Ok(())
}

/// Verifies a reopened store against what was acknowledged. `acked` ops returned `Ok`;
/// `doubtful` ops started and did not return `Ok` (they may or may not have taken effect,
/// but never partially).
pub fn check_reopened(dir: &Path, acked: &[Op], doubtful: &[Op], context: &str) {
    let memory = open(dir).unwrap_or_else(|e| panic!("{context}: reopen failed: {e:#}"));
    let mut present = std::collections::BTreeMap::new();
    let mut gone = std::collections::BTreeSet::new();
    for op in acked {
        match *op {
            Op::Remember(i) => {
                present.insert(i, true);
                gone.remove(&i);
            }
            Op::Forget(i) => {
                present.remove(&i);
                gone.insert(i);
            }
            Op::Compact => {}
        }
    }
    let maybe: std::collections::BTreeSet<usize> = doubtful
        .iter()
        .filter_map(|op| match *op {
            Op::Remember(i) | Op::Forget(i) => Some(i),
            Op::Compact => None,
        })
        .collect();
    for &i in present.keys() {
        if maybe.contains(&i) {
            continue;
        }
        let fact = memory
            .get(&id(i))
            .unwrap()
            .unwrap_or_else(|| panic!("{context}: acknowledged {} is missing", id(i)));
        assert_eq!(fact.text.as_deref(), Some(text(i).as_str()), "{context}");
    }
    for &i in &gone {
        if maybe.contains(&i) {
            continue;
        }
        assert!(
            memory.get(&id(i)).unwrap().is_none(),
            "{context}: forgotten {} came back",
            id(i)
        );
    }
    // No partial state: every document the store holds is whole and found by its own token,
    // and the doubtful ones are either absent or complete.
    let mut every = maybe.clone();
    every.extend(present.keys().copied());
    for i in every {
        if let Some(fact) = memory.get(&id(i)).unwrap() {
            assert_eq!(
                fact.text.as_deref(),
                Some(text(i).as_str()),
                "{context}: partial {}",
                id(i)
            );
            let found = memory
                .recall(&token(i), &RecallOptions::default())
                .unwrap()
                .into_iter()
                .any(|hit| hit.id == id(i));
            assert!(found, "{context}: {} is stored but not searchable", id(i));
        }
    }
    // Deleted text is in no file once the store is open: either the delete never committed
    // (the document is still here) or it is erased.
    for &i in &gone {
        if memory.get(&id(i)).unwrap().is_none() {
            let found = leaks(dir, &token(i));
            assert!(
                found.is_empty(),
                "{context}: {} text remains in {found:?}",
                id(i)
            );
        }
    }
    for i in maybe {
        if memory.get(&id(i)).unwrap().is_none() && !present.contains_key(&i) {
            // A forget that committed but was not acknowledged also erases on reopen.
            let found = leaks(dir, &token(i));
            if gone.contains(&i)
                || doubtful
                    .iter()
                    .any(|op| matches!(op, Op::Forget(j) if *j == i))
            {
                assert!(
                    found.is_empty(),
                    "{context}: {} text remains in {found:?}",
                    id(i)
                );
            }
        }
    }
}
