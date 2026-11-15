// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! WARM side of the hybrid contract (benchmarks/competitive/hybrid/README.md). One process per phase.
//!
//! `embed`        embeds document passages and/or queries with WARM's local embedder into raw little-endian f32 files
//!                that every system indexes or queries.
//! `build`        ingests the corpus with those document vectors (no embedding at ingest) into `--store`.
//! `serve`        opens the store and times single-thread queries: a pass from the raw query string (the engine
//!                embeds inside the timer) and passes with the query vector supplied.
//! `embed-serve`  helper for comparators: reads length-prefixed UTF-8 queries on stdin, answers with one f32 vector
//!                each; a length of u32::MAX ends it and is answered with its CPU seconds as an f64.
//! `dump`         untimed: lexical-only, vector-only and the engine's own fused lists of every query, for the
//!                fusion ablation on development data.

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use writerslogic_warm::core::documents::{
    BatchTooLarge, DocumentResult, EncodeMode, IngestOptions, MAX_BATCH_ITEMS,
};
use writerslogic_warm::core::embed::{EmbedOptions, Embedder};
use writerslogic_warm::core::encoding::TextEncoder;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{DocumentInput, SearchOptions, WarmCore};

const USAGE: &str = "usage: competitive_hybrid <embed|build|serve|embed-serve|dump> --input FILE [--store DIR] [--out FILE] \
    [--doc-vectors FILE] [--query-vectors FILE] [--embed-docs true|false] [--threads N] [--passes N (2)] \
    [--including N (100)] [--timed N (400)] [--fusion NAME] [--candidate-limit N] [--lexical-weight X] [--semantic-weight X]";

#[derive(Deserialize)]
struct Input {
    documents: Vec<DocumentInput>,
    queries: Vec<Query>,
}

#[derive(Deserialize)]
struct Query {
    id: String,
    text: String,
}

struct Args {
    phase: String,
    flags: HashMap<String, String>,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut words = std::env::args().skip(1);
        let phase = words.next().context(USAGE)?;
        let mut flags = HashMap::new();
        while let Some(flag) = words.next() {
            let name = flag
                .strip_prefix("--")
                .with_context(|| format!("{flag}: {USAGE}"))?;
            flags.insert(
                name.to_owned(),
                words
                    .next()
                    .with_context(|| format!("{flag} needs a value"))?,
            );
        }
        Ok(Self { phase, flags })
    }
    fn text(&self, name: &str) -> Option<&str> {
        self.flags.get(name).map(String::as_str)
    }
    fn path(&self, name: &str) -> Result<PathBuf> {
        Ok(PathBuf::from(self.text(name).with_context(|| {
            format!("--{name} is required; {USAGE}")
        })?))
    }
    fn number<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T> {
        match self.text(name) {
            Some(v) => v
                .parse()
                .map_err(|_| anyhow::anyhow!("--{name} takes a number, not {v:?}")),
            None => Ok(default),
        }
    }
    fn optional<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>> {
        self.text(name)
            .map(|v| {
                v.parse()
                    .map_err(|_| anyhow::anyhow!("--{name} takes a number, not {v:?}"))
            })
            .transpose()
    }
    /// Search options of the contract: top 100, everything else the engine's default unless a flag overrides it.
    fn search_options(&self) -> Result<SearchOptions> {
        Ok(SearchOptions {
            k: Some(100),
            candidate_limit: self.optional("candidate-limit")?,
            lexical_weight: self.optional("lexical-weight")?,
            semantic_weight: self.optional("semantic-weight")?,
            fusion: self.text("fusion").map(str::to_owned),
            ..Default::default()
        })
    }
}

fn embedder(threads: usize) -> Result<Embedder> {
    Embedder::new(&EmbedOptions {
        threads,
        ..Default::default()
    })
}

fn write_f32(out: &mut impl Write, vectors: &[Vec<f32>]) -> Result<()> {
    for vector in vectors {
        for value in vector {
            out.write_all(&value.to_le_bytes())?;
        }
    }
    Ok(())
}

fn read_f32(path: &Path, dim: usize, count: usize) -> Result<Vec<Vec<f32>>> {
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    ensure!(
        bytes.len() == count * dim * 4,
        "{}: {} bytes, expected {count} x {dim} f32",
        path.display(),
        bytes.len()
    );
    Ok(bytes
        .chunks_exact(dim * 4)
        .map(|row| {
            row.as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect()
        })
        .collect())
}

/// User plus system CPU seconds of this process (Linux), the figure the orchestrator's foreign-CPU gate subtracts.
fn cpu_seconds() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit(')').next().unwrap_or("");
    let fields: Vec<&str> = after.split_whitespace().collect();
    let tick = |i: usize| {
        fields
            .get(i)
            .and_then(|f| f.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (tick(11) + tick(12)) / 100.0
}

fn process_memory() -> serde_json::Value {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let field = |text: &str, name: &str| -> Option<u64> {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
    };
    let status = read("/proc/self/status");
    json!({"vm_hwm_kib": field(&status, "VmHWM:"), "rss_kib": field(&status, "VmRSS:")})
}

fn ingest(core: &WarmCore, documents: &[DocumentInput], options: &IngestOptions) -> Result<()> {
    let mut size = MAX_BATCH_ITEMS;
    let mut at = 0;
    while at < documents.len() {
        let end = (at + size).min(documents.len());
        match core.memorize_documents_with(documents[at..end].to_vec(), options) {
            Ok(_) => at = end,
            Err(error) if size > 1 && error.is::<BatchTooLarge>() => size /= 2,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn rows_of(results: &[DocumentResult]) -> Vec<(&str, f64)> {
    results
        .iter()
        .map(|r| (r.document_id.as_str(), r.score))
        .collect()
}

fn main() -> Result<()> {
    let args = Args::parse()?;
    if args.phase == "embed-serve" {
        return embed_serve(&args);
    }
    let input: Input = serde_json::from_slice(&std::fs::read(args.path("input")?)?)?;
    let report = match args.phase.as_str() {
        "embed" => embed(&args, &input)?,
        "build" => build(&args, input)?,
        "serve" => return serve(&args, &input),
        "dump" => return dump(&args, &input),
        other => bail!("unknown phase {other}; {USAGE}"),
    };
    std::fs::write(args.path("out")?, serde_json::to_vec(&report)?)?;
    Ok(())
}

fn embed(args: &Args, input: &Input) -> Result<serde_json::Value> {
    let threads = args.number("threads", 0usize)?;
    let embedder = embedder(threads)?;
    let (mut docs_s, mut queries_s) = (0.0, 0.0);
    if args.text("embed-docs") != Some("false") {
        if let Some(path) = args.text("doc-vectors") {
            let start = Instant::now();
            let mut out = BufWriter::new(std::fs::File::create(path)?);
            let total = input.documents.len();
            for (n, batch) in input.documents.chunks(256).enumerate() {
                let texts: Vec<&str> = batch.iter().map(|d| d.text.as_str()).collect();
                write_f32(&mut out, &embedder.embed_passages(&texts)?)?;
                if n % 20 == 0 {
                    eprintln!(
                        "docs {}/{} {:.0}s",
                        (n * 256).min(total),
                        total,
                        start.elapsed().as_secs_f64()
                    );
                }
            }
            out.flush()?;
            docs_s = start.elapsed().as_secs_f64();
        }
    }
    if let Some(path) = args.text("query-vectors") {
        let start = Instant::now();
        let mut out = BufWriter::new(std::fs::File::create(path)?);
        for batch in input.queries.chunks(64) {
            let texts: Vec<&str> = batch.iter().map(|q| q.text.as_str()).collect();
            write_f32(&mut out, &embedder.embed_query(&texts)?)?;
        }
        out.flush()?;
        queries_s = start.elapsed().as_secs_f64();
    }
    Ok(
        json!({"model": embedder.model(), "revision": embedder.revision(), "pack_sha256": embedder.pack_sha256(),
        "quantization": format!("{:?}", embedder.quantization()), "dimensions": embedder.dimensions(),
        "max_length": embedder.max_length(), "threads": embedder.threads(), "documents": input.documents.len(),
        "queries": input.queries.len(), "docs_s": docs_s, "queries_s": queries_s}),
    )
}

fn build(args: &Args, input: Input) -> Result<serde_json::Value> {
    let store = args.path("store")?;
    ensure!(!store.exists(), "store exists; remove it first");
    let embedder = embedder(args.number("threads", 1usize)?)?;
    let dim = embedder.dimensions();
    let vectors = read_f32(&args.path("doc-vectors")?, dim, input.documents.len())?;
    let mut documents = input.documents;
    for (document, vector) in documents.iter_mut().zip(vectors) {
        document.store_text = Some(false);
        document.embeddings = Some(vec![vector.into_iter().map(f64::from).collect()]);
    }
    let start = Instant::now();
    let core = WarmCore::new(
        1024,
        Some(store.display().to_string()),
        Some(store_config()),
    )?;
    ingest(
        &core,
        &documents,
        &IngestOptions {
            encode: EncodeMode::Lexical,
            ..Default::default()
        },
    )?;
    Ok(
        json!({"documents": documents.len(), "ingest_s": start.elapsed().as_secs_f64(), "cpu_s": cpu_seconds()}),
    )
}

/// The configuration a store is built and reopened with: the local embedder, one thread, for the single-thread contract.
fn store_config() -> WarmConfig {
    WarmConfig {
        text_encoder: TextEncoder::LocalEmbedV1,
        local_embed: Some(EmbedOptions {
            threads: 1,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn open(args: &Args) -> Result<(WarmCore, Arc<Embedder>)> {
    let core = WarmCore::new(
        1024,
        Some(args.path("store")?.display().to_string()),
        Some(store_config()),
    )?;
    Ok((core, Arc::new(embedder(1)?)))
}

fn serve(args: &Args, input: &Input) -> Result<()> {
    ensure!(!input.queries.is_empty(), "nothing to time");
    let started = Instant::now();
    let (core, embedder) = open(args)?;
    let open_s = started.elapsed().as_secs_f64();
    let dim = embedder.dimensions();
    let vectors = read_f32(&args.path("query-vectors")?, dim, input.queries.len())?;
    let supplied: Vec<Vec<f64>> = vectors
        .iter()
        .map(|v| v.iter().copied().map(f64::from).collect())
        .collect();
    let options = args.search_options()?;
    let with_vector = |i: usize| SearchOptions {
        embedding: Some(supplied[i].clone()),
        ..options.clone()
    };
    let including = args.number("including", 100usize)?.min(input.queries.len());
    let timed = args.number("timed", 400usize)?.min(input.queries.len());
    for (i, q) in input.queries.iter().take(16).enumerate() {
        core.search_documents(&q.text, &options)?;
        core.search_documents(&q.text, &with_vector(i))?;
    }
    // Untimed: every query's ranking with the query vector supplied, the rows quality is computed from.
    let mut rows = Vec::with_capacity(input.queries.len());
    for (i, q) in input.queries.iter().enumerate() {
        let results = core.search_documents(&q.text, &with_vector(i))?;
        rows.push(json!({"id": q.id, "results": rows_of(&results)}));
    }
    // Timed from the raw string: the engine embeds the query inside the call.
    let (mut including_ns, mut embed_ns, mut mismatched) = (Vec::new(), Vec::new(), 0usize);
    for (i, q) in input.queries.iter().take(including).enumerate() {
        let t = Instant::now();
        let results = core.search_documents(&q.text, &options)?;
        including_ns.push(t.elapsed().as_nanos() as u64);
        let t = Instant::now();
        let v = embedder.embed_query(&[q.text.as_str()])?;
        embed_ns.push(t.elapsed().as_nanos() as u64);
        std::hint::black_box(&v);
        let reference = core.search_documents(&q.text, &with_vector(i))?;
        let ids =
            |r: &[DocumentResult]| r.iter().map(|h| h.document_id.clone()).collect::<Vec<_>>();
        if ids(&results) != ids(&reference) {
            mismatched += 1;
        }
    }
    // Timed with the vector supplied: retrieval and fusion alone.
    let mut passes = Vec::new();
    for _ in 0..args.number("passes", 2usize)? {
        let mut ns = Vec::with_capacity(timed);
        for (i, q) in input.queries.iter().take(timed).enumerate() {
            let t = Instant::now();
            let results = core.search_documents(&q.text, &with_vector(i))?;
            ns.push(t.elapsed().as_nanos() as u64);
            std::hint::black_box(&results);
        }
        passes.push(ns);
    }
    let report = json!({"open_s": open_s, "rows": rows, "including_ns": including_ns, "embed_ns": embed_ns,
        "excluding_passes_ns": passes, "including_id_mismatches": mismatched, "cpu_s": cpu_seconds(), "memory": process_memory()});
    let mut out = BufWriter::new(std::fs::File::create(args.path("out")?)?);
    serde_json::to_writer(&mut out, &report)?;
    out.flush()?;
    Ok(())
}

fn embed_serve(_args: &Args) -> Result<()> {
    let embedder = embedder(1)?;
    let (stdin, stdout) = (std::io::stdin(), std::io::stdout());
    let (mut input, mut output) = (stdin.lock(), BufWriter::new(stdout.lock()));
    loop {
        let mut length = [0u8; 4];
        if input.read_exact(&mut length).is_err() {
            return Ok(());
        }
        let length = u32::from_le_bytes(length);
        if length == u32::MAX {
            output.write_all(&cpu_seconds().to_le_bytes())?;
            output.flush()?;
            return Ok(());
        }
        let mut text = vec![0u8; length as usize];
        input.read_exact(&mut text)?;
        let text = String::from_utf8(text)?;
        write_f32(&mut output, &embedder.embed_query(&[text.as_str()])?)?;
        output.flush()?;
    }
}

fn dump(args: &Args, input: &Input) -> Result<()> {
    let (core, embedder) = open(args)?;
    let vectors = read_f32(
        &args.path("query-vectors")?,
        embedder.dimensions(),
        input.queries.len(),
    )?;
    let depth = args.number("depth", 1000u32)?;
    let base = SearchOptions {
        k: Some(depth),
        candidate_limit: Some(depth),
        ..Default::default()
    };
    let mut out = BufWriter::new(std::fs::File::create(args.path("out")?)?);
    for (q, vector) in input.queries.iter().zip(vectors) {
        let embedding: Vec<f64> = vector.into_iter().map(f64::from).collect();
        let lexical = core.search_documents(
            &q.text,
            &SearchOptions {
                semantic_weight: Some(0.0),
                ..base.clone()
            },
        )?;
        let semantic = core.search_documents(
            &q.text,
            &SearchOptions {
                lexical_weight: Some(0.0),
                embedding: Some(embedding.clone()),
                ..base.clone()
            },
        )?;
        let mut engine = serde_json::Map::new();
        for (name, fusion, limit) in [
            ("blend100", "blend", 100u32),
            ("blend300", "blend", 300),
            ("blend1000", "blend", 1000),
            ("rrf100", "rrf", 100),
            ("rrf1000", "rrf", 1000),
        ] {
            let options = SearchOptions {
                k: Some(100),
                candidate_limit: Some(limit),
                fusion: Some(fusion.into()),
                embedding: Some(embedding.clone()),
                ..Default::default()
            };
            let results = core.search_documents(&q.text, &options)?;
            engine.insert(name.into(), json!(rows_of(&results)));
        }
        let lexical_scores: Vec<(&str, f64)> = lexical
            .iter()
            .map(|r| (r.document_id.as_str(), r.lexical_score))
            .collect();
        let semantic_scores: Vec<(&str, f64)> = semantic
            .iter()
            .map(|r| (r.document_id.as_str(), r.semantic_score.unwrap_or(f64::NAN)))
            .collect();
        serde_json::to_writer(
            &mut out,
            &json!({"id": q.id, "lex": lexical_scores, "sem": semantic_scores, "engine": engine}),
        )?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    Ok(())
}
