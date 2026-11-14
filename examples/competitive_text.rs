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
    BatchTooLarge, DocumentResult, EncodeMode, IngestOptions, LexicalOptions, LexicalQuery, MAX_BATCH_ITEMS,
};
use writerslogic_warm::{DocumentInput, SearchOptions, WarmCore};

const USAGE: &str =
    "usage: competitive_text <build|serve|ops> --input FILE --store DIR --out FILE \
    [--passes N (3)] [--ops N (100)] [--t0-ns NS (0)] [--encode eager|deferred|lexical (eager)] [--analyzer NAME] \
    [--batch N (0)] [--api documents|lexical (documents)] [--store-text true|false (false)] [--digest true|false (false)]";

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
    /// `build` only: documents per atomic batch (0 = as many per batch as the engine accepts).
    batch: usize,
    /// `serve` only: `documents` calls `search_documents` (the product path for document search) with the raw
    /// query string; `lexical` calls `search_lexical` on the query's word set.
    api: String,
    /// `build` only: keep each document's text in the store (`DocumentInput::store_text`), as the
    /// documents contract needs: `search_documents` then returns the text with every hit.
    store_text: bool,
    /// `serve` only: time passes that return the full results and drop them inside the timer, then
    /// run one untimed pass that records a digest of each hit (documents contract; needs `--api documents`).
    digest: bool,
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
            api: "documents".into(),
            store_text: false,
            digest: false,
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
                "--api" => args.api = value,
                "--store-text" => args.store_text = value.parse().with_context(number)?,
                "--digest" => args.digest = value.parse().with_context(number)?,
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
    /// Untimed check of what the hit carried (documents contract): CRC-32 and byte length of the
    /// returned text, the byte range of the document it covers, and the `bucket` metadata value.
    #[serde(skip_serializing_if = "Option::is_none")]
    check: Option<Check>,
}

#[derive(Serialize)]
struct Check {
    crc: u32,
    len: usize,
    start: u32,
    end: u32,
    bucket: Option<i64>,
}

/// CRC-32 (IEEE, reflected), the digest every arm of the documents contract records.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c = CRC_TABLE[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// What `run` returns: the full results (digest mode, dropped by the caller) or ids and scores.
enum Found {
    Documents(Vec<DocumentResult>),
    Ids(Vec<Hit>),
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

/// Lowercase alphanumeric word set, sorted: the reduction every arm applies to a raw query string
/// (`text_worker.words`, the native tantivy arm).
fn words(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut out: Vec<String> = lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn epoch_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Ingests `documents` in the fewest atomic batches the engine accepts. The engine bounds a
/// batch by items, chunks, source and metadata bytes, embedding values and transaction bytes, and
/// rejects an oversized batch before writing anything; so start at `MAX_BATCH_ITEMS` documents and
/// halve on a batch-limit error, keeping the size that worked. A fixed size takes `fixed`.
fn ingest(
    core: &WarmCore,
    documents: &[DocumentInput],
    options: &IngestOptions,
    fixed: usize,
) -> Result<()> {
    let mut size = if fixed > 0 { fixed } else { MAX_BATCH_ITEMS };
    let mut at = 0;
    while at < documents.len() {
        let end = (at + size).min(documents.len());
        match core.memorize_documents_with(documents[at..end].to_vec(), options) {
            Ok(_) => at = end,
            Err(error) if fixed == 0 && size > 1 && error.is::<BatchTooLarge>() => size /= 2,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse()?;
    let input: Input = serde_json::from_slice(&std::fs::read(&args.input)?)?;
    let store = args.store.display().to_string();
    let report = match args.phase.as_str() {
        "build" => {
            ensure!(!args.store.exists(), "store exists; remove it first");
            let mode: EncodeMode = args.encode.parse()?;
            let options = IngestOptions {
                encode: mode,
                ..Default::default()
            };
            let count = input.documents.len();
            let mut documents = input.documents;
            if args.store_text {
                for document in &mut documents {
                    document.store_text = Some(true);
                }
            }
            let start = Instant::now();
            let config = writerslogic_warm::core::WarmConfig {
                analyzer: args.analyzer,
                ..Default::default()
            };
            let core = WarmCore::new(1024, Some(store), Some(config))?;
            ingest(&core, &documents, &options, args.batch)?;
            json!({"documents": count, "ingest_s": start.elapsed().as_secs_f64(),
                "encode": mode.name(), "store_text": args.store_text})
        }
        "serve" => {
            ensure!(
                args.passes > 0 && !input.queries.is_empty(),
                "nothing to time"
            );
            let start = Instant::now();
            let core = WarmCore::new(1024, Some(store), None)?;
            let open_s = start.elapsed().as_secs_f64();
            // One contract for every arm: the raw query string goes in, WARM's own query analysis is inside the
            // timed region, and the top-100 (id, score) pairs come out. `documents` is the product path for
            // document search and builds full documents per hit, more work than the contract needs (conservative);
            // `lexical` is the id-and-score API on the query's word set.
            ensure!(
                matches!(args.api.as_str(), "documents" | "lexical"),
                "--api takes documents or lexical, not {:?}",
                args.api
            );
            let lexical_api = args.api == "lexical";
            let lexical = LexicalOptions {
                k: 100,
                ..Default::default()
            };
            ensure!(
                !args.digest || !lexical_api,
                "--digest needs --api documents (the documents contract)"
            );
            let digest = args.digest;
            let run = |query: &Query| -> Result<Found> {
                if lexical_api {
                    let parsed = LexicalQuery::parse(&words(&query.text).join(" "))?;
                    Ok(Found::Ids(
                        core.search_lexical(&parsed, &lexical)?
                            .hits
                            .into_iter()
                            .map(|h| Hit {
                                document_id: h.document_id,
                                lexical_score: h.score,
                                check: None,
                            })
                            .collect(),
                    ))
                } else if digest {
                    Ok(Found::Documents(core.search_documents(&query.text, &query.options)?))
                } else {
                    Ok(Found::Ids(
                        core.search_documents(&query.text, &query.options)?
                            .into_iter()
                            .map(|r| Hit {
                                document_id: r.document_id,
                                lexical_score: r.lexical_score,
                                check: None,
                            })
                            .collect(),
                    ))
                }
            };
            // The timed unit in digest mode is the call plus freeing what it returned; the hits are
            // read back (`black_box`) so nothing is elided.
            let search = |query: &Query| -> Result<Vec<Hit>> {
                match run(query)? {
                    Found::Ids(hits) => Ok(hits),
                    Found::Documents(documents) => {
                        std::hint::black_box(&documents);
                        Ok(Vec::new())
                    }
                }
            };
            search(&input.queries[0])?;
            let cold_start_s = (epoch_ns().saturating_sub(args.t0_ns)) as f64 / 1e9;
            for query in input.queries.iter().take(16) {
                search(query)?;
            }
            let mut rows = Vec::new();
            let mut passes = Vec::new();
            for pass in 0..args.passes {
                let mut ns = Vec::with_capacity(input.queries.len());
                for query in &input.queries {
                    let t = Instant::now();
                    let results = search(query)?;
                    ns.push(t.elapsed().as_nanos() as u64);
                    if pass == 0 && !digest {
                        rows.push(Row {
                            id: &query.id,
                            results,
                        });
                    }
                }
                passes.push(ns);
            }
            if digest {
                // Untimed: what each hit carried, for the orchestrator to compare with the corpus.
                for query in &input.queries {
                    let Found::Documents(found) = run(query)? else {
                        bail!("digest pass without documents");
                    };
                    let results = found
                        .into_iter()
                        .map(|r| {
                            let text = r.text.as_deref().unwrap_or("");
                            Hit {
                                check: Some(Check {
                                    crc: crc32(text.as_bytes()),
                                    len: text.len(),
                                    start: r.start_byte,
                                    end: r.end_byte,
                                    bucket: r.metadata.get("bucket").and_then(|b| b.as_i64()),
                                }),
                                document_id: r.document_id,
                                lexical_score: r.lexical_score,
                            }
                        })
                        .collect();
                    rows.push(Row {
                        id: &query.id,
                        results,
                    });
                }
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
