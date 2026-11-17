# Operating warm-server

Reference for running the server in production. Settings are in [`configuration.md`](configuration.md); the API is [`openapi.json`](openapi.json).

On Windows, Ctrl+C or Ctrl+Break requests the same graceful drain and flush as
SIGINT/SIGTERM on Unix. Use the authenticated admin endpoints to reload keys or
TLS certificates on Windows; SIGHUP and Unix sockets are Unix-only. Forceful
process termination does not request a graceful shutdown.

## Deployment artifacts

| Target | Path | Notes |
|---|---|---|
| Container | `deploy/docker/Dockerfile` | Static musl binary on `gcr.io/distroless/static-debian12:nonroot`, uid 65532, `/data` volume, self-probing `HEALTHCHECK`. |
| Compose | `deploy/compose/compose.yaml` | One service, read-only root, all capabilities dropped, `127.0.0.1` published, key required. |
| Kubernetes | `deploy/helm/warm-server` | Single-replica StatefulSet with a PVC, probes on `/healthz` and `/readyz`, restricted pod security context, optional Ingress, NetworkPolicy and ServiceMonitor. `helm lint` and `kubeconform -strict` clean. |
| systemd | `deploy/systemd/warm-server.service` | `DynamicUser`, `StateDirectory`, strict filesystem and syscall sandbox, `ExecReload` sends SIGHUP. |
| Benchmarks | `deploy/bench/server_bench.py` | The comparison and soak harness behind `benchmarks/results/server_v1.json` and `server_soak_v1.json`. |

One replica per data directory: a tenant store is owned by a single process through an exclusive file lock. Scale a deployment by giving the pod more CPU and memory, or by running several independent servers for disjoint tenant sets.

## TLS and sockets

`--tls-cert`/`--tls-key` serve HTTPS with rustls (TLS 1.2 and 1.3, ALPN `h2` and `http/1.1`). Replace the files and send SIGHUP or `POST /api/v1/admin/tls/reload`; a bad pair is rejected and the running certificate stays in use. `--unix-socket PATH` serves on a socket with mode 0600 instead of TCP (a stale socket file is replaced, a live one or a non-socket file is refused). Without TLS on a non-loopback address the server logs a `plaintext_on_public_address` warning at startup.

## Health, readiness and shutdown

- `GET /healthz`: the process is serving. Use as the liveness probe.
- `GET /readyz`: 200 once the listener is bound; 503 `not_ready` from the moment SIGTERM or SIGINT arrives. Use as the readiness probe.
- Shutdown order: readiness fails, `--shutdown-delay-seconds` pass with the server still answering (so a balancer can notice), the listener closes, open connections get `--shutdown-grace-seconds`, admitted work finishes, every store is flushed, the process exits 0. Set the orchestrator's termination grace to at least delay + grace + the time a flush takes (the Helm chart defaults to 60 s for 5 + 30).

## Metrics

`GET /metrics` (admin scope, or public with `--metrics-public`) is Prometheus text format 0.0.4. For a key limited to some tenants, the per-tenant series and `warm_tenants_open` cover only those tenants:

| Series | Meaning |
|---|---|
| `warm_http_request_duration_seconds` (histogram, `route`) | Latency by route template, buckets 1 ms to 30 s. |
| `warm_http_requests_total{route,method,status}` | Requests; routes are templates (`/api/v1/documents/{id}`), so cardinality is bounded. |
| `warm_http_in_flight_requests`, `warm_connections_open`, `warm_connections_total` | Load. |
| `warm_work_slots_in_use`, `warm_work_slots_capacity`, `warm_queue_depth` | Admission saturation; sustained `in_use == capacity` with a growing queue means raise `--max-concurrent-work` or add capacity. |
| `warm_idempotency_records`, `warm_idempotency_body_bytes`, `warm_idempotency_unreplayable` | Retained retry keys, response-buffer allocation bytes, and keys requiring reconciliation. Records and bytes expire lazily on the next keyed request. |
| `process_resident_memory_bytes` | RSS (Linux `/proc`, macOS `proc_pidinfo`). |
| `warm_tenants_open`, `warm_tenant_documents{tenant}`, `warm_tenant_vectors{tenant}`, `warm_tenant_index_bytes{tenant}`, `warm_tenant_document_index_trained{tenant}` | Per-tenant size; a tenant that is mid-snapshot is skipped for that scrape. |
| `warm_tenant_compactions_total`, `_failures_total`, `_seconds_total` | Compaction. |
| `warm_auth_failures_total`, `warm_rate_limited_total`, `warm_quota_rejections_total`, `warm_busy_rejections_total`, `warm_idempotent_replays_total`, `warm_tenant_evictions_total`, `warm_tls_handshake_failures_total`, `warm_ingest_documents_total` | Policy outcomes. |
| `warm_ready`, `warm_up`, `warm_uptime_seconds`, `warm_build_info{version}` | State. |

Per-tenant series are bounded by `--max-tenants` (only open stores report).

## Logs

One JSON object per line on stderr (`--log-format text` for key=value). Every request logs `method`, `route` template, `status`, `latency_ms`, `tenant`, `key_id`, `bytes_in` and `request_id`; `X-Request-Id` from the client is honored when it is 1 to 64 characters of `[A-Za-z0-9._-]`, otherwise a fresh id is generated, and the id appears in the response header and in error bodies. Request bodies, vectors, query text, metadata, document ids and credentials are never written. Lifecycle events (`listening`, `shutdown_started`, `shutdown_complete`, `tenant_evicted`, `snapshot_created`, `snapshot_restored`, `restore_stage`, `compaction_done`, `keys_reloaded`, `tls_reloaded`) and failures carry identifiers and counts only. A test checks that secrets and content sent through the API never appear in the log.

Restore uploads emit `restore_stage` events for receive, upload sync (including byte count), unpack, store sync, verification, and installation. These contain tenant identifiers and stage names, never archive contents; the timestamps help locate a stalled restore.

## Backup and restore

- `POST /api/v1/snapshots` flushes the tenant's store and copies it to `<data-dir>/snapshots/<tenant>/<id>/`. The tenant pauses (requests wait) while the copy runs; other tenants are unaffected. Expect roughly the store's size in extra disk per snapshot; an empty store is about 64 MiB because the engine preallocates its first arena segment.
- `GET /api/v1/snapshots/{id}/archive` streams the snapshot as a tar. This is the off-box backup: store it anywhere.
  Format-4 archives omit unused zero padding beyond the active journal's required 64 MiB allocation boundary. Live files and stored snapshots are unchanged; archive size can therefore be smaller than the snapshot's reported on-disk bytes. Journal contents, the end marker, nonzero tails, and older formats are preserved.
- `POST /api/v1/snapshots/{id}/restore` replaces the tenant's store with the snapshot; `PUT /api/v1/restore` does the same from an uploaded tar (to the same server or a different one). Both verify the candidate store (checksums, manifests and a full open that authenticates every frame) before touching the live one, refuse archives with links, device nodes or paths outside the store, and, if anything fails after the swap starts, put the previous store back. The replaced store is kept as snapshot `pre-restore-<time>`; delete it when you no longer need it.
- Uploaded tar archives allow GNU long-name and local PAX metadata records up to 64 KiB each. Larger metadata is rejected before extraction; PAX size overrides are included in the expanded-byte limit. Staged read-only files retain their permissions when flushed, including on Windows.
- Snapshots on the same disk protect against mistakes, not against disk loss. Copy the archive off the machine for that.

## Compaction

`--compaction-interval-seconds N` compacts every open store that changed since its last compaction, one at a time. `GET /api/v1/compaction` shows the schedule and the last run (`runs`, `failures`, `last_trigger`, `last_started_at`, `last_duration_ms`); `POST /api/v1/compaction` runs it now. Compaction rewrites the store's logs without the superseded and deleted entries. It runs on a blocking worker outside the request slots; the engine decides how long it holds writers back, so schedule it for quiet periods on large stores.

## Capacity planning

- Memory is dominated by open stores. `--tenant-idle-seconds` closes idle ones; `warm_tenant_index_bytes` and `process_resident_memory_bytes` show the effect.
- `--max-concurrent-work` is the number of requests the engine runs at once; ingest and search are CPU-bound, so set it near the number of cores and use `--max-queue` for bursts.
- Rate limits and quotas are per tenant and off by default; enable them when tenants are mutually untrusted.

## Measured behavior

Two runs of the same harness (`deploy/bench`), on a shared 10-core Apple M4 with client and servers on one host, so absolute figures are indicative. Synthetic clustered vectors, 384 dimensions, top 10, 6 s per level after a 2 s warm-up, a fresh server per system and size, one run each. Recall at 10 was 1.0 for every system and size in both runs. Neither run is at the current release, and the two were not run as a pair.

**Run 1, exact scan** ([`benchmarks/results/server_v1.json`](../../benchmarks/results/server_v1.json): 2026-10-20, `hms-server 0.6.1`, `git_commit` not recorded, load average 25 at the start, Qdrant 1.19.2). QPS at 1 / 4 / 16 clients and p50 at 16:

| N | warm-server (exact) | Qdrant HNSW | Qdrant exact | SQLite + FastAPI |
|---|---|---|---|---|
| 1,000 | 828 / 2,538 / **7,780**, 1.2 ms | 772 / **2,676** / 4,398, 1.6 ms | 777 / 2,491 / 4,310, 2.4 ms | 570 / 933 / 1,316, 10.8 ms |
| 10,000 | 135 / 487 / 895, 12.9 ms | 480 / 1,159 / **2,458**, 3.0 ms | **579 / 1,363** / 2,137, 2.9 ms | 206 / 190 / 377, 35.2 ms |
| 50,000 | 22 / 92 / 178, 85.2 ms | **693 / 1,440 / 1,899**, 5.7 ms | 591 / 847 / 1,198, 9.6 ms | 422 / 502 / 504, 31.3 ms |

In run 1 the exact-scan route is fastest only at 1,000 documents with 1 and 16 clients; Qdrant HNSW is ahead at 1,000 documents with 4 clients. At 10,000 and 50,000 documents both Qdrant variants are ahead at every client count, and SQLite + FastAPI is ahead at 50,000 and at 10,000 with 1 client. The `exact+index` variant in this run (a bounding-box tree, since replaced) was not faster than exact. The exact route's vector query is an exact scan inside the engine (about 6.9 of 7.2 ms at 10,000 documents is spent in the engine search). Ingest was 2,141 to 2,899 documents/s for warm-server (exact) against 1,461 to 4,667 for the others (batches of 500, durable commit per batch).

**Run 2, graph route** ([`benchmarks/research/server-ann/server_v2.json`](../../benchmarks/research/server-ann/server_v2.json): 2026-10-21, `warm-server 0.6.1` at `9c3e4521`, load average 10 to 13 at the start). The `exact+index` variant serves vector queries from the vertex graph after `index/maintain`. QPS at 1 / 4 / 16 clients and p99 at 16:

| N | warm-server (graph) | warm-server (exact) | Qdrant HNSW | Qdrant exact | SQLite + FastAPI |
|---|---|---|---|---|---|
| 1,000 | **5,856 / 10,690 / 16,066**, 2.4 ms | 2,578 / 5,580 / 7,669, 5.1 ms | 4,148 / 4,277 / 3,151, 19.8 ms | 4,427 / 4,168 / 3,202, 18.0 ms | 2,417 / 3,198 / 3,323, 5.7 ms |
| 10,000 | **5,491 / 7,904 / 12,708**, 4.0 ms | 353 / 932 / 924, 39.1 ms | 1,721 / 2,126 / 2,505, 30.1 ms | 1,514 / 1,930 / 2,526, 28.4 ms | 1,207 / 1,408 / 1,475, 20.3 ms |
| 50,000 | **3,099 / 6,838 / 11,332**, 5.0 ms | 66 / 107 / 168, 146.5 ms | 930 / 1,662 / 1,801, 42.1 ms | 388 / 620 / 623, 108.5 ms | 345 / 420 / 434, 60.7 ms |

In run 2 the graph route is fastest at every size and client count. It loses on memory: peak server RSS during the query levels at 50,000 documents was 674 MB against 213 MB for Qdrant HNSW, and its ingest (3,337 to 3,952 documents/s) is below Qdrant HNSW (4,861 to 5,976). Run 2 also measured every other system faster than run 1 on a less loaded host, so the two runs' absolute figures are not comparable; the verdict that holds for the graph route is run 2's, and a paired rerun at the current release has not been made.

**Soak** (4 minutes, 16 clients, 137,634 successful mixed requests, tenants rotating through 12 ids with at most 6 open): no errors other than 27 expected `tenant_limit` refusals, open file descriptors 26 to 35, work slots idle at the end, every tenant's contents matched the client's model both before and after a SIGTERM restart, SIGTERM exit code 0. RSS moved between 55 and 270 MB following tenant open and eviction cycles and ended at 144 MB from 139 MB at the start; the second-half linear fit reads +4.4 MB/min over that oscillation, so a longer run is needed to call it flat.

**Container** (Lima VM, Docker 29, arm64): image 15,015,731 bytes (15 MB, of which 5.84 MB is the server binary); uid 65532, read-only root, no capabilities; RSS 10.0 MB (cgroup 13.5 MiB) after loading 2,000 documents of 16 dimensions; `docker stop` took 0.19 s and exited 0; data survived a restart on the volume; 118 MiB at 10,000 documents of 384 dimensions. Compose and the systemd unit were run for real (`systemd-analyze security`: exposure 1.2 OK). Not measured: x86_64 images, musl against glibc allocator throughput, a live Kubernetes cluster (no kind available; `helm lint` and `kubeconform -strict` pass).

Schema 6 audit recovery exposes `warm_tenant_audit_pending` (committed records awaiting
acknowledgement) and `warm_tenant_audit_recoverable` (1 for schema 6, 0 for legacy schemas).
Investigate a persistent nonzero backlog before its 1024-entry limit rejects audited writes.
Repair the underlying file/storage failure and call the existing flush operation or restart;
never replay the business mutation to manufacture its missing audit record. Preserve both
`audit.bin` and `audit.effects.bin` in backups. An upgrade enables future durable delivery,
without reconstructing historical gaps.
