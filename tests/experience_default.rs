// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The experience core on the default memory path: routing by shape, one merged recall,
//! provenance in explain, forget of an episode, and durability across reopen.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::WarmCore;

const DIM: u32 = 4096;

fn open(dir: &Path) -> Memory {
    let core = Arc::new(WarmCore::new(DIM, Some(dir.display().to_string()), None).unwrap());
    Memory::new(core).with_clock(Arc::new(ManualClock::new(1)))
}

fn tree(title: &str, buttons: &[&str], handles: bool) -> Value {
    let children: Vec<Value> = buttons
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut n = json!({"role": "button", "name": b});
            if handles {
                n["handle"] = json!(format!("{}", i + 1));
            }
            n
        })
        .collect();
    json!({"role": "RootWebArea", "name": title, "children": children})
}

fn step(episode: &str, n: u32, title: &str, buttons: &[&str], action: Option<Value>) -> Value {
    let mut o = json!({
        "episode": episode, "step": n, "url": "http://shop/orders/12",
        "tree": tree(title, buttons, true),
    });
    if let Some(a) = action {
        o["action"] = a;
    }
    o
}

fn remember(m: &Memory, v: Value) -> Value {
    m.call("remember", v).unwrap()
}

fn files_with(dir: &Path, needle: &str) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        // The store's lock file is held open, and Windows refuses to read it.
        if p.is_file()
            && p.extension().is_none_or(|x| x != "lock")
            && std::fs::read(&p)
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
        {
            found.push(p.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    found
}

fn experience_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("experience"))
        .collect()
}

#[test]
fn text_only_store_has_no_experience_state_and_recalls_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    remember(&m, json!({"id": "a", "text": "the quick brown fox jumps"}));
    remember(
        &m,
        json!({"facts": [{"id": "b", "text": "a lazy dog sleeps"}]}),
    );
    let results = m.call("recall", json!({"query": "fox dog"})).unwrap();
    let ids: Vec<&str> = results["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    m.core().flush().unwrap();
    assert!(experience_files(dir.path()).is_empty());
}

#[test]
fn observations_route_by_shape_and_recall_merges_with_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    remember(
        &m,
        json!({"id": "note", "text": "the checkout page lists a Place order button"}),
    );
    let r = remember(
        &m,
        json!({"facts": [
            step("ep1", 0, "Checkout", &["Place order", "Cancel"], None),
            {"id": "note2", "text": "remember to apply the discount code"},
            step("ep1", 1, "Checkout", &["Place order", "Cancel", "Order placed"], Some(json!({"label": "click Place order"}))),
        ]}),
    );
    let ids: Vec<&str> = r["remembered"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["experience:ep1:0", "note2", "experience:ep1:1"]);
    // Retrying the same step changes nothing.
    let again = remember(
        &m,
        step(
            "ep1",
            1,
            "Checkout",
            &["Place order", "Cancel", "Order placed"],
            Some(json!({"label": "click Place order"})),
        ),
    );
    assert_eq!(again["remembered"][0]["created"], false);

    let out = m
        .call(
            "recall",
            json!({"query": "Place order button", "k": 5, "explain": true}),
        )
        .unwrap();
    let results = out["results"].as_array().unwrap();
    let region = results
        .iter()
        .find(|r| r["id"].as_str().unwrap().starts_with("experience:r"))
        .expect("a region");
    assert_eq!(region["metadata"]["kind"], "experience-region");
    assert!(results.iter().any(|r| r["id"] == "note"));
    let scores: Vec<f64> = results
        .iter()
        .map(|r| r["score"].as_f64().unwrap())
        .collect();
    assert!(
        scores.windows(2).all(|w| w[0] >= w[1]),
        "one ordering: {scores:?}"
    );
    let prov = &region["explanation"]["provenance"];
    assert_eq!(prov[0]["episode"], "ep1");

    let id = region["id"].as_str().unwrap();
    let e = m
        .call("explain", json!({"query": "Place order button", "id": id}))
        .unwrap();
    assert_eq!(e["explanation"]["provenance"][0]["episode"], "ep1");
    assert_eq!(e["explanation"]["retrieved"], true);

    // Filters regions do not have leave the document results alone.
    let filtered = m
        .call(
            "recall",
            json!({"query": "Place order button", "subject": "none"}),
        )
        .unwrap();
    assert!(filtered["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| !r["id"].as_str().unwrap().starts_with("experience:")));
}

#[test]
fn oversized_and_malformed_observations_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    let mut t = json!({"role": "leaf", "name": "x"});
    for _ in 0..200 {
        t = json!({"role": "group", "name": "g", "children": [t]});
    }
    let err = m
        .call("remember", json!({"episode": "deep", "step": 0, "tree": t}))
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<MemoryError>(),
            Some(MemoryError::Invalid(_))
        ),
        "{err:#}"
    );
    assert!(
        experience_files(dir.path()).is_empty(),
        "a request refused before ingest leaves no files"
    );
    let err = m
        .call(
            "remember",
            json!({"episode": "bad", "step": 0, "tree": {"role": "", "name": "x"}}),
        )
        .unwrap_err();
    assert!(matches!(
        err.downcast_ref::<MemoryError>(),
        Some(MemoryError::Invalid(_))
    ));
    let err = m
        .call(
            "remember",
            json!({"episode": "u", "step": 0, "tree": {"role": "a"}, "bogus": 1}),
        )
        .unwrap_err();
    assert!(matches!(
        err.downcast_ref::<MemoryError>(),
        Some(MemoryError::Invalid(_))
    ));
}

#[test]
fn acknowledged_steps_survive_reopen_and_labels_resolve_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    {
        let m = open(dir.path());
        remember(&m, step("ep1", 0, "Cart", &["Checkout", "Remove"], None));
        remember(
            &m,
            step(
                "ep1",
                1,
                "Checkout",
                &["Pay"],
                Some(json!({"call": "click('1')"})),
            ),
        );
        // Dropped without a flush: the journal carries both steps.
    }
    let m = open(dir.path());
    let hits = m
        .core()
        .with_experience(|s| s.search_regions("Checkout Pay", 5).unwrap())
        .unwrap();
    assert!(!hits.is_empty());
    let label = m
        .core()
        .with_experience(|s| s.episode("ep1").unwrap().path[1].action.clone())
        .unwrap();
    assert_eq!(label.as_deref(), Some("click button 'Checkout'"));
    // A step after the reload resolves a handle of the step before it.
    remember(
        &m,
        step(
            "ep1",
            2,
            "Paid",
            &["Done"],
            Some(json!({"call": "click('1')"})),
        ),
    );
    m.core().flush().unwrap();
    let journal = std::fs::metadata(dir.path().join("experience.wexj"))
        .unwrap()
        .len();
    assert_eq!(journal, 8, "a checkpoint empties the journal");
    drop(m);
    let m = open(dir.path());
    let label = m
        .core()
        .with_experience(|s| s.episode("ep1").unwrap().path[2].action.clone())
        .unwrap();
    assert_eq!(label.as_deref(), Some("click button 'Pay'"));
    let stats = m.core().with_experience(|s| s.stats()).unwrap();
    assert_eq!((stats.episodes, stats.steps), (1, 3));
}

#[test]
fn forget_episode_removes_it_and_scrubs_every_file() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    remember(
        &m,
        step("keep", 0, "Home", &["Open", "Shared control"], None),
    );
    remember(
        &m,
        step(
            "gone",
            0,
            "Secret",
            &["Zebrafish launch codes", "Shared control"],
            None,
        ),
    );
    m.core().flush().unwrap();
    remember(
        &m,
        step(
            "gone",
            1,
            "Secret",
            &["Zebrafish launch codes", "More"],
            Some(json!({"label": "click More"})),
        ),
    );
    assert!(
        !files_with(dir.path(), "Zebrafish").is_empty(),
        "the text is stored before the forget"
    );
    let dry = m
        .call(
            "forget",
            json!({"scope": {"kind": "episode", "episode": "gone"}, "dryRun": true}),
        )
        .unwrap();
    assert_eq!(dry["forgotten"][0], "experience:episode:gone");
    assert!(m.core().with_experience(|s| s.has_episode("gone")).unwrap());
    let done = m
        .call(
            "forget",
            json!({"scope": {"kind": "episode", "episode": "gone"}}),
        )
        .unwrap();
    assert_eq!(done["forgotten"][0], "experience:episode:gone");
    assert_eq!(done["purged"], true);
    assert!(!m.core().with_experience(|s| s.has_episode("gone")).unwrap());
    assert!(m.core().with_experience(|s| s.has_episode("keep")).unwrap());
    let hits = m
        .core()
        .with_experience(|s| s.search_regions("Zebrafish launch codes", 5).unwrap())
        .unwrap();
    assert!(hits.iter().all(|h| !h.text.contains("Zebrafish")));
    assert!(
        files_with(dir.path(), "Zebrafish").is_empty(),
        "{:?}",
        files_with(dir.path(), "Zebrafish")
    );
    drop(m);
    let m = open(dir.path());
    assert!(!m.core().with_experience(|s| s.has_episode("gone")).unwrap());
    let again = m
        .call(
            "forget",
            json!({"scope": {"kind": "episode", "episode": "gone"}}),
        )
        .unwrap();
    assert_eq!(again["forgotten"].as_array().unwrap().len(), 0);
}

#[test]
fn version_1_snapshot_opens() {
    // Written by the release that introduced the experience core (format version 1, no journal).
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/experience/snapshot_v1.wexp"),
        dir.path().join("experience.wexp"),
    )
    .unwrap();
    let m = open(dir.path());
    let (steps, hits) = m
        .core()
        .with_experience(|s| {
            (
                s.stats().steps,
                s.search_regions("Legacy button", 3).unwrap().len(),
            )
        })
        .unwrap();
    assert_eq!(steps, 2);
    assert!(hits > 0);
    remember(&m, step("new", 0, "Home", &["Fresh button"], None));
    m.core().flush().unwrap();
    drop(m);
    let m = open(dir.path());
    assert_eq!(m.core().with_experience(|s| s.stats().steps).unwrap(), 3);
}

#[cfg(feature = "security")]
#[test]
fn an_encrypted_store_keeps_observations_sealed_and_reopens() {
    use writerslogic_warm::core::WarmConfig;
    let dir = tempfile::tempdir().unwrap();
    let open_encrypted = |dir: &Path| {
        let mut config = WarmConfig::default();
        config.security.encryption_enabled = true;
        config.security.encryption_passphrase = Some("experience-default-fixture".into());
        let core =
            Arc::new(WarmCore::new(DIM, Some(dir.display().to_string()), Some(config)).unwrap());
        Memory::new(core).with_clock(Arc::new(ManualClock::new(1)))
    };
    {
        let m = open_encrypted(dir.path());
        remember(&m, step("ep", 0, "Vault", &["Zebrafish secret"], None));
        remember(
            &m,
            step(
                "ep",
                1,
                "Vault",
                &["Zebrafish secret", "Open"],
                Some(json!({"label": "click Open"})),
            ),
        );
        // Steps 0 and 1 are in the journal; checkpoint after step 1 so the snapshot is sealed too.
        m.core().flush().unwrap();
        remember(
            &m,
            step(
                "ep",
                2,
                "Vault",
                &["Zebrafish secret", "Open", "Close"],
                Some(json!({"label": "click Close"})),
            ),
        );
    }
    assert!(
        files_with(dir.path(), "Zebrafish").is_empty(),
        "snapshot and journal are sealed"
    );
    let m = open_encrypted(dir.path());
    assert_eq!(m.core().with_experience(|s| s.stats().steps).unwrap(), 3);
    let hits = m
        .core()
        .with_experience(|s| s.search_regions("Zebrafish secret", 3).unwrap())
        .unwrap();
    assert!(!hits.is_empty());
}

fn shared_page(title: &str, typed: &str, extra: Option<&str>) -> Value {
    let links: Vec<Value> = (0..80)
        .map(
            |i| json!({"role": "link", "name": format!("navigation entry number {i} of the site")}),
        )
        .collect();
    let mut kids = vec![
        json!({"role": "navigation", "name": "site menu", "children": links}),
        json!({"role": "form", "name": "profile form", "children": [
            {"role": "textbox", "name": "display name", "props": format!("value='{typed}'")},
            {"role": "button", "name": "save profile"},
        ]}),
    ];
    if let Some(x) = extra {
        kids.push(json!({"role": "button", "name": x}));
    }
    json!({"role": "RootWebArea", "name": title, "children": kids})
}

#[test]
fn forget_scrubs_what_a_surviving_shared_region_took_from_the_episode() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    let obs =
        |ep: &str, n: u32, title: &str, typed: &str, extra: Option<&str>, a: Option<Value>| {
            let mut o = json!({"episode": ep, "step": n, "url": "http://shop/profile",
            "tree": shared_page(title, typed, extra)});
            if let Some(a) = a {
                o["action"] = a;
            }
            o
        };
    remember(
        &m,
        obs(
            "gone",
            0,
            "Quokkatitle only here",
            "Quokkavalue",
            None,
            None,
        ),
    );
    remember(
        &m,
        obs(
            "gone",
            1,
            "Quokkatitle only here",
            "Quokkavalue",
            Some("Quokkaline"),
            Some(json!({"label": "click save"})),
        ),
    );
    remember(&m, obs("keep", 0, "Plain page", "ordinary", None, None));
    m.core().flush().unwrap();
    for needle in ["Quokkatitle", "Quokkavalue", "Quokkaline"] {
        assert!(
            !files_with(dir.path(), needle).is_empty(),
            "{needle} stored"
        );
    }
    m.call(
        "forget",
        json!({"scope": {"kind": "episode", "episode": "gone"}}),
    )
    .unwrap();
    for needle in ["Quokkatitle", "Quokkavalue", "Quokkaline"] {
        assert!(
            files_with(dir.path(), needle).is_empty(),
            "{needle} left in {:?}",
            files_with(dir.path(), needle)
        );
    }
    let hits = m
        .core()
        .with_experience(|s| s.search_regions("navigation entry", 5).unwrap())
        .unwrap();
    assert!(!hits.is_empty(), "the shared region survives");
    assert!(hits.iter().all(|h| !h.text.contains("Quokka")));
    drop(m);
    let m = open(dir.path());
    let hits = m
        .core()
        .with_experience(|s| s.search_regions("navigation entry", 5).unwrap())
        .unwrap();
    assert!(!hits.is_empty());
}

fn chain(depth: usize) -> Value {
    let mut n = json!({"role": "button", "name": "leaf of the chain"});
    for i in 1..depth {
        n = json!({"role": "group", "name": format!("level {i}"), "children": [n]});
    }
    n
}

#[test]
fn trees_at_the_depth_limit_survive_a_crash_before_checkpoint() {
    let limit = 128;
    for depth in [limit - 1, limit] {
        let dir = tempfile::tempdir().unwrap();
        let m = open(dir.path());
        remember(
            &m,
            json!({"episode": "deep", "step": 0, "url": "http://x/d", "tree": chain(depth)}),
        );
        // No flush: the only copy is the journal record.
        drop(m);
        let m = open(dir.path());
        assert!(
            m.core().with_experience(|s| s.has_episode("deep")).unwrap(),
            "depth {depth} acknowledged but lost on reopen"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    let over = m.call(
        "remember",
        json!({"episode": "deep", "step": 0, "url": "http://x/d", "tree": chain(limit + 1)}),
    );
    assert!(over.is_err(), "depth {} must be refused", limit + 1);
}

#[test]
fn action_effects_are_recalled_explained_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    remember(
        &m,
        step("trip", 0, "Checkout", &["Place order", "Cancel"], None),
    );
    remember(
        &m,
        step(
            "trip",
            1,
            "Checkout",
            &["Place order", "Cancel", "Confirmation banner shown"],
            Some(json!({"label": "click Place order"})),
        ),
    );
    let recall = |m: &Memory| {
        m.call(
            "recall",
            json!({"query": "what happens after click Place order confirmation banner",
                   "k": 10, "explain": true}),
        )
        .unwrap()
    };
    let find = |v: &Value| {
        v["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                r["id"]
                    .as_str()
                    .is_some_and(|i| i.starts_with("experience:x"))
            })
            .cloned()
    };
    let hit = find(&recall(&m)).expect("an effect row is recalled");
    assert!(hit["text"]
        .as_str()
        .unwrap()
        .contains("Confirmation banner shown"));
    let id = hit["id"].as_str().unwrap().to_string();
    let why = m
        .call(
            "explain",
            json!({"query": "click Place order banner", "id": id}),
        )
        .unwrap();
    assert_eq!(why["explanation"]["provenance"][0]["episode"], "trip");
    assert_eq!(why["explanation"]["provenance"][0]["step"], 1);
    m.core().flush().unwrap();
    drop(m);
    let m = open(dir.path());
    assert!(find(&recall(&m)).is_some(), "effect index rebuilt on open");
    m.call(
        "forget",
        json!({"scope": {"kind": "episode", "episode": "trip"}}),
    )
    .unwrap();
    assert!(
        find(&recall(&m)).is_none(),
        "the row is gone with its last episode"
    );
}

#[test]
fn a_failing_observation_leaves_the_facts_of_its_request_unstored() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    remember(&m, step("ep", 0, "Checkout", &["Place order"], None));
    let clash = m.call(
        "remember",
        json!({"facts": [
            {"id": "f1", "text": "a fact sent with a clashing step"},
            step("ep", 0, "Different page", &["Other"], None),
        ]}),
    );
    assert!(clash.is_err());
    assert!(
        m.core().get_document("f1").unwrap().is_none(),
        "the fact was stored although the request failed"
    );
    // The same request with a valid step stores both.
    remember(
        &m,
        json!({"facts": [
            {"id": "f1", "text": "a fact sent with a good step"},
            step("ep", 1, "Checkout", &["Place order", "Done"], None),
        ]}),
    );
    assert!(m.core().get_document("f1").unwrap().is_some());
}

#[test]
fn a_fact_with_an_experience_shaped_id_explains_as_a_fact_and_unknown_episodes_forget_as_absent() {
    let dir = tempfile::tempdir().unwrap();
    let m = open(dir.path());
    remember(
        &m,
        json!({"id": "experience:r1", "text": "apple pie recipe"}),
    );
    let e = m
        .explain("apple pie", "experience:r1", &RecallOptions::default())
        .unwrap();
    assert!(e.retrieved);
    let report = m
        .call(
            "forget",
            json!({"scope": {"kind": "episode", "episode": "nope"}}),
        )
        .unwrap();
    assert_eq!(report["forgotten"], json!([]));
    assert_eq!(report["purged"], json!(false));
    assert!(experience_files(dir.path()).is_empty());
}
