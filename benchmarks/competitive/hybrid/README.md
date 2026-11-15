# Hybrid contract

The lexical contracts of `warm-bench` compare one algorithm, BM25, implemented by every engine, so quality can only tie.
WARM's product is hybrid search: stemmed BM25 fused with exact-cosine vector scoring from a local embedding model. This
contract compares that, and only that, against other in-process hybrid stacks. Results are from one machine (the
machine class is recorded in every results file) and are not official or leaderboard numbers.

## Contract

**Input** is the raw query string. Every system embeds the query with the same local model inside the timed region, runs
its own lexical retrieval, its own vector retrieval and its own fusion, and returns the top 100 `(id, score)` pairs.
Documents are embedded once at build with WARM's pure-Rust embedder (`granite-embedding-30m-english`, Apache-2.0, int8
pack, 384 dimensions); every system indexes the identical document vectors from one shared file, so build times exclude
embedding and the shared embedding cost is reported once per corpus.

Query embedding. No comparator ships this model, so every comparator obtains the query vector from one persistent
`competitive_hybrid embed-serve` helper process (length-prefixed request and reply over pipes, one embedder thread,
inheriting the worker's pinned core). WARM embeds in its own process through its installed embedder. The helper adds
one pipe round trip per query to every comparator and none to WARM, so the including-embedding comparison favours
WARM by that constant (measured and reported as `embed_ns`); the excluding-embedding comparison does not.
Because the embedding of one query costs several milliseconds on one core, including-embedding throughput is dominated by
the shared model and is expected to land near parity for any engine whose retrieval is faster than the embedder; the
engine comparison lives in the excluding-embedding numbers.

**Quality** is nDCG@10 and recall@100 of the returned lists, per query, compared by the paired per-query bootstrap
(`common.paired_bootstrap`). The vectors used for quality are the shared query vectors; the including-embedding pass
must return the identical id lists (checked and recorded).

**Speed** is single-thread: one query at a time, pinned core, embedder threads 1. Per round each system runs
one pass of up to 50 queries from the raw string (embedding in the timer; `including`) and two timed passes over up to
200 queries with the query vector supplied (`excluding`). QPS, p50 and p99 are per-round medians; intervals are
bootstrap intervals over 10 interleaved rounds, Bonferroni-adjusted over all speed comparisons. The foreign-CPU gate
(`hybrid_run.py`) measures busy CPU of the pinned core not attributable to the worker and its helper during every serve
phase and repeats the phase (up to 3 attempts, all recorded) above 5 percent of the pinned core.
Build time is a cold process per repeat, index size is allocated bytes of the index directory (the shared vector file is
excluded for every system), peak RSS is the wait4 rusage of the serve process: WARM's includes its embedder, a comparator's excludes the helper process that holds the embedder, which is conservative against WARM.

**Corpora** (FiQA is subsampled to 15,000 documents and SCIDOCS to 10,000 to fit the single-VM budget of embedding on 16 vCPUs at about 7 documents per second: every document judged relevant to an evaluation or sampled development query is kept, the rest is a seeded random fill, so these two sets are easier than the full corpora and not comparable with the lexical contract's FiQA row): `beir:fiqa`, `beir:scifact`, `beir:nfcorpus`, `memory` (reported splits as the lexical contract), plus
`beir:arguana` and `beir:scidocs`, whose only public qrels are the test split. Their test qrels are read once, at the
evaluation run, after the fusion choice is frozen; no setting was selected on them in this campaign (the lexical campaign
used both as development data, which is disclosed in the PR).

**Fusion and development data**. WARM's shipped fusion is `blend`: min-max normalised scores of each list's top
`candidateLimit` chunks, weighted 1:1 (a convex combination), with `rrf` (k = 60) selectable. The ablation of RRF constants
against convex combinations (min-max and theoretical-minimum normalisation, weights, candidate depth) is chosen on
development data only (FiQA train queries, NFCorpus train queries, the 100 `memory-selection` questions) by the rule in
`fusion_rule.json`, which is written before any evaluation run; the chosen setting is frozen in `fusion_frozen.json`.
Each comparator keeps its documented default (RRF with the constant its documentation uses, 60) and gets a second row
with the RRF constant tuned on the same development data. Comparators are never tuned down.

**Systems** (versions recorded in the results): Apache Lucene 10.6.0 BM25 plus `KnnFloatVectorQuery` (HNSW) fused with
`TopDocs.rrf`, and an exact (brute-force vector scan) variant; SQLite FTS5 plus sqlite-vec fused in one SQL statement
with RRF as the sqlite-vec documentation does; LanceDB embedded hybrid search with its full-text index and default
`RRFReranker`; tantivy BM25 plus a usearch HNSW index with application-side RRF. Every lexical leg reads the lowercase
alphanumeric query word set like the lexical contract.

## WARM's hybrid path

WARM scores the semantic list exactly: every ranked chunk carries `Embedding::cosine`, the sequential `f64` dot product.
Scanning every chunk with that reduction cost 2 to 14 ms per query on these corpora. Unfiltered hybrid queries now read a
quantized table of the store's embeddings first (`src/core/documents/screen.rs`): two `i8` planes per chunk and a 15-bit
query code give each chunk an interval that provably contains its exact cosine (the row and query quantization residuals
are measured when the table is built and when the query arrives, and Cauchy-Schwarz bounds their products). Only chunks
whose interval can reach the `candidateLimit`-th best lower bound, or the `minSemanticScore` floor, are scored exactly,
so the returned scores, ranking and ties are bit-identical to the exact scan; the table changes which chunks are read,
never a score. The table is built for a store generation by its second unfiltered hybrid query and only above 2,048
chunks; below that, and for filtered queries, an `f32` screen with the same guarantee (above 256 candidate chunks) or the
plain exact scan runs. The table is a second copy of the embeddings at 2 bytes per value; it is dropped when the store
changes and rebuilt on demand. Lexical-only and vector-only queries are unchanged.

### Measured miss

On ArguAna the engine change leaves WARM at 0.98 [0.96, 0.99] of the best comparator (lucene-hnsw tuned) on speed excluding embedding. Long queries spend about 1.9 ms in the lexical stage against about 0.55 ms in the semantic scan. Every other corpus is above 1.0 with its interval above 1.0. Results are from one VM and are not official.
