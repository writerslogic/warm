// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Cost of a historical view against a current-state query on a seeded corpus
//! (docs/memory-api/TIME-TRAVEL.md). Each repetition runs, in rotated order, a live
//! `search_documents`, a search on one prebuilt view, and, at the latest and the middle
//! positions, a view opened with the default settings (no reuse budget, so it is built from
//! the journal) and a view opened with the reuse budget opted in while the same position is
//! kept, each searched once. Prints JSON.
//! `cargo run --release --example history_cost -- [documents...]` (default 1000 10000).

use std::time::Instant;

use writerslogic_warm::{DocumentInput, JournalPosition, SearchOptions, WarmCore};

const REPS: usize = 31;
const BATCH: usize = 100;
/// The reuse budget of the opt-in rows.
const OPT_IN_BUDGET: usize = 256 << 20;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn summary(mut micros: Vec<f64>) -> serde_json::Value {
    micros.sort_by(f64::total_cmp);
    serde_json::json!({
        "n": micros.len(),
        "p10_us": percentile(&micros, 0.1),
        "p50_us": percentile(&micros, 0.5),
        "p90_us": percentile(&micros, 0.9),
    })
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    percentile(&values, 0.5)
}

const WORDS: [&str; 24] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango",
    "uniform", "victor", "whiskey", "xray",
];

fn text(i: usize, revision: usize) -> String {
    (0..60)
        .map(|j| {
            format!(
                "{}{}",
                WORDS[(i * 7 + j * 13 + revision * 5) % WORDS.len()],
                (i + j + revision) % 97
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn input(i: usize, revision: usize) -> DocumentInput {
    serde_json::from_value(serde_json::json!({
        "id": format!("doc-{i}"),
        "text": text(i, revision),
        "version": revision.to_string(),
    }))
    .expect("valid input")
}

fn scale(documents: usize) -> anyhow::Result<serde_json::Value> {
    let dir = tempfile::tempdir()?;
    let core = WarmCore::new(4096, Some(dir.path().display().to_string()), None)?;
    let mut middle: Option<JournalPosition> = None;
    for start in (0..documents).step_by(BATCH) {
        let batch = (start..(start + BATCH).min(documents))
            .map(|i| input(i, 0))
            .collect();
        core.memorize_documents(batch)?;
        if middle.is_none() && start + BATCH >= documents / 2 {
            middle = Some(core.journal_position());
        }
    }
    // One single-document transaction per update (a tenth) and per delete (a twentieth).
    let updates = documents / 10;
    let deletes = documents / 20;
    for i in 0..updates {
        core.memorize_document(input(i * 7 % documents, 1))?;
    }
    for i in 0..deletes {
        core.delete_document(&format!("doc-{}", i * 11 % documents))?;
    }
    let middle = middle.expect("corpus has a middle");
    let latest = core.journal_position();
    let bounds = core.history_bounds()?;
    anyhow::ensure!(
        bounds.oldest.offset == 0,
        "history starts at the empty store"
    );
    let queries: Vec<String> = (0..REPS)
        .map(|r| {
            format!(
                "{}{} {}{}",
                WORDS[r % 24],
                r % 97,
                WORDS[(r * 5) % 24],
                (r * 3) % 97
            )
        })
        .collect();
    let options = SearchOptions::default();
    // Equality first: the view at the latest position answers as the live store does.
    let prebuilt = core.history_view(&latest)?;
    for query in &queries {
        anyhow::ensure!(
            serde_json::to_string(&prebuilt.search_documents(query, &options)?)?
                == serde_json::to_string(&core.search_documents(query, &options)?)?,
            "view and live results differ for {query:?}"
        );
    }
    // With the reuse budget opted in, a kept view answers as the live store does too.
    core.set_history_cache_budget(OPT_IN_BUDGET);
    drop(core.history_view(&latest)?);
    let cold = core.history_view(&latest)?;
    for query in &queries {
        anyhow::ensure!(
            serde_json::to_string(&cold.search_documents(query, &options)?)?
                == serde_json::to_string(&core.search_documents(query, &options)?)?,
            "reused view and live results differ for {query:?}"
        );
    }
    drop(cold);
    let mut live = Vec::new();
    let mut prebuilt_search = Vec::new();
    // [latest, middle] x [opt-in reused, default]: open alone, and open plus one search.
    let mut open = [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]];
    let mut open_search = [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]];
    let positions = [&latest, &middle];
    for (rep, query) in queries.iter().enumerate() {
        for slot in 0..6 {
            match (slot + rep) % 6 {
                0 => {
                    let t = Instant::now();
                    std::hint::black_box(core.search_documents(query, &options)?);
                    live.push(t.elapsed().as_secs_f64() * 1e6);
                }
                1 => {
                    let t = Instant::now();
                    std::hint::black_box(prebuilt.search_documents(query, &options)?);
                    prebuilt_search.push(t.elapsed().as_secs_f64() * 1e6);
                }
                other => {
                    let (position, cold) = ((other - 2) % 2, (other - 2) / 2 == 1);
                    if cold {
                        // The default: no budget, nothing kept.
                        core.set_history_cache_budget(0);
                    } else {
                        // Untimed: opt in and make sure the position is kept.
                        core.set_history_cache_budget(OPT_IN_BUDGET);
                        drop(core.history_view(positions[position])?);
                    }
                    let t = Instant::now();
                    let view = core.history_view(positions[position])?;
                    let built = t.elapsed().as_secs_f64() * 1e6;
                    std::hint::black_box(view.search_documents(query, &options)?);
                    open_search[position][usize::from(cold)].push(t.elapsed().as_secs_f64() * 1e6);
                    open[position][usize::from(cold)].push(built);
                    drop(view);
                }
            }
        }
    }
    let ratio = |a: &[f64], b: &[f64]| median(a.iter().zip(b).map(|(x, y)| x / y).collect());
    let mut out = serde_json::json!({
        "documents_ingested": documents,
        "updates": updates,
        "deletes": deletes,
        "live_documents": core.document_count(),
        "journal_bytes": latest.offset,
        "middle_offset": middle.offset,
        "live_search": summary(live.clone()),
        "prebuilt_view_search": summary(prebuilt_search.clone()),
        "paired_factor_prebuilt_search_over_live": ratio(&prebuilt_search, &live),
    });
    for (position, name) in [(0, "latest"), (1, "middle")] {
        for (cold, kind) in [(1, "default"), (0, "opt_in_reused")] {
            let o = &open[position][cold];
            let os = &open_search[position][cold];
            out[format!("view_open_{kind}_{name}")] = summary(o.clone());
            out[format!("view_open_and_search_{kind}_{name}")] = summary(os.clone());
            out[format!("paired_factor_open_{kind}_{name}_over_live")] = ratio(o, &live).into();
            out[format!("paired_factor_open_and_search_{kind}_{name}_over_live")] =
                ratio(os, &live).into();
        }
    }
    Ok(out)
}

fn main() -> anyhow::Result<()> {
    let mut sizes: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    if sizes.is_empty() {
        sizes = vec![1000, 10000];
    }
    let scales = sizes
        .into_iter()
        .map(scale)
        .collect::<anyhow::Result<Vec<_>>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "benchmark": "history_view_cost",
            "harness": "examples/history_cost.rs",
            "repetitions": REPS,
            "pairing": "per repetition, the six measurements run in rotated order with the same query",
            "dimensions": 4096,
            "words_per_document": 60,
            "ingest_batch": BATCH,
            "search": "default SearchOptions (lexical BM25, k = 10)",
            "scales": scales,
        }))?
    );
    Ok(())
}
