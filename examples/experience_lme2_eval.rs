// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Gold-phrase coverage at equal token budgets on LongMemEval-V2 dev trajectories, through the
//! memory API: observation steps stored with `remember` (experience core, arm R), the same
//! trajectories as plain text chunks stored with `remember` (document path, arm D), and both in
//! one store (arm M, the merged ranking). A question counts when every gold phrase occurs in the
//! packed text (the prototype's M1 metric; tokens are chars / 3.5). No model or API calls.
//!
//! Usage: experience_lme2_eval DATA_DIR OUT.json
//! DATA_DIR holds traj_{web,enterprise}.jsonl, b0_{web,enterprise}.jsonl and dev_phrase.jsonl
//! written by export_lme2.py (dev split only).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{json, Value};
use writerslogic_warm::core::experience::tree::parse_indented;
use writerslogic_warm::core::experience::{Action, OwnedObservation};
use writerslogic_warm::core::memory_api::{ManualClock, Memory};
use writerslogic_warm::WarmCore;

#[derive(Deserialize)]
struct State {
    state_index: u32,
    url: String,
    action: Option<String>,
    accessibility_tree: String,
}

#[derive(Deserialize)]
struct Traj {
    id: String,
    goal: String,
    outcome: String,
    states: Vec<State>,
}

#[derive(Deserialize)]
struct Chunk {
    id: String,
    text: String,
}

#[derive(Deserialize)]
struct Question {
    domain: String,
    #[serde(rename = "type")]
    kind: String,
    question: String,
    phrases: Vec<String>,
}

fn norm(s: &str) -> String {
    let s: String = s
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                ' '
            }
        })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lines<T: for<'a> Deserialize<'a>>(path: &str) -> Vec<T> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .map(|l| serde_json::from_str(l).expect("jsonl"))
        .collect()
}

fn open(dir: &std::path::Path) -> Memory {
    let core = Arc::new(WarmCore::new(4096, Some(dir.display().to_string()), None).unwrap());
    Memory::new(core).with_clock(Arc::new(ManualClock::new(1)))
}

/// The observation steps of each trajectory, parsed from the accessibility-tree text.
fn observations(trajs: &[Traj]) -> (Vec<Vec<OwnedObservation>>, usize) {
    let mut raw = 0;
    let all = trajs
        .iter()
        .map(|t| {
            let mut batch = Vec::new();
            for s in &t.states {
                let Some(tree) = parse_indented(&s.accessibility_tree) else {
                    continue;
                };
                raw += s.accessibility_tree.len();
                let mut o = OwnedObservation {
                    episode: t.id.clone(),
                    step: s.state_index,
                    tree,
                    url: Some(s.url.clone()),
                    action: s.action.clone().map(Action::Call),
                    goal: None,
                    outcome: None,
                };
                if batch.is_empty() {
                    o.goal = Some(t.goal.chars().take(1000).collect());
                    o.outcome = Some(t.outcome.clone());
                }
                batch.push(o);
            }
            batch
        })
        .collect();
    (all, raw)
}

/// Through the JSON surface (`Memory::call`), one call per trajectory.
fn ingest_json(m: &Memory, all: &[Vec<OwnedObservation>]) {
    for batch in all {
        let facts: Vec<Value> = batch
            .iter()
            .map(|o| serde_json::to_value(o).unwrap())
            .collect();
        m.call("remember", json!({ "facts": facts }))
            .expect("observations");
    }
}

/// Through the typed API, one call per trajectory.
fn ingest_typed(m: &Memory, all: &[Vec<OwnedObservation>]) {
    for batch in all {
        m.remember_observations(batch.clone())
            .expect("observations");
    }
}

fn ingest_chunks(m: &Memory, chunks: &[Chunk]) {
    for batch in chunks.chunks(256) {
        let facts: Vec<Value> = batch
            .iter()
            .map(|c| json!({"id": c.id, "text": c.text}))
            .collect();
        m.call("remember", json!({ "facts": facts }))
            .expect("chunks");
    }
}

/// The prototype's B0 packing: whole chunks in rank order until one does not fit.
fn pack_whole(texts: &[String], budget_chars: usize) -> String {
    let (mut out, mut size) = (Vec::new(), 0);
    for t in texts {
        if size + t.len() > budget_chars {
            break;
        }
        out.push(t.as_str());
        size += t.len();
    }
    out.join("\n")
}

/// The prototype's unit packing (frac 1): items in rank order, the last one cut to the cap.
fn pack_cut(texts: &[String], cap: usize) -> String {
    let (mut out, mut size) = (Vec::new(), 0usize);
    for t in texts {
        let mut t = t.as_str();
        if size + t.len() > cap {
            if (size as f64) < cap as f64 * 0.9 {
                let mut end = cap - size;
                while !t.is_char_boundary(end) {
                    end -= 1;
                }
                t = &t[..end];
            } else {
                t = "";
            }
            if t.is_empty() {
                continue;
            }
        }
        out.push(t);
        size += t.len() + 1;
        if size >= cap {
            break;
        }
    }
    out.join("\n")
}

fn recall_texts(m: &Memory, query: &str, k: u32, lat: &mut Vec<f64>) -> Vec<(String, String)> {
    let started = Instant::now();
    let out = m
        .call("recall", json!({"query": query, "k": k}))
        .expect("recall");
    lat.push(started.elapsed().as_secs_f64() * 1000.0);
    out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["id"].as_str().unwrap().to_string(),
                r["text"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

/// Regions only (no effect rows), straight from the experience core.
fn region_texts(m: &Memory, query: &str, k: usize, lat: &mut Vec<f64>) -> Vec<(String, String)> {
    let started = Instant::now();
    let hits = m
        .core()
        .with_experience(|s| s.search_regions(query, k).unwrap())
        .unwrap();
    lat.push(started.elapsed().as_secs_f64() * 1000.0);
    hits.into_iter()
        .map(|h| (format!("r{}", h.region), h.text))
        .collect()
}

/// Effect rows only, straight from the experience core.
fn effect_texts(m: &Memory, query: &str, k: usize, lat: &mut Vec<f64>) -> Vec<(String, String)> {
    let started = Instant::now();
    let hits = m
        .core()
        .with_experience(|s| s.search_effects(query, k).unwrap())
        .unwrap();
    lat.push(started.elapsed().as_secs_f64() * 1000.0);
    hits.into_iter().map(|h| (h.id, h.view.render())).collect()
}

/// The document store searched directly, bypassing the memory layer.
fn direct_texts(m: &Memory, query: &str, k: u32, lat: &mut Vec<f64>) -> Vec<(String, String)> {
    let started = Instant::now();
    let opts = writerslogic_warm::core::documents::SearchOptions {
        k: Some(k),
        ..Default::default()
    };
    let hits = m.core().search_documents(query, &opts).unwrap();
    lat.push(started.elapsed().as_secs_f64() * 1000.0);
    hits.into_iter()
        .map(|h| (h.document_id, h.text.unwrap_or_default()))
        .collect()
}

#[derive(Default)]
struct Tally(BTreeMap<String, (u32, u32)>);

impl Tally {
    fn add(&mut self, kind: &str, ok: bool) {
        for key in ["ALL", kind] {
            let e = self.0.entry(key.to_string()).or_default();
            e.0 += u32::from(ok);
            e.1 += 1;
        }
    }
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let dir = std::env::args().nth(1).expect("DATA_DIR");
    let out = std::env::args().nth(2).expect("OUT.json");
    let questions: Vec<Question> = lines(&format!("{dir}/dev_phrase.jsonl"));
    let budgets = [4000usize, 8000, 16000];
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut latency: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut ingest = Vec::new();
    for domain in ["web", "enterprise"] {
        let trajs: Vec<Traj> = {
            let path = format!("{dir}/traj_{domain}.jsonl");
            lines(&path)
        };
        let chunks: Vec<Chunk> = lines(&format!("{dir}/b0_{domain}.jsonl"));
        let (tr, td, tm) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let (r, d, mixed) = (open(tr.path()), open(td.path()), open(tm.path()));
        let started = Instant::now();
        let (all, raw) = observations(&trajs);
        let parse_secs = started.elapsed().as_secs_f64();
        let steps: usize = all.iter().map(Vec::len).sum();
        let started = Instant::now();
        let mut bare = writerslogic_warm::core::experience::Experience::default();
        for o in all.iter().flatten() {
            bare.ingest(&o.as_observation()).unwrap();
        }
        let bare_secs = started.elapsed().as_secs_f64();
        let tt = tempfile::tempdir().unwrap();
        let typed = open(tt.path());
        let started = Instant::now();
        ingest_typed(&typed, &all);
        let typed_secs = started.elapsed().as_secs_f64();
        let started = Instant::now();
        ingest_json(&r, &all);
        let r_secs = started.elapsed().as_secs_f64();
        let started = Instant::now();
        ingest_chunks(&d, &chunks);
        let d_secs = started.elapsed().as_secs_f64();
        ingest_json(&mixed, &all);
        ingest_chunks(&mixed, &chunks);
        for m in [&r, &d, &mixed] {
            m.core().flush().unwrap();
        }
        let bytes = |p: &std::path::Path, prefix: &str| -> u64 {
            std::fs::read_dir(p)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                .map(|e| e.metadata().unwrap().len())
                .sum()
        };
        let stats = r.core().with_experience(|s| s.stats()).unwrap();
        ingest.push(json!({
            "domain": domain, "trajectories": trajs.len(), "steps": steps, "raw_tree_bytes": raw,
            "parse_indented_secs": parse_secs,
            "experience_bare_ingest_secs": bare_secs, "experience_bare_steps_per_sec": steps as f64 / bare_secs,
            "memory_typed_ingest_secs": typed_secs, "memory_typed_steps_per_sec": steps as f64 / typed_secs,
            "experience_ingest_secs": r_secs, "experience_steps_per_sec": steps as f64 / r_secs,
            "experience_snapshot_bytes": bytes(tr.path(), "experience.wexp"),
            "experience_journal_bytes_after_flush": bytes(tr.path(), "experience.wexj"),
            "experience_stats": format!("{stats:?}"),
            "chunks": chunks.len(), "chunk_bytes": chunks.iter().map(|c| c.text.len()).sum::<usize>(),
            "document_ingest_secs": d_secs, "document_chunks_per_sec": chunks.len() as f64 / d_secs,
        }));
        for q in questions.iter().filter(|q| q.domain == domain) {
            for (arm, m, k, whole) in [
                ("R_experience", &r, 200u32, false),
                ("R_regions_only", &r, 200, false),
                ("R_effects_only", &r, 200, false),
                ("D_documents", &d, 300, true),
                ("D_direct", &d, 300, true),
                ("M_merged", &mixed, 300, false),
            ] {
                let lat = latency.entry(arm.to_string()).or_default();
                let hits = match arm {
                    "R_regions_only" => region_texts(m, &q.question, k as usize, lat),
                    "R_effects_only" => effect_texts(m, &q.question, k as usize, lat),
                    "D_direct" => direct_texts(m, &q.question, k, lat),
                    _ => recall_texts(m, &q.question, k, lat),
                };
                if std::env::var_os("LME2_DEBUG").is_some() {
                    eprintln!(
                        "{arm} {} hits={} lens={:?} top={:?}",
                        q.question.chars().take(50).collect::<String>(),
                        hits.len(),
                        hits.iter().take(5).map(|h| h.1.len()).collect::<Vec<_>>(),
                        hits.iter().take(3).map(|h| h.0.clone()).collect::<Vec<_>>()
                    );
                }
                let texts: Vec<String> = hits.into_iter().map(|h| h.1).collect();
                for bt in budgets {
                    let cap = (bt as f64 * 3.5) as usize;
                    let packed = if whole {
                        pack_whole(&texts, cap)
                    } else {
                        pack_cut(&texts, cap)
                    };
                    let n = norm(&packed);
                    let ok = q.phrases.iter().all(|p| n.contains(p.as_str()));
                    tallies
                        .entry(format!("{arm}@{bt}"))
                        .or_default()
                        .add(&q.kind, ok);
                }
            }
        }
    }
    let result: BTreeMap<_, _> = tallies
        .iter()
        .map(|(k, t)| (k.clone(), t.0.clone()))
        .collect();
    let lat: BTreeMap<_, _> = latency
        .iter_mut()
        .map(|(k, v)| {
            (
                k.clone(),
                json!({"p50_ms": percentile(v, 0.5), "p95_ms": percentile(v, 0.95), "n": v.len()}),
            )
        })
        .collect();
    let report = json!({"coverage": result, "recall_latency": lat, "ingest": ingest});
    std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    for (k, t) in &tallies {
        let all = t.0["ALL"];
        println!(
            "{k:18} ALL {:.3} ({}/{})",
            f64::from(all.0) / f64::from(all.1),
            all.0,
            all.1
        );
    }
}
