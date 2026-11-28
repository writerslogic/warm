// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// One phase of the JavaScript comparison per process, so every build and every serve is a cold start.
//
//   node vectra_bench.mjs build <system> <data-dir> <store> <out.json>
//   node vectra_bench.mjs serve <system> <data-dir> <store> <out.json> [max-queries]
//
// Systems: vectra, vectra-bm25 (Vectra 0.15 `queryItems(..., isBm25=true)`), warm-vector (warm-memory/vectra
// `queryItems`, exact), warm-lexical and warm-hybrid (`searchDocuments`). <data-dir> holds docs.f32 and queries.f32
// (the same vectors for every system), corpus.json {ids, texts} and queries.json {ids, texts}.
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(import.meta.url);
// WARM_MEMORY_DIR: a warm-memory checkout built from source (index.js, vectra.js and the .node next to it); default is the pinned npm package.
const warmRoot = process.env.WARM_MEMORY_DIR ?? "warm-memory";
const warmModule = (sub) => require(process.env.WARM_MEMORY_DIR ? join(warmRoot, sub ? `${sub}.js` : "index.js") : sub ? `warm-memory/${sub}` : "warm-memory");
const [phase, system, dataDir, store, outPath, maxQueriesArg] = process.argv.slice(2);
const DIM = 384;
const TOP_K = 10;
// Vectra's BM25 option is slow and leaks per query, so that row is one timed pass without warm-up.
const PASSES = system === "vectra-bm25" ? 1 : 3;
const WARMUP = system === "vectra-bm25" ? 0 : 1;
const MODEL = { model: "granite-embedding-30m-english", revision: "int8-pack", dimensions: DIM };

function f32(path) {
  const buf = readFileSync(path);
  return new Float32Array(buf.buffer, buf.byteOffset, buf.byteLength / 4);
}
const json = (name) => JSON.parse(readFileSync(join(dataDir, name), "utf8"));
const rss = () => process.resourceUsage().maxRSS * 1024; // libuv reports KiB on Linux and macOS
const ns = () => process.hrtime.bigint();
const secs = (t0) => Number(ns() - t0) / 1e9;
// Set by each build just before its final flush: Vectra's batchInsertItems persists inside the call, so its close time is 0.
let ingestEnd = null;

const kind = system.startsWith("vectra") ? "vectra" : "warm";
const WARM_CONFIG = { embeddingModel: MODEL.model, embeddingRevision: MODEL.revision, embeddingDimensions: DIM };
const metadataFor = (system, id, text) =>
  system === "vectra-bm25" ? { documentId: id, startPos: 0, endPos: Math.max(0, text.length - 1) } : {};

async function buildVectra(corpus, vectors) {
  const { LocalIndex } = await import("vectra");
  const index = new LocalIndex(store);
  await index.createIndex({ version: 1, deleteIfExists: true });
  const items = corpus.ids.map((id, i) => ({
    id,
    vector: Array.from(vectors.subarray(i * DIM, (i + 1) * DIM)),
    metadata: metadataFor(system, id, corpus.texts[i]),
  }));
  await index.batchInsertItems(items);
}

async function buildWarmVector(corpus, vectors) {
  const { VectraMigrationIndex } = warmModule("vectra");
  const index = await VectraMigrationIndex.create({ path: store, embeddingSpace: MODEL });
  for (let from = 0; from < corpus.ids.length; from += 1000) {
    const operations = [];
    for (let i = from; i < Math.min(from + 1000, corpus.ids.length); i++) {
      operations.push({ op: "insert", record: { id: corpus.ids[i], vector: vectors.subarray(i * DIM, (i + 1) * DIM), metadata: {} } });
    }
    await index.applyBatch(operations);
  }
  ingestEnd = ns();
  await index.flush();
}

async function buildWarmDocs(corpus, vectors) {
  const { WarmMemorySystem } = warmModule("");
  const memory = new WarmMemorySystem(DIM, store, WARM_CONFIG);
  for (let from = 0; from < corpus.ids.length; from += 500) {
    const inputs = [];
    for (let i = from; i < Math.min(from + 500, corpus.ids.length); i++) {
      inputs.push({
        id: corpus.ids[i],
        text: corpus.texts[i],
        chunkWords: 4096,
        overlapWords: 0,
        storeText: false,
        embeddings: [Array.from(vectors.subarray(i * DIM, (i + 1) * DIM))],
      });
    }
    await memory.memorizeDocuments(inputs);
  }
  ingestEnd = ns();
  if (typeof memory.flush === "function") await memory.flush();
}

async function build() {
  const corpus = json("corpus.json");
  const vectors = f32(join(dataDir, "docs.f32"));
  if (vectors.length !== corpus.ids.length * DIM) throw new Error("docs.f32 does not match corpus.json");
  const t0 = ns();
  if (kind === "vectra") await buildVectra(corpus, vectors);
  else if (system === "warm-vector") await buildWarmVector(corpus, vectors);
  else await buildWarmDocs(corpus, vectors);
  const buildSeconds = secs(t0);
  const ingestSeconds = ingestEnd === null ? buildSeconds : Number(ingestEnd - t0) / 1e9;
  writeFileSync(outPath, JSON.stringify({ system, documents: corpus.ids.length, build_seconds: buildSeconds, ingest_s: ingestSeconds, close_s: buildSeconds - ingestSeconds, peak_rss_bytes: rss() }));
  process.exit(0);
}

function summary(samples) {
  const s = [...samples].sort((a, b) => a - b);
  const at = (q) => s[Math.floor((s.length - 1) * q)];
  return { queries: s.length, total_ns: s.reduce((a, b) => a + b, 0), p50_ns: s[(s.length - 1) >> 1], p90_ns: at(0.9), p99_ns: at(0.99), max_ns: s[s.length - 1] };
}

async function openSearcher(corpusTexts) {
  if (kind === "vectra") {
    const { LocalIndex } = await import("vectra");
    const options = system === "vectra-bm25" ? { docReader: async (id) => corpusTexts.get(id) } : undefined;
    const index = new LocalIndex(store, "index.json", undefined, undefined, options);
    await index.queryItems(new Float32Array(DIM).fill(0.01), "", 1); // loads the index file; counted in cold start
    const bm25 = system === "vectra-bm25";
    return async (vector, text) => (await index.queryItems(vector, text, TOP_K, undefined, bm25)).map((r) => r.item.id);
  }
  if (system === "warm-vector") {
    const { VectraMigrationIndex } = warmModule("vectra");
    const index = await VectraMigrationIndex.open({ path: store });
    return async (vector) => (await index.queryItems(vector, { k: TOP_K, mode: "exact" })).results.map((r) => r.item.id);
  }
  const { WarmMemorySystem } = warmModule("");
  const memory = new WarmMemorySystem(DIM, store, WARM_CONFIG);
  if (system === "warm-lexical") {
    return async (_vector, text) => (await memory.searchDocuments(text, { k: TOP_K, candidateLimit: TOP_K, lexicalWeight: 1, semanticWeight: 0 })).map((r) => r.documentId);
  }
  return async (vector, text) =>
    (await memory.searchDocuments(text, { k: TOP_K, candidateLimit: 100, embedding: Array.from(vector), lexicalWeight: 1, semanticWeight: 1 })).map((r) => r.documentId);
}

async function serve() {
  const queries = json("queries.json");
  const vectors = f32(join(dataDir, "queries.f32"));
  const total = queries.ids.length;
  if (vectors.length !== total * DIM) throw new Error("queries.f32 does not match queries.json");
  const count = Math.min(total, Number(maxQueriesArg) || total);
  let texts = null;
  if (system === "vectra-bm25") {
    const corpus = json("corpus.json");
    texts = new Map(corpus.ids.map((id, i) => [id, corpus.texts[i]]));
  }
  const vec = (i) => vectors.subarray(i * DIM, (i + 1) * DIM);
  const t0 = ns();
  const search = await openSearcher(texts);
  const openSeconds = secs(t0);
  const first = ns();
  await search(vec(0), queries.texts[0]);
  const firstQueryMs = Number(ns() - first) / 1e6;
  const coldStartSeconds = process.uptime();
  const passes = [];
  let ids = null;
  let stable = true;
  for (let pass = 1 - WARMUP; pass <= PASSES; pass++) {
    const samples = [];
    const lists = [];
    for (let i = 0; i < count; i++) {
      const start = ns();
      const found = await search(vec(i), queries.texts[i]);
      samples.push(Number(ns() - start));
      lists.push(found);
    }
    if (pass === 0) continue; // untimed warm-up pass (pass 0 exists only when WARMUP is 1)
    passes.push(summary(samples));
    if (ids === null) ids = lists;
    else if (JSON.stringify(ids) !== JSON.stringify(lists)) stable = false;
  }
  writeFileSync(outPath, JSON.stringify({
    system, queries: count, open_seconds: openSeconds, first_query_ms: firstQueryMs, cold_start_seconds: coldStartSeconds,
    passes, stable, peak_rss_bytes: rss(), ids: queries.ids.slice(0, count).map((id, i) => [id, ids[i]]),
  }));
  process.exit(0);
}

await (phase === "build" ? build() : serve());
