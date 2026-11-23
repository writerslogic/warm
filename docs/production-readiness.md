# Production readiness

## Supported surfaces

Rust `WarmCore` and Node `WarmMemorySystem` expose persistent vector, document, and
structured knowledge operations. `writerslogic-warm/semantic` adds bounded ingestion, local
ONNX embeddings, hybrid retrieval, and optional local cross-encoder reranking. The published
Python wheel (`warm-python` 0.7.1) exposes vector indexes, phase vectors and the in-process `Warm`
engine (document search, `remember`, `recall`, `forget`, `explain`). The Rust MSRV is 1.89.

## Compatibility and migration

| Format | Segment geometry |
|---|---|
| 4 (new stores) | On Unix, 1 GiB logical segments start with 64 MiB of mapped file capacity and grow in 64 MiB increments before appending a frame. Frame packing, logical offsets, transaction encoding and the 50 MiB raw-frame limit match the 1 GiB layout. Other platforms keep full-size allocation before mapping. |
| 3 | Fixed 64 MiB segments |
| 1, 2 | Fixed 1 GiB segments |

- Growth (Unix) flushes the old mapping, extends and fully syncs the file, then remaps under the existing writer guard. Windows must close mapped views before extending a file, so its allocation path stays fixed. Writes crossing a capacity boundary on Unix pay the allocation cost; this does not establish an unconditional latency improvement.
- Formats 1-3 keep their recorded geometry through writes and compaction; opening does not upgrade their layout.
- Older binaries reject format 4 before changing persistent data (opening still updates the ownership-lock PID).
- Storage health reports the current layout version and mapped file capacity (not RSS).
- `warm-admin migrate` copies the source format; re-encoding original documents into a new destination creates format 4.

**Embedding schema 2** is unchanged. `store.json` binds a store to its dimensions, text encoder (`multiscale-words-v1` by default), signed dense projection (`signed-projection-v2`), similarity metric, meaning-memory mode, encryption mode, and optional embedding model/revision/dimensions/normalization/metric. A mismatch fails at construction, so configure the same embedding space and security settings on every reopen. Model identity rules: [embeddings](embeddings/README.md#effective-model-identity-and-migration).

**Existing stores without an embedding schema cannot be opened by this engine.** Old unsigned projections discarded information; a bit-level conversion cannot recover it. Keep the old store and its original application version, and re-encode the original source documents into a new path:

```sh
cargo run --release --features cli --bin warm-admin -- reencode sources.jsonl ./store-v2 --dimensions 16384
cargo run --release --features cli --bin warm-admin -- verify ./store-v2
```

Each UTF-8 JSONL line is a `DocumentInput`, for example:

```json
{"id":"guide","text":"Restore a verified backup.","sourceUri":"guide.md","version":"2","metadata":{"project":"alpha"}}
```

- The CLI streams bounded lines, builds a temporary store, verifies its frames, and publishes the destination by rename. An invalid line leaves the destination absent and the source intact; an existing destination is refused.
- It migrates source documents only; reingest graph relations, triples and rules through their public APIs from your source of truth.
- For externally generated embeddings use Rust `reencode_documents` with an `EmbeddingSpace` configuration or Node `DocumentMemory.ingest` into a new store. Changing models requires new embeddings, not relabeling old vectors.

The experimental `multiscale-capped-union-v1` encoder unions primitive vectors within each text feature level before applying the existing cross-level frequency cap. It needs the `experimental` feature and explicit selection (enabling the feature preserves the default encoder). To evaluate it on original source documents in a separate store:

```sh
cargo run --release --features experimental,cli --bin warm-admin -- reencode sources.jsonl ./union-store --dimensions 16384 --text-encoder multiscale-capped-union-v1
```

In Rust, set `WarmConfig.text_encoder = TextEncoder::MultiscaleCappedUnionV1` (import `TextEncoder` from `writerslogic_warm::core::encoding`). Reopen with the same encoder: the schema rejects a different one before loading arena data. Text encoding and document chunk encoding both use the configured version; lexical document search and supplied dense embeddings keep their scoring. Capped union remains a research option; synthetic gains do not establish document relevance or a speed improvement.

`warm-admin migrate SOURCE DESTINATION` makes a verified copy of an arena and its sidecars; it does **not** change text/vector encoding or upgrade an old embedding schema. Inspection and verification understand legacy frames and the active `CURRENT` generation:

```sh
cargo run --release --features cli --bin warm-admin -- inspect ./store-v2
cargo run --release --features cli --bin warm-admin -- verify ./store-v2
cargo run --release --features cli --bin warm-admin -- migrate ./store-v2 ./store-copy
```

## Durability and concurrency

### Writes and compaction

Every engine holds an exclusive `.hms.lock` for its lifetime: use one instance per store and share it across tasks; a second process or instance cannot open it.

- **Write path.** A persistent mutation validates and serializes its operations, reserves an exclusive writer lease, stages retained text, then synchronously appends one CRC-framed transaction. A failed text preparation or rejected append discards the staged cache tail without publishing. The writer queue holds its state mutex only for reservation and release, so queries can capture the prior live revision during the append. Reader exclusion covers publication of the whole live batch and its revision before acknowledgment. Compaction checkpoints and other writers wait for the same lease, so they cannot pair an unpublished journal tail with an older live snapshot. Batches and a document replacement commit as a unit. Relations, relation removals, relation types, triples, composition rules and derived-fact ownership replay alongside vectors and documents.
- **Replay.** An unacknowledged durable frame may replay after process interruption; acknowledgment still follows both durable append and live publication.
- **Compaction.** It writes and verifies a new arena generation, flushes its files, then atomically publishes a `CURRENT` pointer; old segments are removed only after publication. Interruption tests terminate child processes before publication, after publication and after mapping; reopening must see a complete generation. A coherent snapshot is captured while excluding mutations; staging is rebuilt and verified while mutations and readers proceed, sharing immutable document owners and copying sparse vectors and other snapshot records. Concurrent compactors serialize. Two catch-up passes copy the acknowledged journal outside the mutation gate, retaining each original transaction and encrypted payload; final tail replay, flush, publication and index persistence exclude mutations (readers proceed). That final pause depends on outstanding writes and filesystem costs and has no fixed latency bound; snapshot memory and rebuild costs can still affect serving latency. No throughput improvement has been measured.

### Sync guarantees

| Platform | Journal frame sync before a mutation returns |
|---|---|
| Linux | `msync(MS_SYNC)` |
| macOS | `msync` then `fcntl(F_FULLFSYNC)` (plain `msync`/`fsync` leaves data in the drive cache) |
| Windows | `FlushViewOfFile` and `FlushFileBuffers` |

Per-platform detail: [file-format/CRASH-CONSISTENCY.md](file-format/CRASH-CONSISTENCY.md). On macOS the drive-cache flush dominates a single-document commit: a replace took 4.6 ms and a delete 4.0 ms (median of 7 runs, Apple M4 internal SSD) against 0.51 and 0.23 ms without it (`benchmarks/results/durable_commit_macos_v1.json`). Batch APIs commit many documents in one transaction and pay the flush once.

- Files are flushed before publication; directory entries are synced on Unix. Windows uses file flushes and atomic replacement without directory fsync.
- Power-loss guarantees depend on the filesystem and hardware. The tests exercise process interruption, not a simulated disk controller or power failure. Call `flush()` before controlled shutdown and measure recovery on representative stores.

### Committed-but-failed mutations

Audit/provenance sidecars are separate from the arena transaction, so an error there can be returned after the main data mutation committed. Those failures, publication failures after a durable append, and failed post-publication compaction cleanup carry `CommittedMutationError` (HTTP `mutation_committed`; Node errors carry the same code prefix). Inspect the state and reopen or reconcile before retrying; do not repeat a committed mutation merely to obtain an auxiliary record.

- Retained-text cache writes are prepared before the journal append, so their failure leaves live documents, vectors and the revision unchanged.
- Deleted text is hidden during publication; physical cleanup runs afterwards. A cleanup failure reports a committed outcome and retains pending spans; after the I/O fault clears, `flush()` retries cleanup without replaying the mutation, and new writes drain pending cleanup before committing. Canonical compaction drains and syncs the old segment before replacing its cleanup owner. Other publication failures can still require reopen or reconciliation. A reported cache-scrub failure does not mean the old bytes were erased.
- The contract distinguishes durable data from completed auxiliary effects; it supplies no automatic sidecar repair service or exactly-once external delivery. An external repair/delivery workflow must persist its own stable effect ids and deduplicate delivery; the core audit queue is not a general external-effect retry worker.

**Audit queue (schema 6).** Audit intents are journaled with the audited vector memorize/delete operation and with compaction publication. Reopen and `flush_audit()` deliver pending records without repeating business mutations (`flush()` also drains them), and delivery identities deduplicate an audit write whose acknowledgement did not commit. The queue holds at most 1024 intents and rejects further audited mutations before commit when full. Monitor `warm_tenant_audit_pending` and `warm_tenant_audit_recoverable` (the latter distinguishes legacy schemas with no delivery queue); existing schemas need an explicit format upgrade. It does not reconstruct missing historical audit records or provenance effects, expand which operations are audited, or prove history completeness against deletion of whole records.

**HTTP retries.** A retained idempotency key replays a committed failure within its retention window; capacity pressure rejects new keys instead of evicting protected ones; a response that cannot fit the replay byte budget leaves its key needing reconciliation without reexecution; worker panics keep the same protection. Mechanics and storage: [SERVER.md](SERVER.md#retries-and-conditional-requests).

### Memory policy and restore

- Memory policy writes compare the planned core revision inside the journal writer lease, so independent wrappers and raw document writers cannot commit stale conflict or supersession decisions. Snapshot reads and policy operations have bounded retries and may return `MemoryError::Concurrent` (`concurrent_mutation` over HTTP/Node). Retrying the same fact id and content is idempotent. Multi-batch forgetting can commit a prefix before contention; retry the same scope to finish it.
- Tenant restore publishes a durable intent before moving directories; a lifetime parent lock survives the move and recovery processes the intent before a missing tenant directory can be created. An interrupted uncommitted restore rolls back to the complete old store. These are process-interruption guarantees, subject to the filesystem and hardware limits above.

## Index maintenance

Vector updates and deletes modify affected postings and reuse slots. They invalidate trained
ANN caches when necessary. Inserts update a trained index when supported. Persisted ANN files
have checksums, shard topology, and an arena generation/offset checkpoint. Stale or damaged
caches are discarded; retrieval falls back to indices rebuilt from durable records.

Ingestion no longer trains indices inside the write call. `indexStatus()` reports recommendations.
Call `maintainIndices()` between ingestion batches or from a scheduled job. Native Node methods
run off the JavaScript event loop. Rust callers can use
`Arc<WarmCore>::maintain_indices_background()`. Training uses a snapshot outside the transaction
gate and publishes only if the data revision still matches. If concurrent writes changed the
store, the call returns a retryable maintenance error; ingestion remains committed.

## Documents and resource limits

`memorizeDocument` chunks UTF-8 text and atomically replaces any document with the same ID. `searchDocuments` returns document IDs, versions, source URIs, metadata, passages and byte offsets; metadata equality, source URI and document-ID filters apply before ranking. `deleteDocument` removes every owned chunk, including on restart. `memorizeFile` reads a bounded UTF-8 file as a document (default text-storage policy; use explicit `DocumentInput` settings otherwise); use these search/delete APIs for its output.

| Limit | Value |
|---|---|
| Document text | 8 MiB |
| Document ID | 4,096 bytes |
| Chunks per document | up to 4,096 |
| Default chunking | 256 whitespace-delimited words, 32 words of overlap |
| Each chunk | at most 64 KiB; `chunkWords` 1-4,096; overlap must be smaller |
| Metadata | JSON object, up to 64 KiB |
| Source URI / version | 8,192 bytes / 1-1,024 bytes |
| Dense embeddings | finite, nonzero, schema-matching dimensions; at most 1,048,576 values per document |
| `memorizeBatch` | 1-4,096 items and 8 MiB of combined IDs/text, committed atomically |
| `memorize_documents()` / Node `memorizeDocuments()` | Validates a bounded batch before one journal transaction. Native batches need unique IDs and at most 4,096 documents/chunks, 8 MiB of aggregate source fields, 8 MiB of metadata, 1,048,576 supplied or inferred embedding values and 32 MiB of prepared transaction JSON. HTTP batches keep ordered last-write-wins IDs. Validation failures leave existing documents untouched; durable replay uses the existing transaction recovery protocol. |
| Serialized arena transaction | at most 50 MiB; overlap and JSON encoding count toward it |
| Native results | k 1-1,000; candidate limit k-4,096 (the JS adapter caps candidates at 1,000) |
| `DocumentMemory` queue | 32 pending operations by default; a full queue rejects explicitly. Its async-iterable `ingest` waits for each document, applying backpressure. |

Raw passages are stored by default. `storeText: false` omits passages but retains lexical terms, metadata, source identifiers and embeddings; it is not an anonymization control. Reranking requires stored passages.

### Retrieval behavior

- Hybrid retrieval scores eligible chunks with BM25 and exact dense cosine, combines ranked candidates by a weighted blend of min-max normalized BM25 and cosine (RRF on request), then optionally reranks. Exact versus approximate routing, filter/phrase composition and reasoning budgets: [retrieval contracts](guides/retrieval-contracts.md); model and importer compatibility: [upgrade notes](guides/audit-fix-migrations.md).
- Unfiltered lexical queries use exact matching postings and cached collection statistics; filtered queries keep adaptive scan/postings scoring with exact filtered statistics.
- Untrained dense retrieval scans eligible chunks and keeps embeddings in memory. `train_document_index()` (Rust `maintain_indices()` also builds it; Node `trainDocumentIndex()` and `documentIndexTrained`; server route in [SERVER.md](SERVER.md)) builds a separate exact f64 box tree for semantic and hybrid queries, including broad filtered searches; conservative bounds can skip dot products while preserving scores, ordering and source fields. NSG/IVF remain separate sparse-vector strategies. Measure corpus size, memory and latency before choosing deployment capacity.
- Dense cache: 128-250,000 embedded chunks and 1-4,096 dimensions, bounded by 64 Mi embedding coordinates, 64 MiB of box bounds and 80 MiB of estimated auxiliary memory; it shares source/embedding buffers. Smaller, unsupported, stale and filtered cases scan exactly; equal-vector data can need an exhaustive leaf. Updates invalidate the cache and restart rebuilds it explicitly. Training runs outside locks and publishes only a current document generation, with idempotent concurrent publication. Worst-case search is a full scan. JSON float decoding uses the lossless roundtrip option so persisted f64 embeddings and numeric metadata keep their values across restart.
- Search prepares queries before the read gate, captures immutable document owners and relevant postings/statistics under the gate, then scores and builds results after releasing it. Model query variants share one capture, preventing revision mixing during fusion. An overlapping query can return its captured revision after replacement or deletion; a query captured after the acknowledged mutation sees the new state. Captures share source and embedding buffers and retain owners until scoring completes, but still allocate candidate and posting references. A trained unfiltered semantic query pins the index without capturing every dense row; hybrid capture also follows requested lexical postings; filtered and fallback dense capture stays linear in eligible chunks. This has correctness and mutation-progress regressions but no new end-to-end performance measurement.
- Model-stage installation and direct model search reject invalid weights and native result limits. Fusion rejects duplicate chunk identities, conflicting source versions and nonfinite scores; query-dependent ranking scores do not define source identity.

### Experimental angular IVF

`qgraph::ivf::QuantizedIvf` (`experimental`) is a separate index for supplied unit/zero vectors and trained unit centroids, with 4/8-bit coarse codes and optional 4/5/8-bit residual codes. It validates serialized shape, checksum, offsets, ids and codes before use and is not connected to document semantic retrieval or the default sparse index. Development evidence is in `benchmarks/results/superiority_v3/ann_competition/`; local timings are ineligible for superiority claims.

Conservative runtime code envelopes can skip complete coarse-code evaluations without changing selected lists, rankings or serialization. `scanned()` stays selected-list population; `evaluated()` counts actual coarse-code row evaluations. The CLI reports both plus `auxiliary_index_bytes` (separate from serialized `index_bytes`); per-worker query mask buffers add two padded byte arrays. The combined serialized/cache budget stays bounded, with unpruned fallback when cache allocation would exceed it. Synthetic parity and work-count evidence is in `superiority_v3/ivf_routing_v1/`; public workload latency is unmeasured.

## Security

The standard npm build includes the Rust `security` feature. Custom builds reject requested security/provenance capabilities they cannot provide. `securityStatus()` reports active capabilities and the configured embedding space. An encrypted store cannot silently reopen as plaintext, and wrong-key errors are surfaced. Lossy encoding alone does not protect secrets. Scope of encryption and differential privacy: [PRIVACY.md](PRIVACY.md).

## Reproducible validation

```sh
cargo test --locked --no-default-features
cargo test --locked --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo +1.89.0 check --locked --no-default-features
cargo bench --no-run --no-default-features
npm run build
npm test
npm run test:types
npm run test:package
cargo run --release --bin warm-eval -- --dataset tests/fixtures/relevance.json --dimensions 4096 --k 3 --assert-min-recall 0.90
cargo run --release --features cli --bin warm-workload -- --vectors 1200 --operations 300 --dimensions 4096
```

`warm-eval` emits relevance, rank and latency metrics; `warm-workload` measures concurrent query and update p95/p99; `scripts/evaluate-local-model.mjs` compares lexical and hybrid search with real local models, records model revisions and a dataset digest, and measures sparse projection fidelity against dense-cosine neighbors ([evaluation.md](evaluation.md)). Small curated fixtures and synthetic self-recall are regression signals, not quality or capacity guarantees. CI runs default/all-feature Rust tests, a Rust MSRV check, deterministic retrieval floors, and native/packed Node tests on Linux, macOS and Windows.

Measure durable audit write cost on the deployment filesystem with `cargo bench --locked --features security --bench audit_delivery`, with `TMPDIR` set to an owned directory on that filesystem.

- It compares disabled, unsigned and signed auditing for 16 individually committed 256-dimensional vectors per iteration. Initialization, input generation, reopening, result checks and signature verification are outside the timed region; each timed write includes the journal and audit delivery/acknowledgement work.
- Every iteration uses a fresh store, checks an empty pending queue, and verifies the persisted vectors and expected audit count after reopening. `cargo test --locked --features security --bench audit_delivery -- --test` runs the correctness checks without timings.
- The measurements cover single-writer commits, not concurrent saturation, pending-backlog recovery, large vectors, hardware power loss or production tail-latency guarantees. Retain the revision, build profile, filesystem, host load and Criterion output with any reported result.
