// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The memory API through the real `warm-server` process: HTTP contract, error classes,
//! limits, tenant isolation, configuration file and persistence across a restart.

#![cfg(feature = "server")]

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

mod server_support;
use server_support::{Server, DIM};

fn start(dir: &Path, extra: &[&str]) -> Server {
    let mut args = vec!["--no-auth"];
    args.extend_from_slice(extra);
    Server::start_in(dir, &args, &[])
}

trait MemoryPost {
    fn post(&self, op: &str, tenant: &str, body: &Value) -> (u16, Value);
}

impl MemoryPost for Server {
    fn post(&self, op: &str, tenant: &str, body: &Value) -> (u16, Value) {
        let response = self
            .req("POST", &format!("/api/v1/memory/{op}"))
            .tenant(tenant)
            .json(body)
            .send();
        (response.status, response.json())
    }
}

fn unit(axis: usize) -> Vec<f64> {
    (0..DIM)
        .map(|i| if i == axis { 1.0 } else { 0.0 })
        .collect()
}

fn ids(response: &Value) -> Vec<&str> {
    response["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect()
}

#[test]
fn frozen_native_lifecycle_has_the_same_http_contract() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native_memory_lifecycle_v1.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (phase, steps) in fixture["phases"].as_array().unwrap().iter().enumerate() {
        let server = start(dir.path(), &[]);
        for (step, case) in steps.as_array().unwrap().iter().enumerate() {
            let (status, out) =
                server.post(case["op"].as_str().unwrap(), "lifecycle", &case["request"]);
            if let Some(code) = case["error"].as_str() {
                assert_eq!(status, 409);
                assert_eq!(out["code"], code);
            } else {
                assert_eq!(status, 200, "phase {phase} step {step}: {out}");
                for (pointer, expected) in case["expect"].as_object().unwrap() {
                    assert_eq!(
                        out.pointer(pointer),
                        Some(expected),
                        "phase {phase} step {step}: {pointer}: {out}"
                    );
                }
            }
        }
    }
}

#[test]
fn shared_memory_negotiation_and_validation_http_contract() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/memory_contract_cases_v1.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let server = start(dir.path(), &[]);
    for case in fixture["cases"].as_array().unwrap() {
        let (status, result) =
            server.post(case["op"].as_str().unwrap(), "contract", &case["request"]);
        if case["error"] == "invalid_request" {
            assert_eq!(status, 422, "{case}: {result}");
            assert_eq!(result["code"], "invalid_request");
        } else {
            assert_eq!(status, 200, "{case}: {result}");
            for (pointer, expected) in case["expect"].as_object().unwrap() {
                assert_eq!(result.pointer(pointer), Some(expected));
            }
        }
    }
}

#[test]
fn memory_round_trip_over_http_with_history_errors_and_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(&dir.path().join("data"), &[]);

    let (status, out) = server.post(
        "remember",
        "a",
        &json!({"id": "c1", "text": "Alice lives in Paris", "subject": "alice/city",
                "validFrom": 100, "embedding": unit(0)}),
    );
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["remembered"][0]["created"], json!(true));
    let (status, out) = server.post(
        "remember",
        "a",
        &json!({"facts": [{"id": "c2", "text": "Alice lives in Berlin", "validFrom": 200,
                           "supersedes": ["c1"], "embedding": unit(1)}]}),
    );
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["remembered"][0]["supersedes"], json!(["c1"]));

    // Current only by default; history on request; as-of in the past; vectors rank.
    let (_, current) = server.post(
        "recall",
        "a",
        &json!({"query": "Alice lives", "asOf": 1000}),
    );
    assert_eq!(ids(&current), ["c2"]);
    let (_, all) = server.post(
        "recall",
        "a",
        &json!({"query": "Alice lives", "history": "all", "asOf": 1000}),
    );
    assert_eq!(all["results"].as_array().unwrap().len(), 2);
    let (_, early) = server.post("recall", "a", &json!({"query": "Alice lives", "asOf": 150}));
    assert_eq!(ids(&early), ["c1"]);
    let (_, semantic) = server.post(
        "recall",
        "a",
        &json!({"query": "", "embedding": unit(1), "history": "all", "asOf": 1000,
                "lexicalWeight": 0.0, "minSemanticScore": 0.5}),
    );
    assert_eq!(ids(&semantic), ["c2"]);

    // Explain, get and history.
    let (status, explained) = server.post(
        "explain",
        "a",
        &json!({"query": "Alice Berlin", "id": "c2", "asOf": 1000}),
    );
    assert_eq!(status, 200, "{explained}");
    assert_eq!(
        explained["explanation"]["matchedTerms"],
        json!(["alice", "berlin"])
    );
    assert_eq!(explained["explanation"]["chain"][0]["id"], json!("c1"));
    let (_, got) = server.post("get", "a", &json!({"id": "c1"}));
    assert_eq!(got["fact"]["supersededBy"], json!(["c2"]));
    let (_, history) = server.post("history", "a", &json!({"id": "c2"}));
    assert_eq!(history["history"].as_array().unwrap().len(), 2);

    // Tenants are isolated.
    let (_, other) = server.post("recall", "b", &json!({"query": "Alice", "history": "all"}));
    assert!(ids(&other).is_empty());

    // Error classes.
    assert_eq!(server.post("nope", "a", &json!({})).0, 404);
    assert_eq!(
        server.post("history", "a", &json!({"id": "missing"})).0,
        404
    );
    let (status, conflict) =
        server.post("remember", "a", &json!({"id": "c1", "text": "different"}));
    assert_eq!(
        (status, conflict["error"]["code"].as_str()),
        (409, Some("conflict"))
    );
    for bad in [
        json!({"id": "x", "text": "t", "unknownField": 1}),
        json!({"id": "x", "text": "t", "embedding": [1.0, 2.0]}),
        json!({"id": "hms:x", "text": "t"}),
        json!({"id": "x", "text": "t", "supersedes": ["missing"]}),
        json!({"facts": []}),
    ] {
        let (status, body) = server.post("remember", "a", &bad);
        assert_eq!(status, 422, "{bad} -> {body}");
        assert_eq!(body["error"]["code"], json!("invalid_request"));
    }
    assert_eq!(
        server.post("recall", "a", &json!({"query": "q", "k": 0})).0,
        422
    );
    assert_eq!(server.post("recall", "a", &json!({"k": 3})).0, 422);

    // Retrying is idempotent.
    let (_, again) = server.post(
        "remember",
        "a",
        &json!({"id": "c1", "text": "Alice lives in Paris", "subject": "alice/city",
                "validFrom": 100, "embedding": unit(0)}),
    );
    assert_eq!(again["remembered"][0]["created"], json!(false));

    // Forget: dry run, then for real, then repeated.
    let scope = json!({"kind": "subject", "subject": "alice/city"});
    let (_, dry) = server.post("forget", "a", &json!({"scope": scope, "dryRun": true}));
    assert_eq!(dry["forgotten"], json!(["c1"]));
    let (_, done) = server.post("forget", "a", &json!({"scope": scope}));
    assert_eq!(done["forgotten"], json!(["c1"]));
    let (_, repeat) = server.post("forget", "a", &json!({"scope": scope}));
    assert_eq!(repeat["forgotten"], json!([]));
    let (_, now) = server.post("recall", "a", &json!({"query": "Alice", "history": "all"}));
    assert_eq!(ids(&now), ["c2"]);
}

#[test]
fn memory_state_and_configuration_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("memory.json");
    std::fs::write(
        &config,
        json!({"supersession": "latest-by-subject", "defaultHistory": "current", "defaultK": 3})
            .to_string(),
    )
    .unwrap();
    let data = dir.path().join("data");
    let args = ["--memory-config", config.to_str().unwrap()];
    let mut server = start(&data, &args);
    for (id, from) in [("v1", 100), ("v2", 200), ("v3", 300)] {
        let (status, out) = server.post(
            "remember",
            "t",
            &json!({"id": id, "text": format!("setting value {id}"), "subject": "setting",
                    "validFrom": from}),
        );
        assert_eq!(status, 200, "{out}");
    }
    assert!(server.stop(), "server must exit cleanly on SIGTERM");

    let server = start(&data, &args);
    let (_, current) = server.post(
        "recall",
        "t",
        &json!({"query": "setting value", "asOf": 1000}),
    );
    assert_eq!(ids(&current), ["v3"]);
    let (_, early) = server.post(
        "recall",
        "t",
        &json!({"query": "setting value", "asOf": 250}),
    );
    assert_eq!(ids(&early), ["v2"]);
    let (_, history) = server.post("history", "t", &json!({"id": "v1"}));
    assert_eq!(history["history"].as_array().unwrap().len(), 3);
}

#[test]
fn invalid_memory_config_stops_the_server_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("memory.json");
    std::fs::write(&config, r#"{"defaultK": 0}"#).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_warm-server"))
        .args(["--bind", "127.0.0.1:0", "--data-dir"])
        .arg(dir.path().join("data"))
        .args(["--memory-config", config.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
}

#[test]
fn observations_over_http_are_routed_bounded_recalled_and_forgotten() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(dir.path(), &[]);
    let tree = json!({"role": "RootWebArea", "name": "Checkout", "children": [
        {"role": "button", "name": "Place order"}, {"role": "button", "name": "Cancel"}]});
    let (status, body) = server.post(
        "remember",
        "t1",
        &json!({"episode": "e1", "step": 0, "url": "http://shop/c", "tree": tree}),
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = server.post("recall", "t1", &json!({"query": "Place order button"}));
    assert_eq!(status, 200, "{body}");
    assert!(
        ids(&body).iter().any(|i| i.starts_with("experience:r")),
        "{body}"
    );

    // Nesting is refused before any tree is built: the JSON parser stops at 128 levels, which is
    // 63 tree levels, so the experience depth limit of 128 is not reachable over HTTP.
    let mut deep = json!({"role": "button", "name": "leaf"});
    for _ in 0..600 {
        deep = json!({"role": "group", "name": "g", "children": [deep]});
    }
    let (status, body) = server.post(
        "remember",
        "t1",
        &json!({"episode": "e2", "step": 0, "tree": deep}),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "nesting_too_deep", "{body}");
    // 63 levels is accepted: the parser's limit is the only bound below the library's 128.
    let mut ok = json!({"role": "button", "name": "leaf"});
    for _ in 0..61 {
        ok = json!({"role": "group", "name": "g", "children": [ok]});
    }
    let (status, body) = server.post(
        "remember",
        "t1",
        &json!({"episode": "e2", "step": 0, "tree": ok}),
    );
    assert_eq!(status, 200, "{body}");
    server.post(
        "forget",
        "t1",
        &json!({"scope": {"kind": "episode", "episode": "e2"}}),
    );
    // Byte limits are checked on the raw value, before allocation.
    let (status, _) = server.post(
        "remember",
        "t1",
        &json!({"episode": "e3", "step": 0,
                "tree": {"role": "button", "name": "x".repeat(16 * 1024 + 1)}}),
    );
    assert_eq!(status, 422);
    let (status, _) = server.post(
        "remember",
        "t1",
        &json!({"episode": "e4", "step": 0, "tree": {"role": "", "name": "x"}}),
    );
    assert_eq!(status, 422);
    let (status, _) = server.post(
        "remember",
        "t1",
        &json!({"facts": [
            {"id": "f", "text": "a fact in the same batch"},
            {"episode": "e5", "step": 0, "tree": {"role": "", "name": "x"}}]}),
    );
    assert_eq!(status, 422);
    let (_, body) = server.post(
        "recall",
        "t1",
        &json!({"query": "a fact in the same batch"}),
    );
    assert!(
        ids(&body).is_empty(),
        "a refused batch stores nothing: {body}"
    );

    let (status, body) = server.post(
        "forget",
        "t1",
        &json!({"scope": {"kind": "episode", "episode": "e1"}}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["forgotten"][0], "experience:episode:e1");
    let (_, body) = server.post("recall", "t1", &json!({"query": "Place order button"}));
    assert!(
        !ids(&body).iter().any(|i| i.starts_with("experience:")),
        "{body}"
    );
}

#[test]
fn observations_count_against_the_tenant_quota() {
    // The first observation stores one episode and one region (2 units); each further episode of
    // the same shape shares the region and adds one.
    let obs = |ep: &str| {
        json!({"episode": ep, "step": 0, "url": "http://q/p",
            "tree": {"role": "RootWebArea", "name": "Quota page", "children": [
                {"role": "button", "name": "press the shared button"}]}})
    };
    let units_after = |quota: &str, n: usize| -> Vec<u16> {
        let dir = tempfile::tempdir().unwrap();
        let server = start(dir.path(), &["--max-documents-per-tenant", quota]);
        (0..n)
            .map(|i| server.post("remember", "q", &obs(&format!("e{i}"))).0)
            .collect()
    };
    // Each new episode adds an episode unit and shares the region, so units are 1 + 1 for the
    // first, then +1 per further episode (same structure): 2, 3, 4.
    assert_eq!(
        units_after("3", 3),
        vec![200, 200, 429],
        "quota is reached exactly"
    );
    assert_eq!(
        units_after("4", 3),
        vec![200, 200, 200],
        "quota-1..quota admits"
    );
    assert_eq!(units_after("2", 2), vec![200, 429], "quota+1 refuses");
    let dir = tempfile::tempdir().unwrap();
    let server = start(dir.path(), &["--max-documents-per-tenant", "3"]);
    assert_eq!(server.post("remember", "q", &obs("a")).0, 200);
    assert_eq!(server.post("remember", "q", &obs("b")).0, 200);
    let (status, body) = server.post("remember", "q", &obs("c"));
    assert_eq!(status, 429, "{body}");
    assert_eq!(body["error"]["code"], "quota_exceeded");
    // The refused step left nothing: a fact still cannot fit, and the episode is not stored.
    let (status, _) = server.post("remember", "q", &json!({"id": "f", "text": "a fact"}));
    assert_eq!(status, 429);
}
