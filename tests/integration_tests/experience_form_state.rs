// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Form state in the experience core: controls parsed at ingest from the accessibility-tree
//! text of an observation, journaled, checkpointed and reopened, rendered as the reference
//! renderer of the LongMemEval-V2 adapter renders them, and the snapshot versions around it.

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use writerslogic_warm::core::experience::{parse_indented, TreeNode};
use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::WarmCore;

const DIM: u32 = 4096;

fn open_with(dir: &Path, config: WarmConfig) -> Memory {
    let core = Arc::new(WarmCore::new(DIM, Some(dir.display().to_string()), Some(config)).unwrap());
    Memory::new(core).with_clock(Arc::new(ManualClock::new(1)))
}

fn open(dir: &Path) -> Memory {
    open_with(dir, WarmConfig::default())
}

fn tree_of(accessibility_tree: &str) -> Value {
    let tree =
        parse_indented(accessibility_tree).unwrap_or_else(|| TreeNode::new("RootWebArea", ""));
    serde_json::to_value(tree).unwrap()
}

fn observation(episode: &str, step: u32, url: &str, action: Option<&str>, page: &str) -> Value {
    let mut o = json!({
        "episode": episode, "step": step, "url": url,
        "tree": tree_of(page), "accessibilityTree": page,
    });
    if let Some(a) = action.filter(|a| !a.trim().is_empty()) {
        o["action"] = json!({ "call": a });
    }
    o
}

#[derive(Deserialize)]
struct Fixture {
    groups: Vec<Group>,
}

#[derive(Deserialize)]
struct Group {
    trajectory: String,
    states: Vec<State>,
    expected: Vec<String>,
}

#[derive(Deserialize)]
struct State {
    step: u32,
    url: String,
    action: Option<String>,
    accessibility_tree: String,
}

fn fixture() -> Fixture {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/experience/form_parity.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn tables(m: &Memory, g: &Group) -> Vec<String> {
    let states: Vec<(&str, u32)> = g
        .states
        .iter()
        .map(|s| (g.trajectory.as_str(), s.step))
        .collect();
    m.core().experience_form_tables(&states)
}

fn snapshot_version(dir: &Path) -> u8 {
    std::fs::read(dir.join("experience.wexp")).unwrap()[4]
}

/// Real states from the public LongMemEval-V2 trajectories, ingested through `remember`: the
/// engine's tables equal the reference renderer's blocks byte for byte, from the journal, from
/// the snapshot, and after reopening each.
#[test]
fn form_tables_match_the_reference_renderer_through_journal_and_snapshot() {
    let f = fixture();
    let states: usize = f.groups.iter().map(|g| g.states.len()).sum();
    assert!(states >= 20, "{states}");
    assert!(f
        .groups
        .iter()
        .flat_map(|g| &g.expected)
        .any(|e| e.contains("unchanged from an earlier state")));
    assert!(f
        .groups
        .iter()
        .flat_map(|g| &g.expected)
        .any(|e| e.contains("more)")));

    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    for g in &f.groups {
        for s in &g.states {
            m.call(
                "remember",
                observation(
                    &g.trajectory,
                    s.step,
                    &s.url,
                    s.action.as_deref(),
                    &s.accessibility_tree,
                ),
            )
            .unwrap();
        }
    }
    let check = |m: &Memory, when: &str| {
        for g in &f.groups {
            let got = tables(m, g);
            for (i, (got, want)) in got.iter().zip(&g.expected).enumerate() {
                assert_eq!(got, want, "{when}: trajectory {} state {i}", g.trajectory);
            }
        }
    };
    check(&m, "after ingest");
    drop(m);
    // Nothing checkpointed yet: the controls come back from the journal.
    assert!(!dir.path().join("experience.wexp").exists());
    let m = open(dir.path());
    check(&m, "journal replay");
    m.core().flush().unwrap();
    assert_eq!(snapshot_version(dir.path()), 3);
    drop(m);
    let m = open(dir.path());
    check(&m, "snapshot reopen");
}

#[test]
fn the_page_text_is_not_stored_only_the_controls() {
    let page = "RootWebArea 'Checkout'\n\t[3] StaticText 'zq-unstored-marker'\n\t[4] textbox 'Card', value='zq-stored-value'";
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    m.call(
        "remember",
        json!({"episode": "e", "step": 0, "tree": {"role": "RootWebArea", "name": "Checkout"}, "accessibilityTree": page}),
    )
    .unwrap();
    let journal = std::fs::read(dir.path().join("experience.wexj")).unwrap();
    let has = |b: &[u8], s: &str| b.windows(s.len()).any(|w| w == s.as_bytes());
    assert!(has(&journal, "zq-stored-value"));
    assert!(!has(&journal, "zq-unstored-marker"));
    m.core().flush().unwrap();
    let snap = std::fs::read(dir.path().join("experience.wexp")).unwrap();
    assert!(has(&snap, "zq-stored-value") && !has(&snap, "zq-unstored-marker"));
}

#[test]
fn a_store_without_controls_keeps_writing_version_2() {
    for (form_state, page, version) in [
        (true, None, 2),
        (
            true,
            Some("RootWebArea 'x'\n\t[1] StaticText 'no controls'"),
            2,
        ),
        (false, Some("[1] textbox 'Name'"), 2),
        (true, Some("[1] textbox 'Name'"), 3),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut config = WarmConfig::default();
        config.experience.form_state = form_state;
        let m = open_with(dir.path(), config.clone());
        let mut o =
            json!({"episode": "e", "step": 0, "tree": {"role": "RootWebArea", "name": "x"}});
        if let Some(p) = page {
            o["accessibilityTree"] = json!(p);
        }
        m.call("remember", o).unwrap();
        m.core().flush().unwrap();
        assert_eq!(
            snapshot_version(dir.path()),
            version,
            "{form_state} {page:?}"
        );
        let rows = m.core().experience_form_tables(&[("e", 0)]);
        assert_eq!(rows[0].is_empty(), version == 2);
        drop(m);
        let m = open_with(dir.path(), config);
        assert_eq!(m.core().experience_form_tables(&[("e", 0)]), rows);
    }
}

#[test]
fn version_2_snapshot_opens_and_takes_controls() {
    // Written by the release before form state (format version 2, no controls).
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/experience/snapshot_v2.wexp"),
        dir.path().join("experience.wexp"),
    )
    .unwrap();
    let m = open(dir.path());
    let steps = m.core().with_experience(|s| s.stats().steps).unwrap();
    assert!(steps > 0);
    m.call(
        "remember",
        json!({"episode": "new", "step": 0, "tree": {"role": "RootWebArea", "name": "Form"},
               "accessibilityTree": "[7] checkbox 'Subscribe', checked=true"}),
    )
    .unwrap();
    m.core().flush().unwrap();
    assert_eq!(snapshot_version(dir.path()), 3);
    drop(m);
    let m = open(dir.path());
    assert_eq!(
        m.core().with_experience(|s| s.stats().steps).unwrap(),
        steps + 1
    );
    assert_eq!(
        m.core().experience_form_tables(&[("new", 0)])[0],
        "- Form controls (structured from the page text; * = selected option):\n  checkbox 'Subscribe' checked=true\n"
    );
}

#[test]
fn a_retried_step_must_carry_the_same_controls() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = open(dir.path());
    let o = |page: &str| json!({"episode": "e", "step": 0, "tree": {"role": "RootWebArea"}, "accessibilityTree": page});
    let first = m.call("remember", o("[1] textbox 'A'")).unwrap();
    assert_eq!(first["remembered"][0]["created"], true);
    let again = m.call("remember", o("[1] textbox 'A'")).unwrap();
    assert_eq!(again["remembered"][0]["created"], false);
    let conflict = |m: &Memory| {
        let err = m.call("remember", o("[1] textbox 'B'")).unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<MemoryError>(),
                Some(MemoryError::Conflict(_))
            ),
            "{err}"
        );
    };
    conflict(&m);
    // After a reopen from the journal, then from the snapshot, the stored controls still
    // decide: the same page is a duplicate and another one a conflict.
    for checkpoint in [false, true] {
        if checkpoint {
            m.core().flush().unwrap();
        }
        drop(m);
        m = open(dir.path());
        let again = m.call("remember", o("[1] textbox 'A'")).unwrap();
        assert_eq!(again["remembered"][0]["created"], false, "{checkpoint}");
        conflict(&m);
    }
}

#[test]
fn accessibility_tree_bytes_are_bounded_before_parsing() {
    const LIMIT: usize = 64;
    let dir = tempfile::tempdir().unwrap();
    let mut config = WarmConfig::default();
    config.experience.max_page_bytes = LIMIT;
    let m = open_with(dir.path(), config);
    let line = "[1] textbox 'x'";
    for (step, len) in [(0u32, LIMIT - 1), (1, LIMIT), (2, LIMIT + 1)] {
        let page = format!("{line}{}", " ".repeat(len - line.len()));
        assert_eq!(page.len(), len);
        let r = m.call(
            "remember",
            json!({"episode": "e", "step": step, "tree": {"role": "RootWebArea"}, "accessibilityTree": page}),
        );
        if len <= LIMIT {
            r.unwrap();
        } else {
            let err = r.unwrap_err();
            assert!(
                matches!(
                    err.downcast_ref::<MemoryError>(),
                    Some(MemoryError::Invalid(_))
                ),
                "{err}"
            );
        }
    }
    assert_eq!(m.core().with_experience(|s| s.stats().steps).unwrap(), 2);
}
