---
search:
  boost: 1.6
---

# warm-server

`warm-server` is the HTTP service for WARM: the endpoints behind the portable `warm-sdk` clients for JavaScript/TypeScript (`bindings/js/warm-sdk`) and Python (`bindings/python/warm-sdk`), one `WarmCore` store per tenant, plus scoped per-tenant authentication, limits and backpressure, observability, TLS and unix sockets, snapshots, compaction and graceful shutdown. The clients are separate from the optional embedded Node package, `writerslogic-warm`.

| File | Contents |
|---|---|
| this page | Run it, the API at a glance, authentication, operations, versioning |
| [`openapi.json`](openapi.json) | The OpenAPI 3.1 contract, generated from the server (also served at `/openapi.json`) |
| [`configuration.md`](configuration.md) | Every flag and `WARM_*` variable, defaults, meaning |
| [`errors.md`](errors.md) | The RFC 9457 error model and every error code |
| [`deploy/`](https://github.com/writerslogic/warm/tree/main/deploy) | Dockerfile, Compose file, Helm chart, systemd unit, benchmark harness |

## Build and run

```sh
# or: cargo install --locked --features server,security --bin warm-server warm-rust
# (prebuilt binaries via scripts/install-warm-server.sh start with the first server-v release)
cargo build --release --features server,security --bin warm-server

# One key valid for every tenant (the original setup):
WARM_API_KEY="$(openssl rand -hex 32)" ./target/release/warm-server --input-dim 384

# Per-tenant keys with scopes (authentication below):
./target/release/warm-server keygen --id acme-rw --tenants acme --scopes read,write --keys-file keys.json
./target/release/warm-server --keys-file keys.json --input-dim 384

# Local development without credentials (loopback or a unix socket only):
./target/release/warm-server --no-auth --input-dim 384
```

The server refuses to start without credentials unless `--no-auth` is given, and refuses `--no-auth` on a non-loopback address. It prints `warm-server listening on <addr>` on stdout once bound; everything else goes to stderr as JSON log lines.

```python
from writerslogic_warm_sdk import WarmClient
client = WarmClient(url="http://127.0.0.1:8080", api_key="<token>", tenant_id="acme")
```

Release binaries are built with `--features server,security,mimalloc` and install `writerslogic_warm::allocator::Tuned` (mimalloc, arenas committed on demand, immediate purge); `MIMALLOC_ARENA_EAGER_COMMIT` and `MIMALLOC_PURGE_DELAY` override the options. A build without `mimalloc` uses the system allocator: identical results, ingest slower by about the allocator's share (roughly 15% at 250k documents), same peak resident size.

## API keys

Every request carries an API key. `WARM_API_KEY` sets one key valid for every tenant; `--keys-file` loads per-tenant keys with read, write or admin scopes, created with `warm-server keygen`. Format, scopes and rotation: [authentication](#authentication).

## API at a glance

The contract is [`server/openapi.json`](openapi.json). Every operation lists the scope it needs (`read`, `write`, `admin`) as its security requirement. Errors are `application/problem+json` ([`server/errors.md`](errors.md)).

| Request | Scope | Success |
|---|---|---|
| `POST /api/v1/documents` one document | write | 200 `{"added": 1}` |
| `POST /api/v1/documents/batch` 1..=`max-batch` documents, one atomic commit | write | 200 `{"added": n}` |
| `POST /api/v1/documents/stream` NDJSON, committed every `--stream-batch` lines | write | 200 `{"added","rejected","errors":[{"line","message"}],"errors_truncated"}` |
| `DELETE /api/v1/documents/{id}[?purge=false]` (percent-encoded; by default the delete is recorded as a durable pending purge and a later scrubbing compaction erases the data; `synchronous=true` scrubs before the response, `purge=false` opts out) | write | 200 `{"deleted": true}` |
| `GET /api/v1/documents?limit&after` ids and metadata, id cursor | read | 200 `{"documents":[{"id","metadata"}],"next_cursor","count"}` |
| `GET /api/v1/documents/{id}/chunks/{index}` retained chunk, zero-based index | read | 200 `{"revision","document","index","id","startByte","endByte","text"}`; 404 if absent |
| `GET /api/v1/stats` | read | 200 `{"tenant","documents","vectors","documentIndexTrained","usedBytes","capacityBytes"}` |
| `POST /api/v1/query` | read | 200 `{"matches":[{"id","text","metadata","score"}]}` best first |
| `POST /api/v1/query/vector` | read | 200 `{"matches":[{"id","score"}]}` best first; body is the query as raw little-endian `f32` |
| `POST /api/v1/search` | read | 200 `{"hits":[{"id","score"}]}` best first (BM25; model stages when configured) |
| `POST /api/v1/index/maintain` | write | 200 `{"documentIndexTrained": bool}` |
| `POST /api/v1/memory/{remember,recall,explain,forget,get,history,applyForgetting,consolidate}` | write / read | the memory API, `docs/reference/api.md` |
| `GET /api/v1/compaction`, `POST /api/v1/compaction` | read / admin | status, run now |
| `POST /api/v1/snapshots`, `GET /api/v1/snapshots`, `DELETE /api/v1/snapshots/{id}` | admin | server-side snapshots |
| `GET /api/v1/snapshots/{id}/archive`, `POST /api/v1/snapshots/{id}/restore`, `PUT /api/v1/restore` | admin | tar download, restore from a snapshot or an uploaded archive |
| `POST /api/v1/admin/keys/reload`, `POST /api/v1/admin/tls/reload`, `GET /api/v1/admin/tenants` | admin, all tenants | reload credentials, list tenants |
| `GET /healthz`, `GET /readyz`, `GET /metrics`, `GET /openapi.json` | none (`/metrics`: admin unless `--metrics-public`) | liveness, readiness, Prometheus, spec |

### Documents and queries

A document is `{"id": str, "vector": [float], "text"?: str, "metadata"?: object}`.

| Field | Rule |
|---|---|
| `vector` | Exactly `--input-dim` finite f64 values, not all zero; query vectors follow the same rule. Normalization handles very large and very small finite magnitudes without narrowing to f32. |
| `id` | 1 to 256 bytes, no control characters, not starting with `hms:`. Adding an existing id replaces it. |
| `text` | Must fit one engine chunk (at most 4096 words and 64 KiB), because one vector describes one stored chunk; split longer documents and send one vector per part. |
| `metadata` | At most 64 KiB. |
| Batch totals | 8 MiB of ids and text, 8 MiB of metadata, 1,048,576 embedding values; the native prepared transaction is limited to 32 MiB. All documents are validated before one atomic commit; repeated ids keep their last occurrence while `added` stays the incoming count. |

`GET /api/v1/documents/{id}/chunks/{index}` reads one retained chunk with document identity, metadata, version, source URI and `chunkCount`. It requires read scope and the selected tenant.

- `text: null` means text was not retained; a missing document or out-of-range index returns 404. HTTP-ingested documents have one chunk; native stores restored into the server may have more.
- A chunk holds at most 8 MiB of UTF-8 text (the native whole-document limit), plus bounded document metadata. JSON escaping and serialization copies can make the response and peak allocation larger.
- Chunks preserve their stored passages, including internal whitespace. They do not recover omitted leading/trailing whitespace, gaps between nonoverlapping chunks, unretained text, or original pre-normalization embedding values. Recorded byte spans describe the ingested chunk; vector-only HTTP records use an unretained placeholder, so their spans are not evidence of original source text. Overlapping chunks must not be concatenated blindly.
- `revision` is an opaque string captured with the chunk, not the user-supplied document version and not a durable cross-restart sequence number. When assembling multiple reads, require the same revision throughout and retry if it changes. Concurrent replacement/deletion or text-segment rebuilding triggers bounded internal retries; repeated changes return 503 `concurrent_mutation` with `Retry-After`. Storage failures remain errors, not missing text.
- Chunk reads support ETag and If-None-Match for the complete returned representation.

`POST /api/v1/query` takes `{"query_vector": [...], "top_k"?: 1..=max-top-k (default 3), "filter"?: object}`. `filter` keeps documents whose metadata contains every filter key with an equal JSON value; it is applied before ranking, so it never reduces the number of matches below `top_k` when enough documents match. `score` is the exact cosine similarity between the query and the stored embedding.

`POST /api/v1/query/vector` takes the query vector as the raw request body (`--input-dim` little-endian `f32` values, `Content-Type: application/octet-stream`) and the options in the query string: `top_k` (default 10), `ef`, `index` (`auto`, `exact`, `graph`, as for `query`), and `id_gte` / `id_lt`, which keep documents whose id is an integer in that range. A body whose length differs from `--input-dim * 4` is refused with 422 before it is read, and one holding a non-finite or all-zero vector after it. The reply carries ids and cosine scores only (no stored text or metadata) and skips the per-hit document read of `query`. A range filter is applied inside the graph search: a selective range is scored directly from the index's codes, a permissive one by a wider search that keeps the allowed hits; it needs the vector graph (run `index/maintain` first), otherwise 409 `index_unavailable`.

`POST /api/v1/search` takes `{"query": "...", "top_k"?: 1..=max-top-k (default 10), "include"?: ["text", "metadata"]}` and ranks the tenant's documents, memory facts included, by BM25 (through the on-device model stages instead when the server has them), one hit per document. Hits carry only `id` and `score` unless `include` names stored text or metadata; it applies no memory semantics (validity time, supersession), for which use `memory/recall`.

`index` routes an unfiltered query:

| `index` | Behavior |
|---|---|
| `auto` (default; `--query-index`) | Graph index when one is built, else the exact scan. |
| `exact` | Always scans. |
| `graph` | 409 `index_unavailable` until maintenance has built one. |

`ef` is the beam width of a graph query (`--query-ef`, default 128).

The graph index is persisted with the tenant store ([Storage](#storage)): every document ingested with a vector is appended to the tenant's vector log before the engine commits it; maintenance builds the graph over those rows and writes it next to the store; a restart opens the written graph instead of rebuilding it. Maintenance after a restart is a no-op until something changed; scheduled compaction rebuilds after 64 mutations. A graph file that does not read back is ignored and its rows still answer, exactly, until the next maintenance; after a stop that was not clean the index checks each of its documents against the engine before serving. The graph covers documents ingested through the documents API; while a tenant also holds embeddings written through the memory API, its unfiltered queries go to the engine's own snapshot instead.

Maintenance also builds the tenant's optional document index over stored embeddings and reports whether one is trained; a store too small to train returns `false`. Ingest does not trigger training. If the store changes during training, maintenance returns a fixed internal error and can be retried.

### Bulk ingest

`POST /api/v1/documents/stream` with `Content-Type: application/x-ndjson` reads one document object per line (blank lines ignored, the last line may lack a newline) and groups every `--stream-batch` nonblank lines into one batch, independent of transport frames. Valid documents in each batch commit in one transaction. Rejected lines are reported with their 1-based line number (the first 100) and do not stop the stream. The stream is not atomic across batches: after a failure mid-stream, the committed batches stay. Size limits are `--max-stream-bytes` and `--max-stream-line-bytes`, independent of `--max-body-bytes`; a stalled upload times out after `--body-timeout-seconds` of silence.

### Retries and conditional requests

**Idempotency.** `Idempotency-Key` on `POST /api/v1/documents`, `/documents/batch` and the memory operations replays the recorded status and response bytes for the same key, tenant, method, path and body (`Idempotent-Replayed: true`), including after restart. Repeated key headers are rejected before execution.

| Situation | Result |
|---|---|
| Same key, different request | 422 `idempotency_key_reused` |
| Key still running | 409 `idempotency_in_flight` |
| Interrupted request whose outcome was not recorded | 409 `idempotency_reconciliation_required`; never automatically reexecuted |
| Known transient failure (5xx, 429, 408) | Key released, unless the mutation committed, its outcome is uncertain, or erasure invalidated the request |
| Capacity reached (`--idempotency-capacity`) | New keys get 503 before execution; existing keys can still replay |
| Response does not fit `--idempotency-max-bytes` | Still returned to the original caller; later use of its key requires reconciliation |

The server durably reserves the key before executing work and persists the response, or a bounded reconciliation marker, before acknowledging it. The receipt and business mutation are separate commits: this prevents blind retries across the crash gap but does not make arbitrary operations exactly once. Reconcile an ambiguous outcome or receipt expiry by comparing application-owned ids, versions and resulting state.

**Receipt storage.**

- Receipts live in `DATA_DIR/http-retries`, outside tenant snapshots, under one exclusive process lease. Checksummed private files hold tenant/key digests, request fingerprints, status and response bytes; request bodies and credentials are not copied. Responses can contain source text, so like the server's other unencrypted files, receipts need an encrypted volume when encryption at rest is needed. Copying or deleting this directory independently of the server's data can discard retry protection.
- Admission preserves running and unexpired completed keys. Size `--idempotency-capacity` for the distinct keys admitted during the TTL plus in-flight work: the defaults (10,000 keys, 600 s) are exceeded by a sustained stream of new keys above roughly 16 per second. `--idempotency-max-bytes` (64 MiB by default) independently bounds retained serialized response buffers; it does not cover response construction, active HTTP sends, key/record overhead or total process RSS (the key-count limit bounds record overhead separately). Response clones share one serialized buffer.
- Expiry uses a monotonic clock. Restart begins a fresh retention window at least as long as the configured TTL and the longest recorded TTL, so a clock change or shorter startup setting cannot release protected keys early. Running keys never expire in-process; interrupted reservations become reconciliation markers on restart. Startup fails if receipt integrity, ownership or configured storage bounds cannot be satisfied.
- Before forget, forgetting maintenance, consolidation, document deletion or tenant restore, a durable tenant barrier blocks new keyed calls and scrubs previously retained response bodies; protected key fingerprints remain as reconciliation markers. In-flight completions cannot restore those bodies; the current keyed erasure operation can retain its own result. The barrier spans the operation and survives interruption; startup finishes interrupted receipt scrubbing before admitting requests, and cleanup failure leaves protection unavailable. Restore invalidates receipts once replacement begins, even if it rolls back. Other tenants are unaffected; independent backups are not altered.

**Conditional requests.** `GET /api/v1/stats`, `GET /api/v1/documents` and retained-chunk reads return a weak `ETag` derived from the serialized response and process epoch, and answer matching `If-None-Match` with 304 (weak comparison across repeated header lines and quoted lists; a lone `*` matches an existing representation; malformed conditions are ignored; see [RFC 9110 §13.1.2](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.2)). Validators are response-specific, not tenant revision tokens: unchanged pages may keep their tag after unrelated writes. Conditional reads render and hash the response before deciding whether to omit the body, which prevents reused validators during concurrent writes but adds work to 304 responses. Chunk validators include the captured revision; they are not whole-document snapshot handles.

## Tenants

`X-Tenant-ID` selects the store; a key bound to one tenant needs no header ([authentication](#authentication)), otherwise the default is `default`.

| Rule | Behavior |
|---|---|
| Id format | 1 to 64 characters of `[A-Za-z0-9_-]`, case-sensitive; anything else is 400 |
| Repeated header (even identical) | 400 `invalid_tenant` before store access |
| Filesystem | `<data-dir>/tenants/<hex of id>/`, so ids never reach the filesystem as paths |
| Opening | On first use. Simultaneous first requests share one initialization; pending initializations count toward `--max-tenants`; filesystem work runs outside the registry lock; a failed initialization releases its reservation. |
| Idle | Flushed and closed after `--tenant-idle-seconds`, so `--max-tenants` bounds open stores, not tenants ever served. |

## Limits and backpressure

Flags and defaults: [server/configuration.md](configuration.md#request-limits-and-backpressure).

| Mechanism | Behavior |
|---|---|
| `--max-concurrent-work` (default one slot per available core, at least 4) | Admitted requests reading bodies or running engine work. When full: 503 `server_busy` with `Retry-After` immediately, or, with `--max-queue` above zero (default 4096), a bounded wait of up to `--queue-timeout-ms` (10 s). A closed-loop client count up to the queue size is queued, not shed; `--max-queue 0` restores immediate shedding. `warm_queue_depth` and `warm_work_slots_in_use` expose saturation. |
| `--rate-limit-rps` / `--rate-limit-burst` | Per-tenant token bucket: 429 `rate_limited`, `Retry-After`. Off by default. At most 10,000 tenant buckets; when full, a new tenant gets 429 until idle buckets are reclaimed; existing buckets keep working. |
| `--max-documents-per-tenant` | Per-tenant document quota: 429 `quota_exceeded`. Off by default. |
| `--max-connections`, `--header-read-timeout-seconds`, `--body-timeout-seconds` | Cap connections; drop clients that dribble headers; bound body reads. A disconnected client releases body admission immediately, but a running blocking operation keeps its work slot until it finishes. |

Parsing, validation, store initialization, ingest, search and maintenance run on blocking workers; the async runtime only moves bytes.

Document ingestion and memory `remember` serialize quota checks through commit, so concurrent requests share the same limit. Supply explicit memory ids for retries at a full quota: requests without ids are conservatively counted as new facts. Summaries created by an opt-in consolidation policy currently bypass quota admission.

## Shutdown

SIGINT or SIGTERM makes `/readyz` fail at once. With `--shutdown-delay-seconds` the server keeps serving for that long so a load balancer can drain it; then the listener closes, open connections get `--shutdown-grace-seconds` to finish, admitted work (including operations whose clients disconnected) runs to completion, every open store is flushed and the metadata logs are synced, and the process exits 0. An interrupted flush exits non-zero.

## On-device model stages (`local-models`)

Built with `--features server,local-models`, the server can run the model stages itself instead of requiring client vectors. Every stage is off unless its flag is given; with none given the server behaves as described above. Flags and `WARM_*` variables: [server/configuration.md](configuration.md#on-device-models-features-local-models-builds).

| Flag pair | Stage |
|---|---|
| `--embed-model DIR --embed-revision SHA` | Qwen3-Embedding. Documents may omit `vector` (the text is embedded); `--input-dim` must equal the model's hidden size (1024 for 0.6B). |
| `--rerank-model DIR --rerank-revision SHA` | Qwen3-Reranker over the fused top 20 of a text query. |
| `--llm-model DIR --llm-revision SHA` | Qwen3 instruct LLM (greedy decoding). |
| `--extract-facts` | At ingest, the LLM lists the personal facts in each document; they are indexed with it (lexical and embedding key), the stored text is unchanged. |
| `--rewrite-queries` | The LLM rewrites each text query; the original and the rewrite are searched and merged by RRF. |

A query may then be `{"query": str, "today"?: str, "top_k"?, "filter"?, "query_vector"?}`: hybrid BM25 + cosine per query form, weighted RRF across forms, then the re-rank, with the tuned LongMemEval parameters. `score` is then the fused rank score, not a cosine. Vector-only queries keep the cosine path.

**What runs where.** Every stage runs in the server process: candle on the CPU, on the GPU through Metal on macOS, or through CUDA in builds made with `RUSTFLAGS="--cfg warm_cuda"`. Weights are read from the given local directories, which must record the given revision (a `REVISION` file or the metadata `hf download --revision <sha> --local-dir <dir>` writes); the server never downloads a model and makes no network calls, so no document or query text leaves the machine. Fact extraction adds one LLM generation per document and query rewriting one per text query.

## Storage

Each tenant is one engine store. Documents (embedding, text and metadata) go through the engine's document API. A batch writes its final distinct ids in one transaction; a validation failure leaves existing documents unchanged. Embeddings are kept at full precision, so the sparse-code path measured in [public benchmarks](../benchmarks/index.md) does not apply to this service.

The store records the embedding model, revision and input dimension (`--embedding-model`, `--embedding-revision`, `--input-dim`). Reopening with different values is refused, so vectors from different models are never mixed.

`<tenant>/vindex/` holds the persisted graph index and is copied with snapshots:

| File | Contents |
|---|---|
| `rows.log` | One checksummed frame per inserted, replaced or deleted vector, synced per request at the engine's durability class |
| `CURRENT`, `graph-<epoch>.vertex`, `rows-<epoch>.bin`, `ids-<epoch>.bin` | The snapshot maintenance last published, each written through a temporary file and a rename |
| `dirty` | Present between the first mutation after open and a clean close |

New vector snapshots record a checksum of `rows-<epoch>.bin` in `CURRENT`; a mismatch disables the snapshot so queries fall back to the engine. Older snapshots remain readable and gain the checksum on the next maintenance run. Snapshots, compaction and restore: [Operations](#operations).

## What it does not provide

- Single node: no replication, clustering or HA. A tenant store is owned by one process.
- No encryption at rest (use an encrypted volume) and no audit or attestation features at this layer.
- Keys are bearer tokens: use TLS (`--tls-cert`, or a terminating proxy) when the listener is not loopback or a unix socket.
- The SDK's `zero_trust_key` vector masking is obfuscation, not encryption. The server sees masked vectors, learns every pairwise similarity, and stores text and metadata in plaintext.
- No mutual TLS, no OIDC, no per-key rate limits; limits are per tenant.

## Authentication

| Mode | How | For |
|---|---|---|
| Per-tenant keys | `--keys-file keys.json` (`WARM_KEYS_FILE`) | Production. Keys are bound to tenants and scopes, stored only as SHA-256 digests, can expire, and rotate without a restart. |
| One shared key | `WARM_API_KEY` (environment only) | Small deployments. Valid for every tenant with every scope. |
| No authentication | `--no-auth` | Local development. Refused unless the listener is loopback or a unix socket, and refused together with credentials. |

With neither a key nor `--no-auth` the server does not start. A token looks like
`warm_<id>.<64 hex digits>` (256 random bits), is shown once by `warm-server keygen`, and only its
SHA-256 is kept. Requests carry one `Authorization: Bearer` field; repeated fields return 401.

```sh
warm-server keygen --id acme-reader --tenants acme --scopes read --keys-file keys.json
warm-server keygen --id ops --tenants '*' --scopes admin --expires-in-days 90 --keys-file keys.json
```

| Scope | Allows |
|---|---|
| `read` | Query, list, stats, compaction status. |
| `write` | Add, replace and delete documents, index maintenance. Does not include `read`. |
| `admin` | Everything above, plus snapshots, restore, compaction runs, key and TLS reload, and `/metrics`. |

A key bound to one tenant needs no `X-Tenant-ID`; a multi-tenant key selects with `X-Tenant-ID`.
Key-file entries have `id`, `hash` (`sha256:` plus 64 hex digits), `tenants` (ids or `["*"]`),
`scopes`, and optional `not_before`, `expires_at` and `disabled`. To rotate: generate a new key,
reload (`kill -HUP <pid>` or `POST /api/v1/admin/keys/reload`), roll the token out to clients, then
disable the old key and reload. Rotating `WARM_API_KEY` needs a restart.

Not provided: per-request signing, mutual TLS, lockout after repeated failures
(`warm_auth_failures_total` counts them), or per-key rate limits (limits are per tenant,
`--rate-limit-rps`).

## Operations

- **Deployment.** The `deploy/` directory has a Dockerfile (static binary on a distroless image,
  `/data` volume), a Compose file, a single-replica Helm StatefulSet and a systemd unit. Run one
  replica per data directory: a tenant store is owned by one process through an exclusive file lock.
- **TLS.** `--tls-cert` and `--tls-key` serve HTTPS (TLS 1.2 and 1.3). Replace the files and send
  SIGHUP or `POST /api/v1/admin/tls/reload`; a bad pair is rejected and the running pair stays.
- **Health.** `GET /healthz` is liveness. `GET /readyz` is 200 once the listener is bound and 503
  from the moment SIGTERM or SIGINT arrives. Shutdown fails readiness, waits
  `--shutdown-delay-seconds`, closes the listener, then drains for `--shutdown-grace-seconds`.
- **Metrics.** `GET /metrics` (admin scope, or public with `--metrics-public`) serves Prometheus
  text: request latency and counts by route template, in-flight load, admission queue depth,
  per-tenant document, vector and index sizes, compaction, auth failures, rate-limit and quota
  rejections, readiness and build info.
- **Logs.** One JSON object per line on stderr (`--log-format text` for key=value) with method,
  route template, status, latency, tenant, key id and `request_id`. Secrets are never logged.
- **Backup.** `POST /api/v1/snapshots` flushes a tenant store and copies it under the data
  directory; the tenant pauses while it copies. `GET /api/v1/snapshots/{id}/archive` streams the
  snapshot as a tar for off-box storage. Restore verifies the upload before installing it.

## Versioning

The contract is `openapi.json`, and a test fails when the committed file differs from the build.
Within `/api/v1` changes are additive only: new endpoints, optional fields, response fields (ignore
unknown ones) and error codes (treat an unknown `code` by its HTTP status). v1 never removes or
renames an endpoint, field, header or error code, changes a field's type or meaning, makes an
optional field required, or changes an existing status code. A breaking change ships as `/api/v2`,
served beside v1 for at least two minor releases with a `Deprecation` header on v1. Stored data is
versioned separately; a release that needs a store migration says so and refuses to open a store it
cannot read rather than altering it. Flags and variables follow the crate's semver.
