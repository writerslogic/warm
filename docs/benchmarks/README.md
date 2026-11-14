# Comparative benchmark (warm-bench)

Start with [recorded results and limitations](EVALUATION.md) for a summary across memory, text
and vector workloads, including the conditions and known losses.

The modern embedding benchmark and upstream contribution work is tracked in
[VIBE](VIBE.md), single-core results on the ann-benchmarks runner in
[ANN-BENCHMARKS](ANN-BENCHMARKS.md), the big-ann-benchmarks streaming track (inserts,
deletes and searches under an 8 GiB cap) in [BIG-ANN-STREAMING](BIG-ANN-STREAMING.md),
and conversational memory QA against the published Mem0 and Zep LoCoMo results in
[LOCOMO](LOCOMO.md). Each is separate from the SIFT/GloVe comparison below.

One command reproduces the WARM comparison against the vector, text and embedded-search systems
people actually use. The generated results are in [COMPARISON.md](COMPARISON.md); the rule that
governs everything below is that **every number in that file traces to a committed results file
under `benchmarks/results/competitive/`, and a comparator that beats WARM is reported as beating
WARM.**

Historical source snapshots and large raw output caches are indexed in the
[evidence archive guide](../../benchmarks/evidence/README.md). Restore their pinned
bytes before replaying an older experiment. The retained summaries describe their
recorded revisions; no benchmark was rerun as part of source cleanup.

```sh
./benchmarks/competitive/warm-bench list                    # systems, suites, what was not run
./benchmarks/competitive/warm-bench run --suite light       # the hosted-runner subset (about an hour)
./benchmarks/competitive/warm-bench run --suite standard    # a local workstation
./benchmarks/competitive/warm-bench table                   # regenerate COMPARISON.md from committed results
./benchmarks/competitive/warm-bench table --check           # fail if the committed table is stale
uv run --no-project --python 3.13 --with pytest --with numpy==2.5.4 pytest benchmarks/competitive/tests
```

`warm-bench run` builds nothing itself: it needs `public-bench` and the `competitive_text` example
(`cargo build --profile dist --features cli,mimalloc --bin public-bench --example competitive_text`, with
`CARGO_TARGET_DIR` if you use a private target directory; `dist` is the release build without the
symbol table, the binary the install-size row measures), `uv`, a JDK 21 or newer for the Lucene row,
and network access for the first run (datasets, pinned wheels, two Lucene jars with pinned sha256).

## What is compared

| Family | Systems (version pinned in `benchmarks/competitive/requirements/*.txt`) |
|---|---|
| Vector / ANN | hnswlib (Python binding, and its own C++ `searchKnn` from a native driver on the same index file), FAISS (HNSW, IVF-PQ with and without exact refine, Flat), USearch (f32, int8), Annoy, sqlite-vec, LanceDB (IVF-PQ, IVF-HNSW-SQ), Chroma, Qdrant embedded local mode, Milvus Lite (HNSW, IVF-Flat), DuckDB vss, NGT (ONNG), PyNNDescent, ScaNN and DiskANN (Linux only), Voyager |
| Text (BM25) | SQLite FTS5, tantivy, Apache Lucene 10.6.0 (JVM), Whoosh, bm25s |
| Agent-memory systems | Mem0, Zep, Letta, LangMem: not run here; see "Plugging in more systems" |
| Not run, with reasons | pgvector (needs a PostgreSQL server; not installable from pip/cargo/npm) |

WARM is measured through its own binaries: `public-bench ann-qgraph` (the vertex graph with the
accepted configuration recorded in `benchmarks/results/qgraph_pool_paired_x86_glove_v4.json`,
tuned on a train holdout, never on the benchmark queries) and `examples/competitive_text.rs`
(the document API, lexical only, one chunk per document).

## Datasets

* **glove-100-angular** (1.18M x 100, ann-benchmarks): `light` uses the first 100,000 vectors and
  1,000 queries, `standard` 250,000 and 2,000, `heavy` all 1,183,514 and 10,000. For every subset
  the exact top-100 truth is recomputed (`warmbench/data.py`); the files WARM and every comparator
  read are byte-identical, and their sha256 are recorded.
* **sift-128-euclidean** (1M x 128, ann-benchmarks): `standard` uses the first 250,000 vectors
  and 2,000 queries, `heavy` all 1,000,000 and 10,000. WARM runs `public-bench ann-qgraph
  --metric euclidean` (vertex index on the vectors as given, Euclidean truth).
* **BEIR SciFact (train qrels) and NFCorpus (dev qrels)**, the existing document-competition
  protocol: lexical only, top 100, one chunk per document, test qrels never read.
* **BEIR FiQA (dev qrels)**, `standard` and `heavy` only: 57,600 documents (the 38 empty ones
  are left out for every system), 500 queries, same protocol.
* **memory** (agent-memory-shaped): all sessions of the 100 LongMemEval_S dev questions as one
  searchable memory (about 5,000 sessions, truncated to 4,000 words), relevant documents are the
  answer sessions. The 400 held-out questions are never read.

## Protocol

* Single client thread, one query at a time, per-query `perf_counter_ns` (or the Rust/JVM
  equivalent) around the search call. Builds may use `threads` cores (4 light/standard, 8 heavy)
  for every system that can; WARM builds with the same count.
* Every system runs each phase (build, serve, ops) in a fresh process; the orchestrator reads the
  child's peak RSS from `wait4`. Cold start is the wall time from spawning the serving process to
  the first answered query (interpreter, imports, open, first query; the file cache is warm).
* Serve rounds are interleaved: each round runs WARM and every comparator once, in rotated order,
  so a load spike lands on all systems. Each phase takes a timing lock while it runs, never across a whole suite; the lock is a
  no-op on CI. Load averages are recorded per configuration; a load gate is not enforced on
  shared machines (`load_gate_met` is `null`, not `true`).
* **Matched recall.** Each comparator sweeps its documented good settings (HNSW M 16 and 32 with
  efConstruction 200 and ef 10..1024; IVF nprobe 1..256; Annoy 100 trees with search_k up to
  51,200; and so on, see `ann_worker.py`). For each target recall@10 (0.90, 0.95, 0.99) the row
  with recall at or above the target and the highest pooled median QPS over all rounds is chosen;
  that row's per-round values carry the confidence interval. A system that never reaches the
  target is reported as "not reached" with its best recall, never dropped. The queries are split
  by index: even-indexed queries select the operating point, odd-indexed queries report its recall and
  QPS, so no point is scored on the queries that picked it; a point whose report-half recall falls below
  the target is reported as not reached.
* **Per-build comparison.** A comparator has several builds; build time, serving memory, index size
  and QPS belong to one build. The generated table never takes each metric from a different build:
  the verdict table draws no verdict on build-dependent rows, and a per-build table gives, for every
  comparator build, its own build time, memory, size and QPS at each recall target, with WARM's factor
  against that build alone.
  Comparators are never tuned below their documented defaults.
* **Rounds.** The standard and heavy suites run 10 independent process rounds by default
  (`--rounds N` changes it). With fewer than 8 rounds a bootstrap has too few distinct resamples, so the
  interval printed is the min-max range of the rounds and the section says so. `--pin-cpus LIST` runs the
  single-threaded serve and ops phases under `taskset -c LIST` (Linux) and the pinning is recorded in the
  results.
* **Raw data.** Every results file has `raw_latency`: the name and sha256 of `<result>.latency.npz`, which holds
  the per-query nanoseconds of every timed pass (and the per-query hit counts of every ANN configuration), so
  every interval can be recomputed. Dataset digests (`train_sha256`, `test_sha256`; `input_sha256`,
  `qrels_sha256` for text) are recorded and a run with a missing digest refuses to start.
* **Intervals and verdicts.** 95% bootstrap intervals over rounds (or queries, for quality),
  Bonferroni-adjusted over the family of verdict metrics of the file ("family guard", as in the
  superiority ledger). The advantage factor is oriented so that above 1 means WARM is better; WIN or
  LOSS needs the interval of the factor to exclude 1; deterministic byte counts use a 5% band;
  everything else is TIE. `index size` and `install distributions` are printed without a verdict
  because the two sides measure different things (WARM reports its in-memory index, comparators the
  persisted directory; Python wheels versus statically linked crates).
* **Durability rows.** A replace/delete row compares only systems with the same commit contract; the table
  prints each system's contract (`registry.COMMIT_CONTRACT`). A library whose adapter must save and fsync the
  whole index per operation (`full-index-rewrite`, e.g. hnswlib) has no incremental durable commit, so no row is
  generated against it; a comparator such as sqlite-vec, which commits one transaction per operation, can have one.
  Replace/delete cost is one durable commit per operation on a persisted store
  (on macOS every system's commit ends in `fcntl(F_FULLFSYNC)`, which flushes the drive cache:
  WARM after each journal frame, tantivy through Rust's `sync_all`/`sync_data`, Lucene through the
  JDK's `FileChannel.force`, and SQLite because it runs with `fullfsync`; libraries that need an
  explicit save to be durable include that save in the figure).
* **Hardware classes.** Each file records its class (`hardware.class`) and the table has one
  section per file; rows from different classes are never put side by side. Hosted-runner rows are
  shared virtual machines: read the ratios, not the absolute numbers.

## Evidence validity

Every generated cell carries `validity` (`valid`, `unverified` or `invalid`) and an uncertainty
kind (`sampled`, `unreplicated` or `deterministic`) in `comparison.cells.json`. A source revision
that is not a full commit identifier cannot support a comparative verdict. Explicit historical
measurement invalidations retain raw observations but remove the affected metric from scoring.
The old workstation text RSS rows inherited the orchestrator's high-water mark; they are invalid,
not evidence of equal process residency. New runs use the isolated measurement spawner.

Unreplicated timings remain descriptive and receive `n/v`, even when their ratio exceeds 5%.
The deterministic 5% band applies only to explicitly deterministic byte counts. Text timing
passes are summarized within their process-round before bootstrapping independent process-round
summaries; adding passes does not create more independent observations. Sweep selection and
confirmation are distinct: existing sweep-selected operating points remain exploratory evidence;
freeze settings before a new independent confirmation campaign. These corrections reinterpret
existing bytes and do not claim a fresh performance measurement.

## Known measurement boundaries (read before quoting a ratio)

* WARM is timed through its Rust API; Python comparators through their bindings; Lucene through the
  JVM after untimed warm-up passes to steady state (query analysis, document-id lookup and JVM flags: `comparator_settings` in each results file). Each section prints the boundary per system. The existing
  accepted serving rows (`benchmarks/results/serving_memory_v40.json`) drive both arms from Node,
  which narrows WARM's lead against SQLite; the two protocols are not interchangeable.
* Comparator query preparation (word splitting, SQL expression building, Lucene analysis, tantivy
  parsing) is done outside the timer to favour the comparators; WARM's own parsing stays inside it.
* tantivy is two rows: `tantivy-py` (the CPython binding, ids fetched through Python objects) and
  `tantivy-rs` (the native crate in its own process, ids from a fast field, built by
  `examples/competitive_text_tantivy.rs`, the same tantivy version as the binding). The binding row is
  never a stand-in for the crate.
* LanceDB, Chroma and Milvus Lite run Rust or C++ runtimes with their own threads; QPS is still one
  client at a time.
* Qdrant's embedded local mode is an exact scan (its documentation points to a server for indexed
  search); it is included because it is the only Qdrant that needs no service.
* WARM builds its ANN index in a build process that saves it (`public-bench ann-qgraph
  --save-index`, the vertex index file of `VGraph::save`); each serve round is a fresh process that
  opens that file (`--load-index`) memory-mapped and prefaulted before the timer, as every comparator
  opens its persisted index in a fresh serve process. Build time and build peak RSS come from the
  build processes, serving RSS from the serve processes; the index's file-backed pages count in RSS.
  The serving graph is static, so ANN replace and delete are timed in a separate `public-bench ann-updates` session on the updatable index (one durable commit each); the table prints those rows without a verdict ("n/v").
* Above 200k vectors the slow exact-scan systems answer a prefix of the query file
  (`protocol.query_caps`); recall and QPS are over that prefix.

## Updating the table

`.github/workflows/bench-refresh.yml` validates the generator on relevant pull requests,
main-branch changes and manual dispatch. Performance comparisons run locally; scheduled remote
benchmark execution and automatic result publication are disabled. Use the local commands above,
retain the raw results with their hardware class, then run
`./benchmarks/competitive/warm-bench table` and submit the generated table and source results for review.

## Heavy suite

Full GloVe-100 and SIFT-1M do not fit a hosted runner. Two supported ways to run them:

* Self-hosted or workstation: `./benchmarks/competitive/warm-bench run --suite heavy --only ann`.
  Set `WARM_BENCH_HW_CLASS` if the derived class is not descriptive. Commit the resulting `heavy-*.json` files.
* Modal: `uv run --with modal modal run --detach benchmarks/competitive/modal_run.py --suite heavy --cap-usd 12`
  ships the working tree to one 8 vCPU / 32 GiB container, builds WARM there, runs the suite and
  writes the results back with hardware class `modal-x86_64-8vcpu-32g`. The cap kills the run before
  the published rate would exceed it.

## Multilingual lexical (MIRACL dev)

`benchmarks/results/multilingual_bm25_miracl_dev.json` compares WARM BM25 builds with Lucene
10.6.0 analyzers on MIRACL v1.0 dev (hi, fa, th, sw). Each language's corpus is every judged
passage plus fixed-seed uniform distractors up to 100,000 passages, identical for every system,
so absolute scores sit above full-corpus MIRACL figures. Quality only; nothing is timed. The
Lucene comparator needs JDK 21 or newer.

```sh
PATH=/opt/homebrew/opt/openjdk@25/bin:$PATH uv run --no-project python -I \
  benchmarks/competitive/warmbench/multilingual.py --raw DIR --langs hi fa th sw \
  --warm before=BIN after=BIN --commits before=SHA after=SHA --lucene standard language \
  --extra beir-scifact-train=CACHE/text-scifact-train --out benchmarks/results/multilingual_bm25_miracl_dev.json
```

`DIR` holds `topics-<l>.tsv` and `qrels-<l>.tsv` from `miracl/miracl` and
`corpus-<l>-docs-<n>.jsonl.gz` from `miracl/miracl-corpus`; `BIN` is the `competitive_text`
example built at `SHA`. nDCG@10:

| | WARM `a090f971` | WARM joined `63d9d3d8` | WARM NFC `5e9e3b16` | Lucene Standard | Lucene language |
|---|---|---|---|---|---|
| hi | 0.4599 | 0.4474 | 0.4598 | 0.4470 | 0.5279 (Hindi) |
| fa | 0.4851 | 0.4325 | 0.4851 | 0.4313 | 0.5196 (Persian) |
| th | 0.3746 | 0.2678 | 0.3746 | 0.2694 | 0.5439 (Thai) |
| sw | 0.4444 | 0.4445 | 0.4444 | 0.4473 | none |

Keeping combining marks and ZWNJ inside words (the joined build, branch
`lever/unicode-tokenizer-joined`) lands within 0.2 points of StandardAnalyzer and loses to
the split WARM has always done: without a stemmer the word fragments act as subwords. It is
not merged. The gap that remains is
Lucene's per-language stemming, Persian normalization and Thai dictionary segmentation.
The file also checks the English sets: on SciFact train, NFCorpus dev and LongMemEval dev, the
NFC build's per-query nDCG@10 and recall@100 match `a090f971` exactly.

### Store analyzers

A store's lexical analyzer is chosen when it is created (`WarmConfig::analyzer`,
`competitive_text --analyzer NAME`) and recorded in its `store.json` (`lexical_analyzer`);
a store without the key, every store written before analyzers and every default store, keeps
the legacy analysis. `benchmarks/results/multilingual_bm25_analyzers_miracl_dev.json` runs
WARM `a090f971` (before), WARM `515765ce` with the default analyzer (new) and with the
language's analyzer (`--analyzed lang`: hindi, persian, bigram for Thai, arabic, finnish,
russian, spanish, french, german)
against Lucene 10.6.0's language analyzers, same corpora as above. nDCG@10:

| | WARM `a090f971` | WARM `515765ce` default | WARM `515765ce` language analyzer | Lucene language |
|---|---|---|---|---|
| hi | 0.4599 | 0.4599 | 0.5319 (hindi) | 0.5279 (Hindi) |
| fa | 0.4851 | 0.4851 | 0.5026 (persian) | 0.5196 (Persian) |
| th | 0.3746 | 0.3746 | 0.5564 (bigram) | 0.5439 (Thai) |
| ar | 0.5556 | 0.5556 | 0.6109 (arabic) | 0.6074 (Arabic) |
| fi | 0.5872 | 0.5872 | 0.6583 (finnish) | 0.6640 (Finnish) |
| ru | 0.4134 | 0.4134 | 0.5090 (russian) | 0.5171 (Russian) |
| es | 0.5980 | 0.5980 | 0.6295 (spanish) | 0.6246 (Spanish) |
| fr | 0.4231 | 0.4231 | 0.4324 (french) | 0.4329 (French) |
| de | 0.4278 | 0.4278 | 0.4532 (german) | 0.4732 (German) |

The file's `summary` gives the share of the mean gap to Lucene's language analyzers that the
WARM analyzers close over these nine languages: 0.955 (mean gap 0.0651 before, 0.0029 after).
The mean nets languages where WARM is ahead (hi, th, ar, es) against those where it is still
behind (de by 0.020, fa by 0.017, ru by 0.008, fi by 0.006, fr by 0.0005). The default analyzer's top 100 ids and scores match `a090f971` on
every query of every set, including SciFact train, NFCorpus dev and LongMemEval dev. The
`english` analyzer (NFC, Turkish I folding, ASCII digits) leaves those three sets' nDCG@10
unchanged; its top 100 ids and scores are identical on 809 of 809 SciFact, 42 of 324 NFCorpus
and 0 of 100 LongMemEval queries. WARM has no stop words; Lucene's analyzers remove them. The analyzers, their file-format marks and
the binding follow-ups are in [file-format/LEXICAL.md](../file-format/LEXICAL.md#analyzers).

```sh
PATH=/opt/homebrew/opt/openjdk@25/bin:$PATH uv run --no-project python -I \
  benchmarks/competitive/warmbench/multilingual.py --raw DIR --langs hi fa th ar fi ru es fr de \
  --warm before=BIN_BEFORE new=BIN lang=BIN --analyzed lang --lucene language --reuse \
  --extra beir-scifact-train=CACHE/text-scifact-train ... \
  --out benchmarks/results/multilingual_bm25_analyzers_miracl_dev.json
```

`benchmarks/results/analyzer_query_latency_scifact.json` is a paired query-latency check of the
default analyzer against `a090f971` on SciFact (`warmbench/analyzer_latency.py`, rounds
alternate the binaries on a loaded host, so only the ratio is reported).

## Server mode

Server systems are compared with `warm-server` (feature `server`), never with the embedded rows;
their files carry `mode: "server"` and render under "Server mode results".

| Kind | Systems | Source |
|---|---|---|
| ANN | `warm-server`, Qdrant, Weaviate, OpenSearch k-NN, PostgreSQL + pgvector (HNSW, IVFFlat), Redis 8 query engine (HNSW, FLAT) | `warmbench/servers.py` |
| Text | OpenSearch BM25, Meilisearch, Typesense, PostgreSQL tsvector/ts_rank, ParadeDB pg_search | `warmbench/servers.py` |

```sh
./benchmarks/competitive/warm-bench install-servers            # downloads, sha256/sha512 checks, client env
./benchmarks/competitive/warm-bench run --suite smoke --mode server
```

* Downloaded servers are pinned per platform with a digest; PostgreSQL's pgvector (macOS: Homebrew
  keg; Linux: PGDG apt) and Redis 8 with its query engine module (Homebrew; packages.redis.io) come
  from the package manager and are recorded by reported version. PostgreSQL itself on macOS is the
  pinned zonky build; ParadeDB's package is unpacked, never installed, and loaded through
  `extension_control_path`. OpenSearch has no macOS build. Milvus standalone is not run: its
  current releases ship only Docker Compose files.
* `warmbench/server_proc.py` owns each server: stable loopback ports per data directory, its own
  session, bounded readiness probe, log in the work directory, SIGTERM then SIGKILL of the whole
  process group, sampled RSS of the group (PSS on Linux).
* The protocol (one client, one persistent connection, sequential queries, client-side timing;
  build until queryable; cold start from process start to first answer; durable replace/delete
  semantics per system) is written into every results file under `protocol`.
* `warm-server`'s text row ingests through the memory API (`remember`) and queries through
  `recall` with lexical weight 1 and semantic weight 0, the same document-search options as the
  embedded `warm` row. Its ANN graph is persisted with the tenant store and opened at start; the
  harness still sends index maintenance after every restart (a no-op when nothing changed), so
  cold start covers that check.

### Server mode results (`benchmarks/results/server_v2.json`)

Paired run on one Apple M4 (macOS, Darwin arm64): the `before` arm is `warm-server` at `791749b`
(graph rebuilt by maintenance after every restart) with every comparator, the `after` arm is
`warm-server` at `feae232` (persisted graph opened at start) alone, set against the comparator rows
of the before arm. Comparators run the build matching WARM's build effort (m32e500 variants); one
build, three interleaved serve rounds; verdicts are ratios of round medians with a 5% band, without
bootstrap intervals. The host carried other work during the before arm (load recorded per round in
the file); Qdrant's second-round cold start (8.9 s) is kept as measured. Server data directories
lived on the internal SSD. Replace and delete are one durable request each (`protocol.durability`).

Machine: Apple M4, Darwin arm64. Load (1 min) during the ANN serve rounds: before arm 6.0-8.6, after arm 3.0-3.5.

ANN (glove-100-angular, 20,000 vectors, 600 queries, k=10; QPS and p50 at the sweep point with recall@10 >= 0.95; medians over 3 serve rounds):

| System | Arm | Build s | Cold start s (rounds) | QPS @0.95 | p50 us | Recall | Replace ms/op | Delete ms/op | Serving RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| warm-server (HTTP) (graph) | before | 2.0 | 1.32 (1.11, 1.48, 1.32) | 6826 | 128 | 0.955 | 3.20 | 9.22 | 52 |
| Qdrant server (m32e500) | comparator | 21.7 | 0.35 (0.35, 8.89, 0.13) | 4445 | 206 | 0.964 | 1.41 | 1.25 | 103 |
| Weaviate (m32e500) | comparator | 15.7 | 0.78 (0.85, 0.62, 0.78) | 1049 | 867 | 0.986 | 13.30 | 0.10 | 169 |
| PostgreSQL + pgvector (hnsw_m32e500) | comparator | 12.9 | 0.15 (0.13, 0.15, 0.30) | 1382 | 686 | 0.950 | 17.82 | 4.10 | 27 |
| Redis 8 query engine (hnsw_m32e500) | comparator | 7.5 | 3.47 (3.47, 3.67, 2.94) | 4654 | 200 | 0.975 | 4.93 | 4.33 | 47 |
| warm-server (HTTP) (graph) | after | 2.4 | 0.07 (0.34, 0.07, 0.07) | 7762 | 128 | 0.954 | 3.83 | 3.95 | 50 |

Text (BEIR nfcorpus, 3,633 documents, 323 queries, k=100, 3 passes per round; medians over 3 rounds):

| System | Arm | Ingest s | Cold start s (rounds) | QPS | p50 us | nDCG@10 | Replace ms/op | Delete ms/op |
|---|---|---|---|---|---|---|---|---|
| warm-server (HTTP) | before | 1.45 | 0.13 (0.20, 0.13, 0.12) | 3300 | 380 | 0.292 | 6.04 | 9.10 |
| Meilisearch | comparator | 1.80 | 0.33 (0.43, 0.23, 0.33) | 956 | 850 | 0.226 | 60.52 | 261.20 |
| Typesense | comparator | 2.60 | 3.27 (3.28, 3.25, 3.27) | 166 | 3845 | 0.208 | 1.40 | 0.57 |
| PostgreSQL tsvector (ts_rank) | comparator | 1.01 | 0.28 (1.15, 0.18, 0.28) | 1500 | 288 | 0.275 | 3.83 | 3.98 |
| ParadeDB pg_search (BM25) | comparator | 2.56 | 0.44 (0.44, 0.32, 0.52) | 2616 | 237 | 0.290 | 3.20 | 3.99 |
| warm-server (HTTP) | after | 1.32 | 0.13 (0.16, 0.13, 0.12) | 3412 | 372 | 0.292 | 5.97 | 9.16 |

Verdicts, warm-server against each comparator (ratio of medians, 5% band), before -> after:

- qdrant-server:m32e500: cold_start LOSS 3.80x -> WIN 4.80x; build WIN 10.61x -> WIN 9.09x; qps_at_0.95 WIN 1.54x -> WIN 1.75x; p50_at_0.95 WIN 1.61x -> WIN 1.62x; replace LOSS 2.27x -> LOSS 2.71x; delete LOSS 7.38x -> LOSS 3.16x; serve_rss WIN 1.99x -> WIN 2.07x
- weaviate:m32e500: cold_start LOSS 1.69x -> WIN 10.77x; build WIN 7.68x -> WIN 6.58x; qps_at_0.95 WIN 6.50x -> WIN 7.40x; p50_at_0.95 WIN 6.78x -> WIN 6.79x; replace WIN 4.15x -> WIN 3.48x; delete LOSS 96.30x -> LOSS 41.25x; serve_rss WIN 3.29x -> WIN 3.42x
- pgvector:hnsw_m32e500: cold_start LOSS 9.09x -> WIN 2.01x; build WIN 6.31x -> WIN 5.40x; qps_at_0.95 WIN 4.94x -> WIN 5.61x; p50_at_0.95 WIN 5.37x -> WIN 5.38x; replace WIN 5.56x -> WIN 4.66x; delete LOSS 2.25x -> TIE 1.04x; serve_rss LOSS 1.93x -> LOSS 1.85x
- redis-vss:hnsw_m32e500: cold_start WIN 2.62x -> WIN 47.84x; build WIN 3.68x -> WIN 3.15x; qps_at_0.95 WIN 1.47x -> WIN 1.67x; p50_at_0.95 WIN 1.56x -> WIN 1.56x; replace WIN 1.54x -> WIN 1.29x; delete LOSS 2.13x -> WIN 1.10x; serve_rss LOSS 1.09x -> TIE 0.96x
- meilisearch: cold_start WIN 2.56x -> WIN 2.57x; ingest WIN 1.24x -> WIN 1.36x; qps WIN 3.45x -> WIN 3.57x; p50 WIN 2.23x -> WIN 2.29x; replace WIN 10.02x -> WIN 10.14x; delete WIN 28.70x -> WIN 28.52x
- typesense: cold_start WIN 25.02x -> WIN 25.11x; ingest WIN 1.79x -> WIN 1.97x; qps WIN 19.88x -> WIN 20.56x; p50 WIN 10.11x -> WIN 10.34x; replace LOSS 4.31x -> LOSS 4.26x; delete LOSS 15.97x -> LOSS 16.07x
- pg-tsvector: cold_start WIN 2.16x -> WIN 2.17x; ingest LOSS 1.44x -> LOSS 1.31x; qps WIN 2.20x -> WIN 2.27x; p50 LOSS 1.32x -> LOSS 1.29x; replace LOSS 1.58x -> LOSS 1.56x; delete LOSS 2.29x -> LOSS 2.30x
- paradedb: cold_start WIN 3.38x -> WIN 3.39x; ingest WIN 1.76x -> WIN 1.93x; qps WIN 1.26x -> WIN 1.30x; p50 LOSS 1.60x -> LOSS 1.57x; replace LOSS 1.89x -> LOSS 1.87x; delete LOSS 2.28x -> LOSS 2.30x

## Plugging in more systems

* **ANN or vector store:** add an `Adapter` subclass in `warmbench/ann_worker.py` (`build`, `open`,
  `configure`, `query`, optional `replace`/`delete`/`flush`), register it in `ann_worker.SYSTEMS`
  and `warmbench/registry.py`, add its pinned requirement to `warmbench/envs.py` (`PINS`) and run
  `./benchmarks/competitive/warm-bench lock`.
* **Text or memory retrieval:** add a class with `build(documents)`, `open()`, `prepare(text)`,
  `search(prepared)`, `replace(doc)`, `delete(doc)` in `warmbench/text_worker.py`. The `memory`
  dataset already has the agent-memory shape (sessions as documents, answer sessions as relevant),
  so Mem0, Zep, Letta and LangMem plug in as text systems with their own pinned environment; the
  stage that runs them is a separate piece of work and none of them is run here.
* Facts that are not measurements (service needed, encryption at rest) live in
  `registry.FACTS` with their basis; only WARM's carries a measured source.

## Files

| Path | Purpose |
|---|---|
| `benchmarks/competitive/warm-bench` | entry point |
| `benchmarks/competitive/warmbench/` | runner, workers, analysis, table generator |
| `benchmarks/competitive/lucene/LuceneBm25.java` | Lucene BM25 comparator |
| `benchmarks/competitive/requirements/` | pinned, locked comparator environments |
| `benchmarks/competitive/tests/` | generator and traceability tests |
| `benchmarks/results/competitive/*.json` | raw results (one file per suite, dataset and hardware class) |
| `docs/benchmarks/COMPARISON.md`, `GAPS.md`, `comparison.cells.json`, `benchmarks/competitive/README-block.md` | generated |
