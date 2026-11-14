// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Native Rust tantivy arm of the competitive text benchmark (benchmarks/competitive), the
//! same process model as `competitive_text` (WARM's arm): one process per phase, a built index
//! on disk, one query at a time on one thread.
//!
//! `build`  indexes the corpus (one writer thread, `en_stem` analyzer, one commit).
//! `serve`  opens the index, answers the first query (cold start), then times `--passes`
//!          passes over all queries. Queries are parsed before the timed region, as the Python
//!          comparators receive theirs; document ids come from an array built at open, as the
//!          Lucene arm and WARM keep theirs.
//! `ops`    replaces `--ops` documents and then deletes `--ops` others, one commit each.
//!
//! Schema and analyzer match the tantivy-py arm (`text_worker.py`): `id` raw-tokenized and
//! stored, `body` indexed with positions through `en_stem`, OR over the lowercase alphanumeric
//! query word set, top 100. The crate version is pinned to the tantivy-py pin's tantivy.

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tantivy::{
    collector::{Count, TopDocs},
    doc,
    query::{Query, QueryParser},
    schema::{Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions},
    Index, IndexReader, ReloadPolicy, Term,
};

const USAGE: &str =
    "usage: competitive_text_tantivy <build|serve|ops> --input FILE --store DIR --out FILE \
    [--passes N (3)] [--ops N (100)] [--t0-ns NS (0)] [--collector topk|count (topk)]";
const K: usize = 100;
const BUILD_HEAP: usize = 200_000_000;
const OPS_HEAP: usize = 50_000_000;

struct Args {
    phase: String,
    input: PathBuf,
    store: PathBuf,
    out: PathBuf,
    passes: usize,
    ops: usize,
    t0_ns: u128,
    exhaustive: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut words = std::env::args().skip(1);
        let phase = words.next().context(USAGE)?;
        let (mut input, mut store, mut out) = (None, None, None);
        let (mut passes, mut ops, mut t0_ns) = (3, 100, 0);
        let mut exhaustive = false;
        while let Some(flag) = words.next() {
            let value = words
                .next()
                .with_context(|| format!("{flag} needs a value; {USAGE}"))?;
            let number = || format!("{flag} takes a non-negative integer, not {value:?}");
            match flag.as_str() {
                "--input" => input = Some(PathBuf::from(&value)),
                "--store" => store = Some(PathBuf::from(&value)),
                "--out" => out = Some(PathBuf::from(&value)),
                "--passes" => passes = value.parse().with_context(number)?,
                "--ops" => ops = value.parse().with_context(number)?,
                "--t0-ns" => t0_ns = value.parse().with_context(number)?,
                "--collector" => {
                    exhaustive = match value.as_str() {
                        "topk" => false,
                        "count" => true,
                        _ => bail!("--collector takes topk or count; {USAGE}"),
                    }
                }
                _ => bail!("unknown argument {flag}; {USAGE}"),
            }
        }
        Ok(Args {
            phase,
            input: input.context(USAGE)?,
            store: store.context(USAGE)?,
            out: out.context(USAGE)?,
            passes,
            ops,
            t0_ns,
            exhaustive,
        })
    }
}

#[derive(Deserialize)]
struct Document {
    id: String,
    text: String,
}

#[derive(Deserialize)]
struct Query_ {
    id: String,
    text: String,
}

#[derive(Deserialize)]
struct Input {
    documents: Vec<Document>,
    queries: Vec<Query_>,
}

/// Lowercase alphanumeric word set, sorted: the same reduction `text_worker.words` applies.
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

fn schema() -> (Schema, Field, Field) {
    let mut builder = Schema::builder();
    // Raw-tokenized id: stored, indexed for `delete_term`, and a fast field so a hit's id is a
    // column read rather than a document-store decompression.
    let id = builder.add_text_field(
        "id",
        TextOptions::default()
            .set_stored()
            .set_fast("raw")
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("raw")
                    .set_index_option(IndexRecordOption::Basic),
            ),
    );
    let body = builder.add_text_field(
        "body",
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("en_stem")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        ),
    );
    (builder.build(), id, body)
}

fn epoch_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Document ids of every segment, read once at open into an array indexed by document id, as the
/// Lucene arm and WARM keep theirs: a hit's id is then an array read. Resolving through the fast
/// field's term dictionary per hit costs a dictionary block lookup each and is not what a serving
/// tantivy application would do inside the query path.
struct Ids {
    segments: Vec<Vec<String>>,
}

impl Ids {
    fn open(reader: &IndexReader) -> Result<Self> {
        let searcher = reader.searcher();
        let mut segments = Vec::new();
        for segment in searcher.segment_readers() {
            let column = segment
                .fast_fields()
                .str("id")?
                .context("id fast field missing")?;
            let mut table = Vec::with_capacity(segment.max_doc() as usize);
            let mut buffer = String::new();
            for doc in 0..segment.max_doc() {
                let ord = column
                    .term_ords(doc)
                    .next()
                    .context("document without an id")?;
                buffer.clear();
                ensure!(
                    column.ord_to_str(ord, &mut buffer)?,
                    "id ordinal not in dictionary"
                );
                table.push(buffer.clone());
            }
            segments.push(table);
        }
        Ok(Ids { segments })
    }

    fn resolve(&self, segment: u32, doc: u32) -> &str {
        &self.segments[segment as usize][doc as usize]
    }
}

fn main() -> Result<()> {
    let args = Args::parse()?;
    let input: Input = serde_json::from_slice(&std::fs::read(&args.input)?)?;
    let (schema, id_field, body_field) = schema();
    let report = match args.phase.as_str() {
        "build" => {
            ensure!(!args.store.exists(), "store exists; remove it first");
            std::fs::create_dir_all(&args.store)?;
            let start = Instant::now();
            let index = Index::create_in_dir(&args.store, schema)?;
            let mut writer = index.writer_with_num_threads(1, BUILD_HEAP)?;
            for d in &input.documents {
                writer
                    .add_document(doc!(id_field => d.id.as_str(), body_field => d.text.as_str()))?;
            }
            writer.commit()?;
            // Merged to one segment, as the Lucene arm is force-merged: queries then pay one
            // dictionary and postings lookup, not one per segment the heap budget flushed. The
            // merge is inside the ingest clock.
            let segments_before = index.searchable_segment_ids()?;
            if segments_before.len() > 1 {
                writer.merge(&segments_before).wait()?;
            }
            writer.wait_merging_threads()?;
            let segments = index.searchable_segment_ids()?.len();
            ensure!(segments == 1, "expected one segment, found {segments}");
            json!({"documents": input.documents.len(), "ingest_s": start.elapsed().as_secs_f64(),
                "tantivy": "0.27.0", "threads": 1, "heap_bytes": BUILD_HEAP,
                "segments_before_merge": segments_before.len(), "segments": segments})
        }
        "serve" => {
            ensure!(
                args.passes > 0 && !input.queries.is_empty(),
                "nothing to time"
            );
            let start = Instant::now();
            let index = Index::open_in_dir(&args.store)?;
            let reader = index
                .reader_builder()
                .reload_policy(ReloadPolicy::Manual)
                .try_into()?;
            let searcher = reader.searcher();
            let ids = Ids::open(&reader)?;
            let open_s = start.elapsed().as_secs_f64();
            let parser = QueryParser::for_index(&index, vec![body_field]);
            let prepared: Vec<Box<dyn Query>> = input
                .queries
                .iter()
                .map(|q| parser.parse_query_lenient(&words(&q.text).join(" ")).0)
                .collect();
            // `topk` is the bare top-100 collector, which lets tantivy prune with block-max WAND.
            // `count` adds a Count collector, which forces exhaustive scoring (tantivy-py always
            // does, its `count` argument defaults to true). Neither changes the hits returned; which
            // is faster depends on the corpus, so the harness times both as separate rows.
            let top = TopDocs::with_limit(K).order_by_score();
            let top_and_count = (TopDocs::with_limit(K).order_by_score(), Count);
            let run = |query: &dyn Query| -> Result<Vec<(&str, f32)>> {
                let hits = if args.exhaustive {
                    searcher.search(query, &top_and_count)?.0
                } else {
                    searcher.search(query, &top)?
                };
                Ok(hits
                    .into_iter()
                    .map(|(score, address)| (ids.resolve(address.segment_ord, address.doc_id), score))
                    .collect())
            };
            run(prepared[0].as_ref())?;
            let cold_start_s = (epoch_ns().saturating_sub(args.t0_ns)) as f64 / 1e9;
            for query in prepared.iter().take(16) {
                run(query.as_ref())?;
            }
            let mut rows = Vec::new();
            let mut passes = Vec::new();
            for pass in 0..args.passes {
                let mut ns = Vec::with_capacity(prepared.len());
                for (q, query) in input.queries.iter().zip(&prepared) {
                    let t = Instant::now();
                    let found = run(query.as_ref())?;
                    ns.push(t.elapsed().as_nanos() as u64);
                    if pass == 0 {
                        let results: Vec<_> = found
                            .into_iter()
                            .map(|(id, score)| json!({"documentId": id, "lexicalScore": score}))
                            .collect();
                        rows.push(json!({"id": q.id, "results": results}));
                    }
                }
                passes.push(ns);
            }
            json!({"open_s": open_s, "collector": if args.exhaustive { "top-100 + Count" } else { "top-100" }, "cold_start_s": cold_start_s, "rows": rows, "passes_ns": passes})
        }
        "ops" => {
            ensure!(
                input.documents.len() >= 2 * args.ops,
                "corpus too small for the ops count"
            );
            let index = Index::open_in_dir(&args.store)?;
            let mut writer =
                index.writer_with_num_threads::<tantivy::TantivyDocument>(1, OPS_HEAP)?;
            let t = Instant::now();
            for d in input.documents.iter().take(args.ops) {
                writer.delete_term(Term::from_field_text(id_field, &d.id));
                let text = format!("{} benchmarkrevision", d.text);
                writer
                    .add_document(doc!(id_field => d.id.as_str(), body_field => text.as_str()))?;
                writer.commit()?;
            }
            let replace_s = t.elapsed().as_secs_f64();
            let t = Instant::now();
            for d in input.documents.iter().skip(args.ops).take(args.ops) {
                writer.delete_term(Term::from_field_text(id_field, &d.id));
                writer.commit()?;
            }
            let delete_s = t.elapsed().as_secs_f64();
            json!({"count": args.ops,
                "replace": {"supported": true, "per_op_ms_durable": replace_s / args.ops as f64 * 1e3},
                "delete": {"supported": true, "per_op_ms_durable": delete_s / args.ops as f64 * 1e3}})
        }
        other => bail!("unknown phase {other}"),
    };
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args.out)?);
    serde_json::to_writer(&mut out, &report)?;
    std::io::Write::flush(&mut out)?;
    Ok(())
}
