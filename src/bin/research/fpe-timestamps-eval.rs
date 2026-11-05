// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Issue #164: do fractional-power-encoded timestamps help retrieval over the memory API's
//! filter approach (`as_of` validity filter and exponential recency decay)?
//!
//! Retrieval only: no LLM, no reader, no judge, no network. Only the ids listed as `dev` in the
//! split file are read; a held-out id is never selected, and the program refuses to run if the
//! split lists an id in both sets. Abstention questions are excluded, as in `memory-api-lme`.
//!
//! Two paths over the same dev questions:
//!
//! - dense: cached all-MiniLM-L6-v2 session embeddings (`session.f32`, `queries.f32`), cosine
//!   score times a time factor; configurations are the existing filter family (plain, `as_of`
//!   filter, `as_of` filter plus exponential decay) and the FPE family (point kernel, validity
//!   window, relative-time target; with and without the hard `as_of` filter; vector encoding and
//!   the exact closed-form kernel);
//! - lexical: the memory API's BM25 recall as in `memory-api-lme`, with the FPE policy plugged
//!   in through `Memory::with_recency`.
//!
//! The decision rule is fixed in `docs/research/fpe-timestamps-164.md` before the run.

use anyhow::{ensure, Context, Result};
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use writerslogic_warm::core::experimental::fpe_time::{FpeRecency, FpeTime, Kernel};
use writerslogic_warm::core::memory_api::{
    Memory, MemoryConfig, RecallOptions, RecencyBasis, RecencySpec, RememberInput,
};
use writerslogic_warm::{SearchOptions, WarmCore};

#[derive(Parser)]
struct Args {
    /// longmemeval_s_cleaned.json
    #[arg(long)]
    data: PathBuf,
    /// benchmarks/public/longmemeval_split.json
    #[arg(long)]
    split: PathBuf,
    /// Cached embeddings: meta.json, questions.jsonl, queries.f32, session.jsonl, session.f32
    #[arg(long)]
    embeddings: PathBuf,
    /// benchmarks/results/memory_api_longmemeval_dev.json, to check the lexical baseline.
    #[arg(long)]
    committed: Option<PathBuf>,
    #[arg(long)]
    out: PathBuf,
    /// Seed of the FPE frequency draw.
    #[arg(long, default_value_t = 164)]
    seed: u64,
    #[arg(long, value_delimiter = ',', default_values_t = [1u64, 2, 3, 4, 5])]
    extra_seeds: Vec<u64>,
    /// Encoder dimension of the main grid.
    #[arg(long, default_value_t = 1024)]
    dim: usize,
    #[arg(long, value_delimiter = ',', default_values_t = [64usize, 256, 1024, 4096])]
    dims: Vec<usize>,
    #[arg(long)]
    skip_lexical: bool,
}

#[derive(Deserialize)]
struct Split {
    dev: Vec<String>,
    heldout: Vec<String>,
}

#[derive(Deserialize)]
struct Turn {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct RawQuestion {
    question_id: String,
    question_type: String,
    question: String,
    question_date: String,
    haystack_session_ids: Vec<String>,
    haystack_dates: Vec<String>,
    haystack_sessions: Vec<Vec<Turn>>,
    answer_session_ids: Vec<String>,
}

const DAY_MS: f64 = 86_400_000.0;
const KS: [usize; 4] = [1, 3, 5, 10];
/// An open validity window ends this many length scales past the evaluation time, so its far
/// edge contributes about exp(-6) at most. A far horizon would put the stratified frequency grid
/// under-resolved: a code for [a, b) is accurate only while (b - a) * (pi / 2) / (sigma * dim) << 1.
const HORIZON_SIGMAS: f64 = 6.0;
const SIGMAS: [f64; 4] = [7.0, 30.0, 90.0, 365.0];
const WEIGHTS: [f64; 3] = [0.25, 0.5, 1.0];
const HALF_LIVES: [f64; 4] = [7.0, 30.0, 90.0, 365.0];

/// "2023/05/20 (Sat) 02:21" as Unix milliseconds (UTC).
fn parse_date(text: &str) -> Result<i64> {
    let mut parts = text.split_whitespace();
    let date = parts.next().context("date")?;
    let time = parts.last().context("time")?;
    let ymd: Vec<i64> = date.split('/').map(str::parse).collect::<Result<_, _>>()?;
    let hm: Vec<i64> = time.split(':').map(str::parse).collect::<Result<_, _>>()?;
    ensure!(ymd.len() == 3 && hm.len() == 2, "unexpected date {text:?}");
    let (y, m, d) = (ymd[0] - i64::from(ymd[1] <= 2), ymd[1], ymd[2]);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Ok(((days * 24 + hm[0]) * 60 + hm[1]) * 60_000)
}

/// "N days/weeks/months/years ago", "last week/month/year" or "yesterday" in the question, as
/// days before the question date. The only query-time signal the harness reads.
fn relative_offset_days(question: &str) -> Option<f64> {
    let lower = question.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let unit = |word: &str| match word.trim_end_matches('s') {
        "day" => Some(1.0),
        "week" => Some(7.0),
        "month" => Some(30.4375),
        "year" => Some(365.25),
        _ => None,
    };
    let number = |word: &str| -> Option<f64> {
        const NAMES: [&str; 12] = [
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
            "eleven", "twelve",
        ];
        match word {
            "a" | "an" => Some(1.0),
            _ => word.parse::<f64>().ok().or_else(|| {
                NAMES
                    .iter()
                    .position(|name| *name == word)
                    .map(|i| (i + 1) as f64)
            }),
        }
    };
    for i in 0..words.len() {
        if words[i] == "yesterday" {
            return Some(1.0);
        }
        if words[i] == "last" && i + 1 < words.len() {
            if let Some(days) = unit(words[i + 1]).filter(|d| *d >= 7.0) {
                return Some(days);
            }
        }
        if i + 2 < words.len() && words[i + 2] == "ago" {
            if let (Some(n), Some(days)) = (number(words[i]), unit(words[i + 1])) {
                return Some(n * days);
            }
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Group {
    KnowledgeUpdate,
    TemporalReasoning,
    Other,
}

struct Item {
    day: f64,
    evidence: bool,
    /// An older evidence session of a question with a newer one: the oracle supersession edge
    /// (older superseded by newest). Labels, so only used in the oracle rows.
    superseded_by_newest: bool,
}

struct Q {
    raw: usize,
    id: String,
    group: Group,
    qday: f64,
    target_day: Option<f64>,
    newest_day: Option<f64>,
    items: Vec<Item>,
    base: Vec<f64>,
}

struct Embeddings {
    dim: usize,
    queries: Vec<f32>,
    sessions: Vec<f32>,
    query_index: HashMap<String, usize>,
    query_text: Vec<String>,
    session_row: HashMap<(usize, String), usize>,
}

fn read_f32(path: &Path) -> Result<Vec<f32>> {
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    ensure!(bytes.len() % 4 == 0, "{} is not f32", path.display());
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn load_embeddings(dir: &Path) -> Result<Embeddings> {
    let meta: Value = serde_json::from_slice(&std::fs::read(dir.join("meta.json"))?)?;
    let dim = meta["dim"].as_u64().context("meta dim")? as usize;
    ensure!(
        meta["normalization"] == "l2",
        "cosine needs l2-normalized rows"
    );
    let queries = read_f32(&dir.join("queries.f32"))?;
    let sessions = read_f32(&dir.join("session.f32"))?;
    let mut query_index = HashMap::new();
    let mut query_text = Vec::new();
    for (i, line) in std::fs::read_to_string(dir.join("questions.jsonl"))?
        .lines()
        .enumerate()
    {
        let row: Value = serde_json::from_str(line)?;
        query_index.insert(row["id"].as_str().context("id")?.to_owned(), i);
        query_text.push(row["text"].as_str().context("text")?.to_owned());
    }
    ensure!(queries.len() == query_text.len() * dim, "queries.f32 size");
    let mut session_row = HashMap::new();
    for line in std::fs::read_to_string(dir.join("session.jsonl"))?.lines() {
        let row: Value = serde_json::from_str(line)?;
        let q = row["q"].as_u64().context("q")? as usize;
        let e = row["e"].as_u64().context("e")? as usize;
        ensure!(
            (e + 1) * dim <= sessions.len(),
            "session row {e} out of range"
        );
        session_row
            .entry((q, row["id"].as_str().context("id")?.to_owned()))
            .or_insert(e);
    }
    Ok(Embeddings {
        dim,
        queries,
        sessions,
        query_index,
        query_text,
        session_row,
    })
}

fn build_questions(all: &[RawQuestion], dev: &BTreeSet<&str>, emb: &Embeddings) -> Result<Vec<Q>> {
    let mut out = Vec::new();
    for (raw, q) in all.iter().enumerate() {
        if !dev.contains(q.question_id.as_str()) || q.question_id.ends_with("_abs") {
            continue;
        }
        let qi = *emb
            .query_index
            .get(&q.question_id)
            .context("question has no cached embedding")?;
        ensure!(
            emb.query_text[qi] == q.question,
            "cached query text differs for {}",
            q.question_id
        );
        let query = &emb.queries[qi * emb.dim..(qi + 1) * emb.dim];
        let qday = parse_date(&q.question_date)? as f64 / DAY_MS;
        let mut seen = BTreeSet::new();
        let (mut items, mut base) = (Vec::new(), Vec::new());
        for ((id, date), session) in q
            .haystack_session_ids
            .iter()
            .zip(&q.haystack_dates)
            .zip(&q.haystack_sessions)
        {
            if !session
                .iter()
                .any(|t| t.role == "user" && !t.content.is_empty())
                || !seen.insert(id.as_str())
            {
                continue;
            }
            // The cache renames evidence sessions "answer_*" to "noans_*"; the raw id stays the
            // label, only the embedding row is looked up by the cached name.
            let cached = id.replacen("answer_", "noans_", 1);
            let row = *emb
                .session_row
                .get(&(qi, id.clone()))
                .or_else(|| emb.session_row.get(&(qi, cached)))
                .with_context(|| {
                    format!("session {id} of {} has no cached embedding", q.question_id)
                })?;
            let vector = &emb.sessions[row * emb.dim..(row + 1) * emb.dim];
            base.push(
                query
                    .iter()
                    .zip(vector)
                    .map(|(a, b)| f64::from(*a) * f64::from(*b))
                    .sum::<f64>(),
            );
            items.push(Item {
                day: parse_date(date)? as f64 / DAY_MS,
                evidence: q.answer_session_ids.contains(id),
                superseded_by_newest: false,
            });
        }
        let newest_day = items
            .iter()
            .filter(|i| i.evidence)
            .map(|i| i.day)
            .reduce(f64::max);
        if let Some(newest) = newest_day {
            for item in items.iter_mut().filter(|i| i.evidence) {
                item.superseded_by_newest = item.day < newest;
            }
        }
        let group = match q.question_type.as_str() {
            "knowledge-update" => Group::KnowledgeUpdate,
            "temporal-reasoning" => Group::TemporalReasoning,
            _ => Group::Other,
        };
        out.push(Q {
            raw,
            id: q.question_id.clone(),
            group,
            qday,
            target_day: relative_offset_days(&q.question).map(|d| qday - d),
            newest_day,
            items,
            base,
        });
    }
    // Temporal slice first, so group slices are contiguous.
    out.sort_by_key(|q| (q.group == Group::Other, q.group != Group::KnowledgeUpdate));
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Variant {
    Point,
    Window,
    Target,
}

#[derive(Clone, Copy, Debug)]
enum Method {
    Plain,
    Exp(f64),
    Fpe {
        variant: Variant,
        sigma: f64,
        weight: f64,
        exact: bool,
    },
}

#[derive(Clone, Debug)]
struct Cfg {
    name: String,
    method: Method,
    filter: bool,
    oracle: bool,
}

type GKey = (Variant, u64, bool, bool);

fn laplace_window_exact(sigma: f64, start: f64, end: f64, t: f64) -> f64 {
    let f = |s: f64| {
        if s >= t {
            1.0 - (-(s - t) / sigma).exp()
        } else {
            (-(t - s) / sigma).exp() - 1.0
        }
    };
    0.5 * (f(end) - f(start))
}

/// The time factor of every item before the mixing weight: in [0, 1].
fn g_vector(q: &Q, key: GKey, enc: &FpeTime) -> Result<Vec<f64>> {
    let (variant, sigma_bits, exact, oracle) = key;
    let sigma = f64::from_bits(sigma_bits);
    let point = |center: f64, day: f64| -> f64 {
        let sim = if exact {
            enc.kernel_value(center - day)
        } else {
            enc.similarity(center, day)
        };
        sim.max(0.0)
    };
    q.items
        .iter()
        .map(|item| {
            let window = |item: &Item| -> Result<f64> {
                let end = match (oracle, q.newest_day) {
                    (true, Some(newest)) if item.superseded_by_newest => newest,
                    _ => item.day.max(q.qday) + HORIZON_SIGMAS * sigma,
                };
                let membership = if exact {
                    laplace_window_exact(sigma, item.day, end, q.qday)
                } else {
                    let code = enc.window(item.day, end)?;
                    enc.membership(&code, q.qday)
                };
                Ok(membership.clamp(0.0, 1.0))
            };
            match variant {
                Variant::Point => Ok(point(q.qday, item.day)),
                Variant::Window => window(item),
                Variant::Target => match q.target_day {
                    Some(target) => Ok(point(target, item.day)),
                    None => window(item),
                },
            }
        })
        .collect()
}

#[derive(Clone, Copy, Default)]
struct PerQ {
    any: [bool; 4],
    all: [bool; 2],
    newest: [bool; 2],
}

fn evaluate(q: &Q, ranked: &[usize]) -> PerQ {
    let evidence: Vec<usize> = (0..q.items.len())
        .filter(|i| q.items[*i].evidence)
        .collect();
    let newest = evidence
        .iter()
        .copied()
        .max_by(|a, b| q.items[*a].day.total_cmp(&q.items[*b].day).then(b.cmp(a)));
    let top = |k: usize| &ranked[..k.min(ranked.len())];
    let mut out = PerQ::default();
    for (slot, k) in KS.iter().enumerate() {
        out.any[slot] = top(*k).iter().any(|i| q.items[*i].evidence);
    }
    for (slot, k) in [5usize, 10].iter().enumerate() {
        out.all[slot] = !evidence.is_empty() && evidence.iter().all(|e| top(*k).contains(e));
        out.newest[slot] = newest.is_some_and(|n| top(*k).contains(&n));
    }
    out
}

fn rank(q: &Q, cfg: &Cfg, g: Option<&[f64]>) -> Vec<usize> {
    let keep = |i: usize| {
        let item = &q.items[i];
        if !cfg.filter {
            return true;
        }
        let future = item.day > q.qday;
        let superseded = cfg.oracle
            && item.superseded_by_newest
            && q.newest_day.is_some_and(|newest| newest <= q.qday);
        !future && !superseded
    };
    let score = |i: usize| -> f64 {
        let base = q.base[i].max(0.0);
        match cfg.method {
            Method::Plain => base,
            Method::Exp(half_life) => {
                let age = (q.qday - q.items[i].day).max(0.0);
                base * 0.5f64.powf(age / half_life)
            }
            Method::Fpe { weight, .. } => {
                base * ((1.0 - weight) + weight * g.expect("g vector")[i])
            }
        }
    };
    let mut order: Vec<usize> = (0..q.items.len()).filter(|i| keep(*i)).collect();
    order.sort_by(|a, b| score(*b).total_cmp(&score(*a)).then(a.cmp(b)));
    order
}

fn configs() -> Vec<Cfg> {
    let mut out = Vec::new();
    let mut push = |name: String, method: Method, filter: bool, oracle: bool| {
        out.push(Cfg {
            name,
            method,
            filter,
            oracle,
        })
    };
    for oracle in [false, true] {
        let suffix = if oracle { "+oracle_supersession" } else { "" };
        if !oracle {
            push("dense_plain".into(), Method::Plain, false, false);
        }
        push(format!("dense_as_of{suffix}"), Method::Plain, true, oracle);
        for h in HALF_LIVES {
            push(
                format!("dense_as_of_exp{h}d{suffix}"),
                Method::Exp(h),
                true,
                oracle,
            );
        }
    }
    for oracle in [false, true] {
        for (variant, label) in [
            (Variant::Point, "point"),
            (Variant::Window, "window"),
            (Variant::Target, "target"),
        ] {
            for exact in [false, true] {
                for sigma in SIGMAS {
                    for weight in WEIGHTS {
                        for filter in [false, true] {
                            // The oracle only changes a config that filters or encodes windows.
                            if oracle && !filter && variant != Variant::Window {
                                continue;
                            }
                            let name = format!(
                                "fpe_{label}{}_s{sigma}d_w{weight}{}{}",
                                if exact { "_exact" } else { "" },
                                if filter { "+as_of" } else { "" },
                                if oracle { "+oracle_supersession" } else { "" },
                            );
                            push(
                                name,
                                Method::Fpe {
                                    variant,
                                    sigma,
                                    weight,
                                    exact,
                                },
                                filter,
                                oracle,
                            );
                        }
                    }
                }
            }
        }
    }
    out
}

/// Per-question results of every config at one `(dim, seed)`.
fn run_grid(qs: &[Q], cfgs: &[Cfg], dim: usize, seed: u64) -> Result<Vec<Vec<PerQ>>> {
    let encoders: HashMap<u64, FpeTime> = SIGMAS
        .iter()
        .map(|s| Ok((s.to_bits(), FpeTime::new(Kernel::Laplace, *s, dim, seed)?)))
        .collect::<Result<_>>()?;
    let mut results = vec![Vec::with_capacity(qs.len()); cfgs.len()];
    for q in qs {
        let mut cache: HashMap<GKey, Vec<f64>> = HashMap::new();
        for (c, cfg) in cfgs.iter().enumerate() {
            let g = match cfg.method {
                Method::Fpe {
                    variant,
                    sigma,
                    exact,
                    ..
                } => {
                    let key = (variant, sigma.to_bits(), exact, cfg.oracle);
                    if !cache.contains_key(&key) {
                        let g = g_vector(q, key, &encoders[&sigma.to_bits()])?;
                        cache.insert(key, g);
                    }
                    Some(cache[&key].as_slice())
                }
                _ => None,
            };
            results[c].push(evaluate(q, &rank(q, cfg, g)));
        }
    }
    Ok(results)
}

fn indices(qs: &[Q], pick: impl Fn(Group) -> bool) -> Vec<usize> {
    (0..qs.len()).filter(|i| pick(qs[*i].group)).collect()
}

fn aggregate(per_q: &[PerQ], idx: &[usize]) -> Value {
    let n = idx.len().max(1) as f64;
    let count = |f: &dyn Fn(&PerQ) -> bool| idx.iter().filter(|i| f(&per_q[**i])).count();
    json!({
        "questions": idx.len(),
        "recall_any_at_1": count(&|p| p.any[0]) as f64 / n,
        "recall_any_at_3": count(&|p| p.any[1]) as f64 / n,
        "recall_any_at_5": count(&|p| p.any[2]) as f64 / n,
        "recall_any_at_10": count(&|p| p.any[3]) as f64 / n,
        "recall_all_at_5": count(&|p| p.all[0]) as f64 / n,
        "recall_all_at_10": count(&|p| p.all[1]) as f64 / n,
        "newest_evidence_at_5": count(&|p| p.newest[0]) as f64 / n,
        "newest_evidence_at_10": count(&|p| p.newest[1]) as f64 / n,
        "hits_any_at_5": count(&|p| p.any[2]),
        "hits_any_at_10": count(&|p| p.any[3]),
    })
}

fn hits(per_q: &[PerQ], idx: &[usize], slot: usize) -> usize {
    idx.iter().filter(|i| per_q[**i].any[slot]).count()
}

/// Selection key on the temporal slice: recall_any@5, then @10, then recall_all@5.
fn selection_key(per_q: &[PerQ], idx: &[usize]) -> (usize, usize, usize) {
    (
        hits(per_q, idx, 2),
        hits(per_q, idx, 3),
        idx.iter().filter(|i| per_q[**i].all[0]).count(),
    )
}

fn best(results: &[Vec<PerQ>], cfgs: &[Cfg], idx: &[usize], pick: impl Fn(&Cfg) -> bool) -> usize {
    let mut best: Option<usize> = None;
    for c in (0..cfgs.len()).filter(|c| pick(&cfgs[*c])) {
        if best.is_none_or(|b| selection_key(&results[c], idx) > selection_key(&results[b], idx)) {
            best = Some(c);
        }
    }
    best.expect("a config matches")
}

/// Wins, losses and ties of `a` against `b` on recall_any at slot (2 = @5, 3 = @10).
fn paired(a: &[PerQ], b: &[PerQ], idx: &[usize], slot: usize) -> (usize, usize, usize) {
    idx.iter().fold((0, 0, 0), |(w, l, t), i| {
        match (a[*i].any[slot], b[*i].any[slot]) {
            (true, false) => (w + 1, l, t),
            (false, true) => (w, l + 1, t),
            _ => (w, l, t + 1),
        }
    })
}

/// Two-sided exact sign test.
fn sign_test_p(wins: usize, losses: usize) -> f64 {
    let n = wins + losses;
    if n == 0 {
        return 1.0;
    }
    let mut tail = 0.0;
    let mut choose = 1.0f64;
    for i in 0..=wins.min(losses) {
        if i > 0 {
            choose = choose * (n - i + 1) as f64 / i as f64;
        }
        tail += choose;
    }
    (2.0 * tail / 2f64.powi(n as i32)).min(1.0)
}

fn compare(label: &str, a: &str, b: &str, ra: &[PerQ], rb: &[PerQ], idx: &[usize]) -> Value {
    let mut out = json!({"comparison": label, "a": a, "b": b});
    for (slot, name) in [(2usize, "recall_any_at_5"), (3, "recall_any_at_10")] {
        let (w, l, t) = paired(ra, rb, idx, slot);
        out[name] = json!({"a_wins": w, "a_losses": l, "ties": t, "net": w as i64 - l as i64,
            "sign_test_p": sign_test_p(w, l)});
    }
    out
}

fn recall_any(ranked: &[String], evidence: &BTreeSet<&str>, k: usize) -> bool {
    ranked
        .iter()
        .take(k)
        .any(|id| evidence.contains(id.as_str()))
}

/// The memory API's BM25 recall over the temporal slice, with the FPE policy plugged in.
fn lexical(all: &[RawQuestion], qs: &[Q], seed: u64, dim: usize) -> Result<Value> {
    let mut configs: Vec<(String, Option<f64>, Option<(f64, f64)>)> = vec![
        ("document_search".into(), None, None),
        ("memory_default".into(), None, None),
        ("memory_as_of".into(), None, None),
    ];
    configs.extend(
        HALF_LIVES
            .iter()
            .map(|d| (format!("memory_recency_{d}d"), Some(*d), None)),
    );
    for sigma in SIGMAS {
        for weight in WEIGHTS {
            configs.push((
                format!("memory_fpe_s{sigma}d_w{weight}"),
                None,
                Some((sigma, weight)),
            ));
        }
    }
    let encoders: HashMap<u64, FpeTime> = SIGMAS
        .iter()
        .map(|s| Ok((s.to_bits(), FpeTime::new(Kernel::Laplace, *s, dim, seed)?)))
        .collect::<Result<_>>()?;
    let mut tallies: BTreeMap<String, (usize, BTreeMap<String, [usize; 2]>)> = BTreeMap::new();
    let temporal: Vec<&Q> = qs.iter().filter(|q| q.group != Group::Other).collect();
    for q in &temporal {
        let r = &all[q.raw];
        let dir = tempfile::tempdir()?;
        let core = Arc::new(WarmCore::new(
            1024,
            Some(dir.path().display().to_string()),
            None,
        )?);
        let question_ms = parse_date(&r.question_date)?;
        let mut facts = Vec::new();
        let mut seen = BTreeSet::new();
        for ((id, date), session) in r
            .haystack_session_ids
            .iter()
            .zip(&r.haystack_dates)
            .zip(&r.haystack_sessions)
        {
            let text: Vec<&str> = session
                .iter()
                .filter(|t| t.role == "user")
                .map(|t| t.content.as_str())
                .collect();
            if text.is_empty() || !seen.insert(id.as_str()) {
                continue;
            }
            let at = parse_date(date)?;
            facts.push(RememberInput {
                id: Some(id.clone()),
                text: text.join("\n"),
                event_time: Some(at),
                valid_from: Some(at),
                ..Default::default()
            });
        }
        let evidence: BTreeSet<&str> = r.answer_session_ids.iter().map(String::as_str).collect();
        let mut memories: BTreeMap<String, Memory> = BTreeMap::new();
        for (name, half_life, fpe) in &configs {
            let config = MemoryConfig {
                recency: half_life.map_or(RecencySpec::None, |days| RecencySpec::Exponential {
                    half_life_ms: (days * DAY_MS) as i64,
                    basis: RecencyBasis::Event,
                    floor: 0.0,
                }),
                ..Default::default()
            };
            let mut memory = Memory::with_config(core.clone(), config)?;
            if let Some((sigma, weight)) = fpe {
                memory = memory.with_recency(Arc::new(FpeRecency {
                    encoder: encoders[&sigma.to_bits()].clone(),
                    weight: *weight,
                    basis: RecencyBasis::Event,
                    unit_ms: DAY_MS,
                }));
            }
            memories.insert(name.clone(), memory);
        }
        memories["memory_default"].remember_many(facts)?;
        let options = RecallOptions {
            k: Some(10),
            as_of: Some(question_ms),
            ..Default::default()
        };
        let mut results: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        results.insert(
            "document_search",
            core.search_documents(
                &r.question,
                &SearchOptions {
                    k: Some(10),
                    ..Default::default()
                },
            )?
            .into_iter()
            .map(|d| d.document_id)
            .collect(),
        );
        for (name, _, _) in configs.iter().skip(1) {
            let recall_options = if name == "memory_default" {
                RecallOptions {
                    k: Some(10),
                    ..Default::default()
                }
            } else {
                options.clone()
            };
            let found = memories[name].recall(&r.question, &recall_options)?;
            results.insert(name, found.into_iter().map(|f| f.id).collect());
        }
        for (name, ids) in &results {
            let entry = tallies
                .entry(format!("{}::{}", r.question_type, name))
                .or_default();
            entry.0 += 1;
            let hit = entry.1.entry((*name).to_owned()).or_default();
            for (slot, k) in [5usize, 10].into_iter().enumerate() {
                hit[slot] += usize::from(recall_any(ids, &evidence, k));
            }
        }
    }
    let rows: Vec<Value> = tallies
        .iter()
        .map(|(key, (n, hits))| {
            let (question_type, config) = key.split_once("::").expect("key");
            let h = &hits[config];
            json!({"question_type": question_type, "config": config, "questions": n,
                "recall_any_at_5": h[0] as f64 / *n as f64,
                "recall_any_at_10": h[1] as f64 / *n as f64,
                "hits_at_5": h[0], "hits_at_10": h[1]})
        })
        .collect();
    Ok(json!({"questions": temporal.len(), "dim": dim, "seed": seed, "rows": rows}))
}

/// Whether the lexical rows of the existing configs equal the committed result file.
fn reproduces(lexical: &Value, committed: &Path) -> Result<Value> {
    let committed: Value = serde_json::from_slice(&std::fs::read(committed)?)?;
    let key = |row: &Value| {
        (
            row["question_type"].as_str().unwrap_or("").to_owned(),
            row["config"].as_str().unwrap_or("").to_owned(),
        )
    };
    let mine: BTreeMap<_, _> = lexical["rows"]
        .as_array()
        .context("rows")?
        .iter()
        .map(|r| (key(r), (r["hits_at_5"].clone(), r["hits_at_10"].clone())))
        .collect();
    let (mut checked, mut equal) = (0, 0);
    for row in committed["rows"].as_array().context("committed rows")? {
        if let Some(found) = mine.get(&key(row)) {
            checked += 1;
            equal += usize::from(found.0 == row["hits_at_5"] && found.1 == row["hits_at_10"]);
        }
    }
    Ok(json!({"rows_checked": checked, "rows_equal": equal}))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let split: Split = serde_json::from_slice(&std::fs::read(&args.split)?)?;
    let dev: BTreeSet<&str> = split.dev.iter().map(String::as_str).collect();
    let heldout: BTreeSet<&str> = split.heldout.iter().map(String::as_str).collect();
    ensure!(dev.is_disjoint(&heldout), "split lists an id in both sets");
    let all: Vec<RawQuestion> = serde_json::from_slice(&std::fs::read(&args.data)?)?;
    let emb = load_embeddings(&args.embeddings)?;
    let qs = build_questions(&all, &dev, &emb)?;
    ensure!(
        qs.iter().all(|q| !heldout.contains(q.id.as_str())),
        "a held-out question was selected"
    );
    let temporal = indices(&qs, |g| g != Group::Other);
    let other = indices(&qs, |g| g == Group::Other);
    let ku = indices(&qs, |g| g == Group::KnowledgeUpdate);
    let tr = indices(&qs, |g| g == Group::TemporalReasoning);

    let cfgs = configs();
    let results = run_grid(&qs, &cfgs, args.dim, args.seed)?;
    let rows: Vec<Value> = cfgs
        .iter()
        .enumerate()
        .map(|(c, cfg)| {
            json!({"config": cfg.name, "oracle_supersession": cfg.oracle,
                "temporal": aggregate(&results[c], &temporal),
                "knowledge_update": aggregate(&results[c], &ku),
                "temporal_reasoning": aggregate(&results[c], &tr),
                "other_dev": aggregate(&results[c], &other)})
        })
        .collect();

    let existing = |c: &Cfg| !c.oracle && !matches!(c.method, Method::Fpe { .. });
    let fpe_vec = |c: &Cfg| !c.oracle && matches!(c.method, Method::Fpe { exact: false, .. });
    let plain = cfgs
        .iter()
        .position(|c| c.name == "dense_plain")
        .expect("plain");
    let best_existing = best(&results, &cfgs, &temporal, existing);
    let best_fpe = best(&results, &cfgs, &temporal, fpe_vec);
    let twin_name = {
        // The exact-kernel twin differs only by the `_exact` marker after the variant.
        let name = &cfgs[best_fpe].name;
        let variant_end = name[4..].find("_s").map(|i| i + 4).expect("variant");
        format!("{}_exact{}", &name[..variant_end], &name[variant_end..])
    };
    let twin = cfgs
        .iter()
        .position(|c| c.name == twin_name)
        .context("exact twin")?;

    let comparisons = vec![
        compare(
            "fpe_vs_plain_temporal",
            &cfgs[best_fpe].name,
            "dense_plain",
            &results[best_fpe],
            &results[plain],
            &temporal,
        ),
        compare(
            "fpe_vs_best_existing_temporal",
            &cfgs[best_fpe].name,
            &cfgs[best_existing].name,
            &results[best_fpe],
            &results[best_existing],
            &temporal,
        ),
        compare(
            "fpe_vs_exact_twin_temporal",
            &cfgs[best_fpe].name,
            &cfgs[twin].name,
            &results[best_fpe],
            &results[twin],
            &temporal,
        ),
        compare(
            "best_existing_vs_plain_temporal",
            &cfgs[best_existing].name,
            "dense_plain",
            &results[best_existing],
            &results[plain],
            &temporal,
        ),
        compare(
            "fpe_vs_plain_other_dev",
            &cfgs[best_fpe].name,
            "dense_plain",
            &results[best_fpe],
            &results[plain],
            &other,
        ),
        compare(
            "best_existing_vs_plain_other_dev",
            &cfgs[best_existing].name,
            "dense_plain",
            &results[best_existing],
            &results[plain],
            &other,
        ),
    ];

    // The selected FPE config across dimensions and seeds.
    let mut sweep = Vec::new();
    let mut seed_nets = Vec::new();
    let mut seeds = vec![args.seed];
    seeds.extend(&args.extra_seeds);
    let selected_only = vec![cfgs[best_fpe].clone(), cfgs[best_existing].clone()];
    for dim in &args.dims {
        for seed in &seeds {
            let r = run_grid(&qs, &selected_only, *dim, *seed)?;
            let (w5, l5, _) = paired(&r[0], &r[1], &temporal, 2);
            let (w10, l10, _) = paired(&r[0], &r[1], &temporal, 3);
            if *dim == args.dim {
                seed_nets.push(w5 as i64 - l5 as i64);
            }
            sweep.push(json!({"dim": dim, "seed": seed,
                "fpe_temporal": aggregate(&r[0], &temporal),
                "net_vs_best_existing_at_5": w5 as i64 - l5 as i64,
                "net_vs_best_existing_at_10": w10 as i64 - l10 as i64}));
        }
    }

    let net = |name: &str, key: &str| -> i64 {
        comparisons
            .iter()
            .find(|c| c["comparison"] == name)
            .map_or(0, |c| c[key]["net"].as_i64().unwrap_or(0))
    };
    let r1 = net("fpe_vs_best_existing_temporal", "recall_any_at_5") >= 2
        || net("fpe_vs_best_existing_temporal", "recall_any_at_10") >= 2;
    let r2 = net("fpe_vs_plain_other_dev", "recall_any_at_5") >= 0;
    let r3 = seed_nets.iter().filter(|n| **n >= 1).count() >= 5;
    let decision = json!({
        "R1_fpe_beats_best_existing_by_2_questions_at_5_or_10": r1,
        "R2_no_net_loss_vs_plain_on_other_dev_at_5": r2,
        "R3_net_ge_1_vs_best_existing_in_5_of_6_seeds_at_5": r3,
        "seed_nets_at_5": seed_nets,
        "decision": if r1 && r2 && r3 { "go" } else { "no-go" },
    });

    let (lexical_value, reproduced) = if args.skip_lexical {
        (Value::Null, Value::Null)
    } else {
        let lexical_value = lexical(&all, &qs, args.seed, args.dim)?;
        let reproduced = match &args.committed {
            Some(path) => reproduces(&lexical_value, path)?,
            None => Value::Null,
        };
        (lexical_value, reproduced)
    };

    let report = json!({
        "issue": "writerslogic/warm#164",
        "dataset": "LongMemEval_S (longmemeval_s_cleaned.json), dev ids only",
        "split": {"file": "benchmarks/public/longmemeval_split.json", "set": "dev",
            "dev_ids_in_split": split.dev.len(), "heldout_ids_read": 0,
            "split_seed": 20261005},
        "seed": args.seed, "extra_seeds": args.extra_seeds, "dim": args.dim, "dims": args.dims,
        "embeddings": "cached sentence-transformers/all-MiniLM-L6-v2, session level, l2-normalized (meta.json in the cache dir)",
        "protocol": "retrieval only; no LLM, no reader, no judge, no network; abstention questions excluded; session-level items dated by the haystack date; score = max(cosine, 0) times a time factor; as_of = question date",
        "slices": {"temporal": {"types": ["knowledge-update", "temporal-reasoning"], "questions": temporal.len()},
            "knowledge_update": ku.len(), "temporal_reasoning": tr.len(), "other_dev": other.len()},
        "relative_time_target": {"parsed_in_temporal_slice": temporal.iter().filter(|i| qs[**i].target_day.is_some()).count(), "of": temporal.len()},
        "oracle_supersession_questions": qs.iter().filter(|q| q.items.iter().any(|i| i.superseded_by_newest)).count(),
        "selected": {"best_existing": cfgs[best_existing].name, "best_fpe": cfgs[best_fpe].name,
            "exact_kernel_twin_of_best_fpe": cfgs[twin].name,
            "selection": "max recall_any@5, then @10, then recall_all@5, on the temporal dev slice; first in enumeration order on a tie"},
        "comparisons": comparisons,
        "decision_rule_result": decision,
        "dim_seed_sweep_of_selected_fpe": sweep,
        "lexical": lexical_value,
        "lexical_reproduces_committed_memory_api_longmemeval_dev": reproduced,
        "grid": rows,
    });
    std::fs::write(&args.out, serde_json::to_string_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "selected": report["selected"], "comparisons": report["comparisons"],
            "decision": report["decision_rule_result"],
            "reproduces": report["lexical_reproduces_committed_memory_api_longmemeval_dev"],
        }))?
    );
    Ok(())
}
