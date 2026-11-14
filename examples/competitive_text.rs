// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! WARM side of the competitive text benchmark (benchmarks/competitive). The input file is the
//! `{documents, queries}` JSON written by `warm-bench`; every phase is its own process so the
//! orchestrator can read the child's peak RSS and cold-start time.
//!
//! `build`  bulk-ingests the corpus into `--store` with the batch API.
//! `serve`  opens the store, answers the first query (cold start), then times `--passes`
//!          passes over all queries one at a time.
//! `ops`    replaces `--ops` documents and then deletes `--ops` others, one durable commit each.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use writerslogic_warm::core::documents::{
    EncodeMode, IngestOptions,
};
use writerslogic_warm::{DocumentInput, SearchOptions, WarmCore};

const USAGE: &str =
    "usage: competitive_text <build|serve|ops> --input FILE --store DIR --out FILE \
    [--passes N (3)] [--ops N (100)] [--t0-ns NS (0)] [--encode eager|deferred|lexical (eager)] [--analyzer NAME] \
    [--batch N (0)] [--index-budget BYTES]";

/// Command line, parsed by hand: this binary is what the harness measures as WARM's install
/// footprint, and an argument-parsing framework would be most of it.
struct Args {
    /// build | serve | ops
    phase: String,
    input: PathBuf,
    store: PathBuf,
    out: PathBuf,
    passes: usize,
    ops: usize,
    /// Unix epoch nanoseconds taken by the orchestrator just before spawning this process.
    t0_ns: u128,
    /// `build` only: eager (chunk hypervectors at ingest, the default) | deferred | lexical
    /// (BM25 index only, like-for-like with the lexical comparators).
    encode: String,
    /// `build` only: the store's lexical analyzer (`Analyzer` name; the legacy one when absent).
    analyzer: Option<writerslogic_warm::core::documents::Analyzer>,
    /// `build` only: documents per atomic batch (0 = the default, `BATCH_DOCUMENTS`).
    batch: usize,
    /// `build` only: `index_memory_budget` in bytes (the engine default when absent; 0 = never fold).
    index_budget: Option<usize>,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut words = std::env::args().skip(1);
        let phase = words.next().context(USAGE)?;
        let (mut input, mut store, mut out) = (None, None, None);
        let mut args = Args {
            phase,
            input: PathBuf::new(),
            store: PathBuf::new(),
            out: PathBuf::new(),
            passes: 3,
            ops: 100,
            t0_ns: 0,
            encode: "eager".into(),
            analyzer: None,
            batch: 0,
            index_budget: None,
        };
        while let Some(flag) = words.next() {
            let value = words
                .next()
                .with_context(|| format!("{flag} needs a value; {USAGE}"))?;
            let number = || format!("{flag} takes a non-negative integer, not {value:?}");
            match flag.as_str() {
                "--input" => input = Some(PathBuf::from(&value)),
                "--store" => store = Some(PathBuf::from(&value)),
                "--out" => out = Some(PathBuf::from(&value)),
                "--passes" => args.passes = value.parse().with_context(number)?,
                "--ops" => args.ops = value.parse().with_context(number)?,
                "--t0-ns" => args.t0_ns = value.parse().with_context(number)?,
                "--encode" => args.encode = value,
                "--analyzer" => args.analyzer = Some(value.parse()?),
                "--batch" => args.batch = value.parse().with_context(number)?,
                "--index-budget" => args.index_budget = Some(value.parse().with_context(number)?),
                _ => bail!("unknown argument {flag}; {USAGE}"),
            }
        }
        args.input = input.context(USAGE)?;
        args.store = store.context(USAGE)?;
        args.out = out.context(USAGE)?;
        Ok(args)
    }
}

#[derive(Deserialize)]
struct Input {
    documents: Vec<DocumentInput>,
    queries: Vec<Query>,
}

/// One hit as the serve report lists it.
#[derive(Serialize)]
struct Hit {
    #[serde(rename = "documentId")]
    document_id: String,
    #[serde(rename = "lexicalScore")]
    lexical_score: f64,
}

#[derive(Serialize)]
struct Row<'a> {
    id: &'a str,
    results: Vec<Hit>,
}

#[derive(Serialize)]
struct ServeReport<'a> {
    open_s: f64,
    cold_start_s: f64,
    rows: Vec<Row<'a>>,
    passes_ns: Vec<Vec<u64>>,
    memory: serde_json::Value,
}

#[derive(Deserialize)]
struct Query {
    id: String,
    text: String,
    options: SearchOptions,
}

/// The serving process's peak and current resident memory in KiB (Linux procfs: `VmHWM` and
/// `RssFile` from `status`, `Rss` and `Anonymous` from `smaps_rollup`); null elsewhere.
fn process_memory() -> serde_json::Value {
    #[cfg(target_os = "linux")]
    {
        let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        let field = |text: &str, name: &str| -> Option<u64> {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
        };
        let (status, rollup) = (read("/proc/self/status"), read("/proc/self/smaps_rollup"));
        json!({"vm_hwm_kib": field(&status, "VmHWM:"), "rss_kib": field(&rollup, "Rss:"),
            "anonymous_kib": field(&rollup, "Anonymous:"), "rss_file_kib": field(&status, "RssFile:")})
    }
    #[cfg(not(target_os = "linux"))]
    serde_json::Value::Null
}

fn epoch_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Largest batch the build phase commits: documents and source bytes. Documents are moved into
/// the engine, so a batch that exceeds an engine limit is an error rather than a retry.
const BATCH_DOCUMENTS: usize = 4096;
const BATCH_SOURCE_BYTES: usize = 6 << 20;

/// Streams the `documents` array of the input file through `commit` in batches, so the corpus is
/// never resident next to the index: peak memory is one batch plus what the engine holds. The
/// `queries` are skipped (the build phase does not read them). Returns the documents seen.
fn stream_documents(
    path: &std::path::Path,
    fixed: usize,
    commit: &mut dyn FnMut(Vec<DocumentInput>) -> Result<()>,
) -> Result<usize> {
    use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
    struct Documents<'a> {
        limit: usize,
        commit: &'a mut dyn FnMut(Vec<DocumentInput>) -> Result<()>,
        seen: &'a mut usize,
    }
    impl<'de> Visitor<'de> for Documents<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an array of documents")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            let mut batch: Vec<DocumentInput> = Vec::new();
            let mut bytes = 0usize;
            while let Some(document) = seq.next_element::<DocumentInput>()? {
                bytes += document.text.len();
                batch.push(document);
                *self.seen += 1;
                if batch.len() >= self.limit || bytes >= BATCH_SOURCE_BYTES {
                    (self.commit)(std::mem::take(&mut batch)).map_err(serde::de::Error::custom)?;
                    bytes = 0;
                }
            }
            if !batch.is_empty() {
                (self.commit)(batch).map_err(serde::de::Error::custom)?;
            }
            Ok(())
        }
    }
    struct Seed<'a>(Documents<'a>);
    impl<'de> DeserializeSeed<'de> for Seed<'_> {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            d.deserialize_seq(self.0)
        }
    }
    struct Top<'a>(Documents<'a>);
    impl<'de> Visitor<'de> for Top<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("the input object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            let mut documents = Some(self.0);
            while let Some(key) = map.next_key::<String>()? {
                match (key.as_str(), documents.take()) {
                    ("documents", Some(documents)) => map.next_value_seed(Seed(documents))?,
                    _ => {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
            }
            Ok(())
        }
    }
    let mut seen = 0usize;
    let limit = if fixed > 0 { fixed } else { BATCH_DOCUMENTS };
    let reader = std::io::BufReader::with_capacity(1 << 16, std::fs::File::open(path)?);
    let mut json = serde_json::Deserializer::from_reader(reader);
    json.deserialize_map(Top(Documents {
        limit,
        commit,
        seen: &mut seen,
    }))?;
    Ok(seen)
}

fn main() -> Result<()> {
    let args = Args::parse()?;
    let store = args.store.display().to_string();
    let load = || -> Result<Input> { Ok(serde_json::from_slice(&std::fs::read(&args.input)?)?) };
    let report = match args.phase.as_str() {
        "build" => {
            ensure!(!args.store.exists(), "store exists; remove it first");
            let mode: EncodeMode = args.encode.parse()?;
            let options = IngestOptions {
                encode: mode,
                ..Default::default()
            };
            let mut config = writerslogic_warm::core::WarmConfig {
                analyzer: args.analyzer,
                ..Default::default()
            };
            if let Some(budget) = args.index_budget {
                config.document_checkpoint.index_memory_budget = budget;
            }
            let core = WarmCore::new(1024, Some(store), Some(config))?;
            // Only the engine calls are timed: reading and parsing the corpus is the caller's.
            let mut memorize = std::time::Duration::ZERO;
            let count = stream_documents(&args.input, args.batch, &mut |batch| {
                let start = Instant::now();
                core.memorize_documents_with(batch, &options)?;
                memorize += start.elapsed();
                Ok(())
            })?;
            let ingest_s = memorize.as_secs_f64();
            // `close_s` is the durable close (the final checkpoint) that `drop` runs.
            let closing = Instant::now();
            drop(core);
            json!({"documents": count, "ingest_s": ingest_s,
                "close_s": closing.elapsed().as_secs_f64(),
                "encode": mode.name()})
        }
        "serve" => {
            let input = load()?;
            ensure!(
                args.passes > 0 && !input.queries.is_empty(),
                "nothing to time"
            );
            let start = Instant::now();
            let core = WarmCore::new(1024, Some(store), None)?;
            let open_s = start.elapsed().as_secs_f64();
            let first = &input.queries[0];
            core.search_documents(&first.text, &first.options)?;
            let cold_start_s = (epoch_ns().saturating_sub(args.t0_ns)) as f64 / 1e9;
            for query in input.queries.iter().take(16) {
                core.search_documents(&query.text, &query.options)?;
            }
            let mut rows = Vec::new();
            let mut passes = Vec::new();
            for pass in 0..args.passes {
                let mut ns = Vec::with_capacity(input.queries.len());
                for query in &input.queries {
                    let t = Instant::now();
                    let results = core.search_documents(&query.text, &query.options)?;
                    ns.push(t.elapsed().as_nanos() as u64);
                    if pass == 0 {
                        // The same fields the Python workers keep: id and score per hit.
                        let results = results
                            .into_iter()
                            .map(|r| Hit {
                                document_id: r.document_id,
                                lexical_score: r.score,
                            })
                            .collect();
                        rows.push(Row {
                            id: &query.id,
                            results,
                        });
                    }
                }
                passes.push(ns);
            }
            // Streamed to the file: the report is never held as a JSON value tree.
            let report = ServeReport {
                open_s,
                cold_start_s,
                rows,
                passes_ns: passes,
                memory: process_memory(),
            };
            let mut out = std::io::BufWriter::new(std::fs::File::create(&args.out)?);
            serde_json::to_writer(&mut out, &report)?;
            std::io::Write::flush(&mut out)?;
            return Ok(());
        }
        "ops" => {
            let input = load()?;
            ensure!(
                input.documents.len() >= 2 * args.ops,
                "corpus too small for the ops count"
            );
            let core = WarmCore::new(1024, Some(store), None)?;
            let t = Instant::now();
            for document in input.documents.iter().take(args.ops) {
                let mut changed = document.clone();
                changed.text.push_str(" benchmarkrevision");
                changed.version = Some("2".into());
                core.memorize_document(changed)?;
            }
            let replace_s = t.elapsed().as_secs_f64();
            let t = Instant::now();
            for document in input.documents.iter().skip(args.ops).take(args.ops) {
                ensure!(core.delete_document(&document.id)?, "document missing");
            }
            let delete_s = t.elapsed().as_secs_f64();
            json!({"count": args.ops,
                "replace": {"supported": true, "per_op_ms_durable": replace_s / args.ops as f64 * 1e3},
                "delete": {"supported": true, "per_op_ms_durable": delete_s / args.ops as f64 * 1e3}})
        }
        other => anyhow::bail!("unknown phase {other}"),
    };
    std::fs::write(args.out, serde_json::to_vec(&report)?)?;
    Ok(())
}
