// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Supersession probe on the public STALE data (arXiv 2605.06527, CC BY 4.0).
//!
//! Protocol: `docs/research/supersession-probe.md` (pre-registered before this ran). Input is
//! the JSONL written by `benchmarks/supersession_probe/prep.py`. This is a retrieval-level proxy
//! and is not comparable to STALE's LLM-judged accuracy.
//!
//! ```text
//! cargo run --release --features local-embed --example supersession_probe -- \
//!   --probe probe.jsonl --pack bge.hmsemb --out result.json [--limit N] [--scratch DIR]
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use writerslogic_warm::core::embed::{EmbedOptions, Embedder};
use writerslogic_warm::core::memory_api::{
    FactStatus, HistoryMode, Memory, RecallOptions, RememberInput,
};
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{EmbeddingSpace, WarmCore};

const SEED: u64 = 20_261_103;
/// 2026-01-01T00:00:00Z, after every session timestamp (checked at load).
const AS_OF: i64 = 1_767_225_600_000;
const RANK_K: u32 = 50;
const CANDIDATES: u32 = 100;
const SHOW: usize = 5;
const MAX_LINKS: usize = 256;
const TAUS: [f32; 9] = [0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95, 1.01];
const DIMS: [&str; 3] = ["dim1", "dim2", "dim3"];
const ARMS: [&str; 5] = ["a", "b", "c", "c_filter", "d"];

#[derive(Deserialize)]
struct Turn {
    id: String,
    session: u32,
    ts: i64,
    text: String,
}

#[derive(Deserialize)]
struct Queries {
    dim1: String,
    dim2: String,
    dim3: String,
}

#[derive(Deserialize)]
struct Inst {
    uid: String,
    #[serde(rename = "type")]
    kind: String,
    split: String,
    old_id: String,
    new_id: String,
    old_session: u32,
    new_session: u32,
    queries: Queries,
    turns: Vec<Turn>,
}

struct Prepared {
    inst: Inst,
    /// Passage embedding per turn (row into the shared matrix).
    rows: Vec<usize>,
    /// Query embeddings for dim1..dim3.
    qrows: [usize; 3],
}

/// Per-query outcome flags.
#[derive(Clone, Copy, Default)]
struct M {
    cv1: bool,
    new5: bool,
    stale1: bool,
    stale5: bool,
    sess1: bool,
    oldsib1: bool,
}

fn measure(inst: &Inst, list: &[String], session_of: &HashMap<&str, u32>) -> M {
    let top = &list[..list.len().min(SHOW)];
    let first = top.first().map(String::as_str);
    M {
        cv1: first == Some(inst.new_id.as_str()),
        new5: top.iter().any(|id| *id == inst.new_id),
        stale1: first == Some(inst.old_id.as_str()),
        stale5: top.iter().any(|id| *id == inst.old_id),
        sess1: first.and_then(|id| session_of.get(id)) == Some(&inst.new_session),
        oldsib1: first.and_then(|id| session_of.get(id)) == Some(&inst.old_session),
    }
}

/// The supersession graph as a reader sees it.
trait Graph {
    fn successors(&self, id: &str) -> Vec<String>;
    fn is_current(&self, id: &str) -> bool;
    fn ts(&self, id: &str) -> i64;
}

/// Replace every superseded hit by the newest current fact reachable through successors (newest
/// timestamp, ties to the smallest id), drop duplicates, keep the first `SHOW`.
fn follow(
    hits: &[String],
    graph: &dyn Graph,
    memo: &RefCell<HashMap<String, Option<String>>>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for hit in hits {
        let resolved = {
            let cached = memo.borrow().get(hit).cloned();
            match cached {
                Some(r) => r,
                None => {
                    let r = resolve(hit, graph);
                    memo.borrow_mut().insert(hit.clone(), r.clone());
                    r
                }
            }
        };
        if let Some(id) = resolved {
            if !out.contains(&id) {
                out.push(id);
                if out.len() == SHOW {
                    break;
                }
            }
        }
    }
    out
}

fn resolve(id: &str, graph: &dyn Graph) -> Option<String> {
    if graph.is_current(id) {
        return Some(id.to_string());
    }
    let mut seen: Vec<String> = vec![id.to_string()];
    let mut stack = vec![id.to_string()];
    let mut best: Option<(i64, String)> = None;
    while let Some(node) = stack.pop() {
        for next in graph.successors(&node) {
            if seen.contains(&next) {
                continue;
            }
            seen.push(next.clone());
            if graph.is_current(&next) {
                let key = (graph.ts(&next), next.clone());
                best = Some(match best {
                    Some(b) if b.0 > key.0 || (b.0 == key.0 && b.1 <= key.1) => b,
                    _ => key,
                });
            } else {
                stack.push(next);
            }
        }
    }
    best.map(|b| b.1)
}

/// Links simulated from a `supersedes` table (turn index -> earlier turn indices).
struct SimGraph<'a> {
    inst: &'a Inst,
    succ: HashMap<&'a str, Vec<String>>,
}

impl<'a> SimGraph<'a> {
    fn new(inst: &'a Inst, supersedes: &[Vec<usize>]) -> Self {
        let mut succ: HashMap<&str, Vec<String>> = HashMap::new();
        for (x, olds) in supersedes.iter().enumerate() {
            for &y in olds {
                succ.entry(inst.turns[y].id.as_str())
                    .or_default()
                    .push(inst.turns[x].id.clone());
            }
        }
        Self { inst, succ }
    }
}

impl Graph for SimGraph<'_> {
    fn successors(&self, id: &str) -> Vec<String> {
        self.succ.get(id).cloned().unwrap_or_default()
    }
    fn is_current(&self, id: &str) -> bool {
        !self.succ.contains_key(id)
    }
    fn ts(&self, id: &str) -> i64 {
        self.inst
            .turns
            .iter()
            .find(|t| t.id == id)
            .map_or(0, |t| t.ts)
    }
}

/// Links read back from a real store.
struct WarmGraph<'a> {
    memory: &'a Memory,
    cache: RefCell<HashMap<String, (Vec<String>, bool, i64)>>,
}

impl WarmGraph<'_> {
    fn load(&self, id: &str) -> (Vec<String>, bool, i64) {
        if let Some(v) = self.cache.borrow().get(id) {
            return v.clone();
        }
        let rec = self.memory.get(id).ok().flatten();
        let v = rec.map_or((Vec::new(), false, 0), |r| {
            (
                r.superseded_by,
                r.status == FactStatus::Current,
                r.effective_from,
            )
        });
        self.cache.borrow_mut().insert(id.to_string(), v.clone());
        v
    }
}

impl Graph for WarmGraph<'_> {
    fn successors(&self, id: &str) -> Vec<String> {
        self.load(id).0
    }
    fn is_current(&self, id: &str) -> bool {
        self.load(id).1
    }
    fn ts(&self, id: &str) -> i64 {
        self.load(id).2
    }
}

fn f64s(v: &[f32]) -> Vec<f64> {
    v.iter().map(|&x| f64::from(x)).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

struct Ctx<'a> {
    matrix: &'a [Vec<f32>],
    space: EmbeddingSpace,
    scratch: &'a Path,
}

/// Build a store holding every turn of the instance, in chronological order, in one batch.
fn build(
    p: &Prepared,
    ctx: &Ctx,
    supersedes: &[Vec<usize>],
) -> Result<(Memory, tempfile::TempDir)> {
    let dir = tempfile::tempdir_in(ctx.scratch)?;
    let config = WarmConfig {
        embedding_space: Some(ctx.space.clone()),
        ..Default::default()
    };
    let core = WarmCore::new(4096, Some(dir.path().display().to_string()), Some(config))?;
    let memory = Memory::new(Arc::new(core));
    let inputs: Vec<RememberInput> = p
        .inst
        .turns
        .iter()
        .enumerate()
        .map(|(i, t)| RememberInput {
            id: Some(t.id.clone()),
            text: t.text.clone(),
            event_time: Some(t.ts),
            valid_from: Some(t.ts),
            supersedes: (!supersedes[i].is_empty()).then(|| {
                supersedes[i]
                    .iter()
                    .map(|&y| p.inst.turns[y].id.clone())
                    .collect()
            }),
            embedding: Some(f64s(&ctx.matrix[p.rows[i]])),
            ..Default::default()
        })
        .collect();
    memory.remember_many(inputs)?;
    Ok((memory, dir))
}

fn qtext(inst: &Inst, dim: usize) -> &str {
    [&inst.queries.dim1, &inst.queries.dim2, &inst.queries.dim3][dim]
}

fn recall_ids(
    memory: &Memory,
    text: &str,
    q: &[f32],
    k: u32,
    history: HistoryMode,
) -> Result<Vec<String>> {
    let options = RecallOptions {
        k: Some(k),
        as_of: Some(AS_OF),
        history: Some(history),
        embedding: Some(f64s(q)),
        candidate_limit: Some(CANDIDATES),
        ..Default::default()
    };
    Ok(memory
        .recall(text, &options)?
        .into_iter()
        .map(|r| r.id)
        .collect())
}

/// Per-instance results: arm -> [M; 3], plus the simulated d for every tau.
struct InstResult {
    arms: HashMap<&'static str, [M; 3]>,
    d_sim: Vec<[M; 3]>,
    d_sim_mismatch_vs_real: Option<usize>,
}

fn candidate_links(p: &Prepared, ctx: &Ctx) -> Vec<Vec<(usize, f32)>> {
    let turns = &p.inst.turns;
    (0..turns.len())
        .map(|x| {
            let mut v: Vec<(usize, f32)> = (0..x)
                .filter(|&y| turns[y].ts < turns[x].ts)
                .map(|y| (y, dot(&ctx.matrix[p.rows[x]], &ctx.matrix[p.rows[y]])))
                .filter(|&(_, c)| c >= TAUS[0])
                .collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            v.truncate(MAX_LINKS);
            v
        })
        .collect()
}

fn links_at(cand: &[Vec<(usize, f32)>], tau: f32) -> Vec<Vec<usize>> {
    cand.iter()
        .map(|v| {
            v.iter()
                .filter(|&&(_, c)| c >= tau)
                .map(|&(y, _)| y)
                .collect()
        })
        .collect()
}

fn oracle_links(p: &Prepared) -> Vec<Vec<usize>> {
    let idx = |id: &str| p.inst.turns.iter().position(|t| t.id == id);
    let mut v = vec![Vec::new(); p.inst.turns.len()];
    if let (Some(o), Some(n)) = (idx(&p.inst.old_id), idx(&p.inst.new_id)) {
        v[n].push(o);
    }
    v
}

fn run_instance(p: &Prepared, ctx: &Ctx, tau_star: Option<f32>) -> Result<InstResult> {
    let inst = &p.inst;
    let none = vec![Vec::new(); inst.turns.len()];
    let session_of: HashMap<&str, u32> = inst
        .turns
        .iter()
        .map(|t| (t.id.as_str(), t.session))
        .collect();
    let ts_of: HashMap<&str, i64> = inst.turns.iter().map(|t| (t.id.as_str(), t.ts)).collect();
    let cand = candidate_links(p, ctx);

    // Arm a: plain store; its ranked list (history = All, no links) feeds b and the simulations.
    let (plain, _plain_dir) = build(p, ctx, &none)?;
    let mut ranked: Vec<Vec<String>> = Vec::new();
    for dim in 0..3 {
        ranked.push(recall_ids(
            &plain,
            qtext(inst, dim),
            &ctx.matrix[p.qrows[dim]],
            RANK_K,
            HistoryMode::All,
        )?);
    }
    let mut arms: HashMap<&'static str, [M; 3]> = HashMap::new();
    let mut a = [M::default(); 3];
    let mut b = [M::default(); 3];
    for dim in 0..3 {
        a[dim] = measure(inst, &ranked[dim], &session_of);
        let mut top: Vec<String> = ranked[dim].iter().take(SHOW).cloned().collect();
        top.sort_by_key(|id| std::cmp::Reverse(ts_of[id.as_str()]));
        b[dim] = measure(inst, &top, &session_of);
    }
    arms.insert("a", a);
    arms.insert("b", b);

    let sim = |links: &[Vec<usize>]| -> Vec<Vec<String>> {
        let graph = SimGraph::new(inst, links);
        let memo = RefCell::new(HashMap::new());
        ranked.iter().map(|r| follow(r, &graph, &memo)).collect()
    };
    let d_sim: Vec<[M; 3]> = TAUS
        .iter()
        .map(|&tau| {
            let lists = sim(&links_at(&cand, tau));
            std::array::from_fn(|d| measure(inst, &lists[d], &session_of))
        })
        .collect();

    // Arm c: oracle links in a real store (TEST only, to save DEV compute).
    if tau_star.is_some() {
        let oracle = oracle_links(p);
        let (store_c, _dir_c) = build(p, ctx, &oracle)?;
        let graph_c = WarmGraph {
            memory: &store_c,
            cache: RefCell::new(HashMap::new()),
        };
        let memo_c = RefCell::new(HashMap::new());
        let mut c = [M::default(); 3];
        let mut cf = [M::default(); 3];
        for dim in 0..3 {
            let q = &ctx.matrix[p.qrows[dim]];
            let hits = recall_ids(&store_c, qtext(inst, dim), q, RANK_K, HistoryMode::All)?;
            c[dim] = measure(inst, &follow(&hits, &graph_c, &memo_c), &session_of);
            cf[dim] = measure(
                inst,
                &recall_ids(
                    &store_c,
                    qtext(inst, dim),
                    q,
                    SHOW as u32,
                    HistoryMode::Current,
                )?,
                &session_of,
            );
        }
        arms.insert("c", c);
        arms.insert("c_filter", cf);
    }

    // Arm d: real store with the DEV-chosen tau (TEST only).
    let mut mismatch = None;
    if let Some(tau) = tau_star {
        let links = links_at(&cand, tau);
        let (store_d, _dir_d) = build(p, ctx, &links)?;
        let graph_d = WarmGraph {
            memory: &store_d,
            cache: RefCell::new(HashMap::new()),
        };
        let memo_d = RefCell::new(HashMap::new());
        let mut d = [M::default(); 3];
        let mut diff = 0;
        let simulated = sim(&links);
        for dim in 0..3 {
            let hits = recall_ids(
                &store_d,
                qtext(inst, dim),
                &ctx.matrix[p.qrows[dim]],
                RANK_K,
                HistoryMode::All,
            )?;
            ensure!(
                hits == ranked[dim],
                "ranking changed with links on {}",
                inst.uid
            );
            let real = follow(&hits, &graph_d, &memo_d);
            diff += usize::from(real != simulated[dim]);
            d[dim] = measure(inst, &real, &session_of);
        }
        arms.insert("d", d);
        mismatch = Some(diff);
    }
    Ok(InstResult {
        arms,
        d_sim,
        d_sim_mismatch_vs_real: mismatch,
    })
}

fn rate(
    items: &[(&Prepared, &InstResult)],
    arm: &str,
    f: impl Fn(&M) -> bool,
    filter: impl Fn(&Prepared) -> bool,
    dim: Option<usize>,
) -> Value {
    let (mut n, mut hit) = (0usize, 0usize);
    for (p, r) in items.iter().filter(|(p, _)| filter(p)) {
        let _ = p;
        let Some(ms) = r.arms.get(arm) else { continue };
        for (d, m) in ms.iter().enumerate() {
            if dim.is_none_or(|x| x == d) {
                n += 1;
                hit += usize::from(f(m));
            }
        }
    }
    json!({ "n": n, "rate": if n == 0 { Value::Null } else { json!(hit as f64 / n as f64) } })
}

fn summarize(items: &[(&Prepared, &InstResult)], arm: &str) -> Value {
    let mut out = serde_json::Map::new();
    let scopes: [(&str, Box<dyn Fn(&Prepared) -> bool>); 3] = [
        ("pooled", Box::new(|_| true)),
        ("T1", Box::new(|p| p.inst.kind == "T1")),
        ("T2", Box::new(|p| p.inst.kind == "T2")),
    ];
    for (name, filter) in &scopes {
        let metrics: [(&str, fn(&M) -> bool); 6] = [
            ("cva1", |m| m.cv1),
            ("new_in5", |m| m.new5),
            ("stale1", |m| m.stale1),
            ("stale5", |m| m.stale5),
            ("cva_session1", |m| m.sess1),
            ("old_sibling1", |m| m.oldsib1),
        ];
        let mut scope = serde_json::Map::new();
        for (mname, f) in metrics {
            scope.insert(mname.into(), rate(items, arm, f, filter, None));
        }
        for (d, dname) in DIMS.iter().enumerate() {
            scope.insert(
                format!("cva1_{dname}"),
                rate(items, arm, |m| m.cv1, filter, Some(d)),
            );
        }
        out.insert((*name).into(), Value::Object(scope));
    }
    Value::Object(out)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Paired cluster bootstrap (resample instances) of mean CVA@1 difference x - y, in points.
fn bootstrap(items: &[(&Prepared, &InstResult)], x: &str, y: &str, kind: Option<&str>) -> Value {
    let rows: Vec<f64> = items
        .iter()
        .filter(|(p, _)| kind.is_none_or(|k| p.inst.kind == k))
        .filter_map(|(_, r)| {
            let (a, b) = (r.arms.get(x)?, r.arms.get(y)?);
            let s = |ms: &[M; 3]| ms.iter().filter(|m| m.cv1).count() as f64 / 3.0;
            Some(s(a) - s(b))
        })
        .collect();
    if rows.is_empty() {
        return Value::Null;
    }
    let n = rows.len();
    let mean = rows.iter().sum::<f64>() / n as f64;
    let mut rng = Rng(SEED);
    let mut draws: Vec<f64> = (0..10_000)
        .map(|_| {
            (0..n)
                .map(|_| rows[(rng.next() % n as u64) as usize])
                .sum::<f64>()
                / n as f64
        })
        .collect();
    draws.sort_by(f64::total_cmp);
    json!({ "points": 100.0 * mean, "lo95": 100.0 * draws[249], "hi95": 100.0 * draws[9_749], "instances": n })
}

fn pooled_cva(items: &[(&Prepared, &InstResult)], arm: &str) -> f64 {
    rate(items, arm, |m| m.cv1, |_| true, None)["rate"]
        .as_f64()
        .unwrap_or(f64::NAN)
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(Sha256::digest(fs::read(path)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let probe = PathBuf::from(arg(&args, "--probe").context("--probe")?);
    let pack = PathBuf::from(arg(&args, "--pack").context("--pack")?);
    let out = PathBuf::from(arg(&args, "--out").context("--out")?);
    let scratch =
        PathBuf::from(arg(&args, "--scratch").unwrap_or_else(|| "/Volumes/C/.hms-tmp".into()));
    let limit: Option<usize> = arg(&args, "--limit").map(|v| v.parse()).transpose()?;
    let limit_dev: Option<usize> = arg(&args, "--limit-dev")
        .map(|v| v.parse())
        .transpose()?
        .or(limit);
    let limit_test: Option<usize> = arg(&args, "--limit-test")
        .map(|v| v.parse())
        .transpose()?
        .or(limit);
    fs::create_dir_all(&scratch)?;
    let started = Instant::now();

    let mut insts: Vec<Inst> = fs::read_to_string(&probe)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    {
        let mut kept = HashMap::new();
        insts.retain(|i| {
            let c = kept
                .entry((i.split.clone(), i.kind.clone()))
                .or_insert(0usize);
            *c += 1;
            let cap = if i.split == "dev" {
                limit_dev
            } else {
                limit_test
            };
            cap.is_none_or(|n| *c <= n)
        });
    }
    for i in &insts {
        ensure!(
            i.turns.iter().all(|t| t.ts < AS_OF),
            "timestamp after AS_OF in {}",
            i.uid
        );
        ensure!(
            i.turns.iter().any(|t| t.id == i.old_id) && i.turns.iter().any(|t| t.id == i.new_id),
            "missing conflict turn in {}",
            i.uid
        );
        ensure!(
            i.turns.windows(2).all(|w| w[0].ts <= w[1].ts),
            "turns not chronological in {}",
            i.uid
        );
    }

    let embedder = Embedder::new(&EmbedOptions {
        sha256: None,
        offline: Some(true),
        ..EmbedOptions::model(pack.display().to_string())
    })?;
    eprintln!(
        "embedder {} rev {} dim {} pack {}",
        embedder.model(),
        embedder.revision(),
        embedder.dimensions(),
        embedder.pack_sha256()
    );

    // Unique texts -> matrix rows (passages and queries are separate spaces of rows).
    let mut matrix: Vec<Vec<f32>> = Vec::new();
    let mut passage_row: HashMap<String, usize> = HashMap::new();
    let mut query_row: HashMap<String, usize> = HashMap::new();
    let mut passages: Vec<&str> = Vec::new();
    let mut queries: Vec<&str> = Vec::new();
    for i in &insts {
        for t in &i.turns {
            if !passage_row.contains_key(&t.text) {
                passage_row.insert(t.text.clone(), usize::MAX);
                passages.push(&t.text);
            }
        }
        for q in [&i.queries.dim1, &i.queries.dim2, &i.queries.dim3] {
            if !query_row.contains_key(q) {
                query_row.insert(q.clone(), usize::MAX);
                queries.push(q);
            }
        }
    }
    eprintln!(
        "{} instances, {} unique turns, {} unique queries",
        insts.len(),
        passages.len(),
        queries.len()
    );
    for chunk in passages.chunks(1024) {
        let e = embedder.embed_passages(chunk)?;
        for (text, v) in chunk.iter().zip(e) {
            passage_row.insert((*text).to_string(), matrix.len());
            matrix.push(v);
        }
        eprintln!(
            "embedded {} passages, {:.0}s",
            matrix.len(),
            started.elapsed().as_secs_f64()
        );
    }
    for chunk in queries.chunks(256) {
        let e = embedder.embed_query(chunk)?;
        for (text, v) in chunk.iter().zip(e) {
            query_row.insert((*text).to_string(), matrix.len());
            matrix.push(v);
        }
    }
    for v in &matrix {
        ensure!(
            (dot(v, v) - 1.0).abs() < 1e-3,
            "embeddings are not unit length"
        );
    }

    let prepared: Vec<Prepared> = insts
        .into_iter()
        .map(|inst| Prepared {
            rows: inst.turns.iter().map(|t| passage_row[&t.text]).collect(),
            qrows: [
                query_row[&inst.queries.dim1],
                query_row[&inst.queries.dim2],
                query_row[&inst.queries.dim3],
            ],
            inst,
        })
        .collect();
    let ctx = Ctx {
        matrix: &matrix,
        space: embedder.space(),
        scratch: &scratch,
    };

    let dev: Vec<&Prepared> = prepared.iter().filter(|p| p.inst.split == "dev").collect();
    let test: Vec<&Prepared> = prepared.iter().filter(|p| p.inst.split == "test").collect();
    let run = |set: &[&Prepared], tau: Option<f32>| -> Result<Vec<InstResult>> {
        set.par_iter().map(|p| run_instance(p, &ctx, tau)).collect()
    };

    let dev_results = run(&dev, None)?;
    eprintln!("dev done, {:.0}s", started.elapsed().as_secs_f64());
    let dev_items: Vec<(&Prepared, &InstResult)> =
        dev.iter().copied().zip(dev_results.iter()).collect();
    let mut tau_table = Vec::new();
    let mut best: Option<(f64, f32)> = None;
    for (ti, &tau) in TAUS.iter().enumerate() {
        let cva = {
            let (mut n, mut hit) = (0usize, 0usize);
            for (_, r) in &dev_items {
                for m in &r.d_sim[ti] {
                    n += 1;
                    hit += usize::from(m.cv1);
                }
            }
            hit as f64 / n.max(1) as f64
        };
        tau_table.push(json!({ "tau": tau, "dev_pooled_cva1": cva }));
        if best.is_none_or(|(b, _)| cva >= b) {
            best = Some((cva, tau));
        }
    }
    let Some((_, tau_star)) = best else {
        bail!("no dev instances")
    };
    eprintln!("tau* = {tau_star}");

    let test_results = run(&test, Some(tau_star))?;
    eprintln!("test done, {:.0}s", started.elapsed().as_secs_f64());
    let test_items: Vec<(&Prepared, &InstResult)> =
        test.iter().copied().zip(test_results.iter()).collect();

    let mut dev_arms = serde_json::Map::new();
    let mut test_arms = serde_json::Map::new();
    for arm in ARMS {
        dev_arms.insert(arm.into(), summarize(&dev_items, arm));
        test_arms.insert(arm.into(), summarize(&test_items, arm));
    }
    let mismatches: usize = test_results
        .iter()
        .filter_map(|r| r.d_sim_mismatch_vs_real)
        .sum();
    let (a, b, c, d) = (
        pooled_cva(&test_items, "a"),
        pooled_cva(&test_items, "b"),
        pooled_cva(&test_items, "c"),
        pooled_cva(&test_items, "d"),
    );
    let decision = if a >= 0.90 {
        "INCONCLUSIVE"
    } else if 100.0 * (d - a) >= 15.0 && c >= 0.80 {
        "GO"
    } else {
        "NO-GO"
    };
    let boots: Vec<Value> = [("d", "a"), ("c", "a"), ("b", "a"), ("d", "b")]
        .iter()
        .map(|(x, y)| {
            json!({
                "x": x, "y": y,
                "pooled": bootstrap(&test_items, x, y, None),
                "T1": bootstrap(&test_items, x, y, Some("T1")),
                "T2": bootstrap(&test_items, x, y, Some("T2")),
            })
        })
        .collect();
    let result = json!({
        "protocol": "docs/research/supersession-probe.md",
        "label": "retrieval-level proxy on public STALE data; not comparable to the paper's LLM-judged accuracy",
        "seed": SEED,
        "versions": {
            "crate": env!("CARGO_PKG_VERSION"),
            "embedder_model": embedder.model(),
            "embedder_revision": embedder.revision(),
            "embedder_pack_sha256": embedder.pack_sha256(),
            "probe_jsonl_sha256": sha256_file(&probe)?,
            "stale_data_sha256": "5f3ec375179e20e2e94469e018189188f34e2e7e5f21cbecbd99fcfa648c1876",
            "stale_hf_revision": "617c51dc200b5ab09970834144c7e51c77959af0",
        },
        "config": { "as_of": AS_OF, "rank_k": RANK_K, "candidate_limit": CANDIDATES, "show": SHOW, "max_links": MAX_LINKS, "limit_dev_per_type": limit_dev, "limit_test_per_type": limit_test },
        "split": { "dev_instances": dev.len(), "test_instances": test.len() },
        "tau_sweep_dev": tau_table,
        "tau_star": tau_star,
        "dev": dev_arms,
        "test": test_arms,
        "d_real_vs_simulated_list_mismatches_test": mismatches,
        "bootstrap_test_points_cva1": boots,
        "decision": { "rule": "INCONCLUSIVE if a>=0.90; GO if d-a>=15 points and c>=0.80; else NO-GO", "a": a, "b": b, "c": c, "d": d, "result": decision },
        "elapsed_s": started.elapsed().as_secs_f64(),
    });
    fs::write(&out, serde_json::to_string_pretty(&result)?)?;
    println!(
        "{decision}: a={a:.3} b={b:.3} c={c:.3} d={d:.3} tau*={tau_star} mismatches={mismatches}"
    );
    Ok(())
}
