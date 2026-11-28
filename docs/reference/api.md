# API overview

Generated reference lives under [API](../api/index.md): rustdoc for the Rust crate, typedoc for the
embedded Node package, the HTTP contract from `openapi.json`, the C header and the Python SDK. This
page says which surface to use and lists the Cargo features.

## Surfaces

| Surface | Use it for | Main entry points |
|---|---|---|
| Rust (`warm-rust`, library `writerslogic_warm`) | Embedding the engine in a Rust program | `WarmCore::new`, `memorize_documents`, `search_documents`, `delete_document`, `memorize_vector`, `query_vector`, `memory_api::Memory` (`remember`, `recall`, `forget`, `explain`) |
| Embedded Node (`warm-memory`) | The same engine in Node, in process | `WarmMemorySystem`, `DocumentMemory` (`ingest`, `search`), `Memory` (`remember`, `recall`) |
| HTTP (`warm-server`) | Multi-process or remote access, any language | `/api/v1/documents`, `/api/v1/query`, `/api/v1/search`, memory endpoints; see [server](../server/index.md) and [errors](../server/errors.md) |
| JavaScript client (`warm-sdk` on npm) | Calling `warm-server` from JavaScript or TypeScript | `WarmClient`, `.memory` |
| Python client (`warm-sdk` on PyPI) | Calling `warm-server` from Python | `WarmClient`, `.memory.remember_many`, `.memory.recall`, importers |
| Python native (`warm-python`) | In-process vector indexes and phase vectors, and the `Warm` class from 0.7.0 | `writerslogic_warm.Warm` |
| MCP (`warm-mcp`) | Agent and editor integration | memory tools over stdio or streamable HTTP |
| C ABI | Go, Java and .NET bindings | `hms1_*` symbols; check `hms1_abi_version()` |

## Atomic batches

`write_batch` applies document upserts and deletes as one unit: a reopened store and every
read-only reader sees all of the batch or none of it, after a crash at any point.

| Surface | Call |
|---|---|
| Rust | `WarmCore::write_batch(Vec<BatchOp>) -> Vec<BatchResult>`; `BatchOp::Memorize { document }`, `BatchOp::Delete { id }` |
| Node | `await sys.writeBatch([{ op: 'memorize', document }, { op: 'delete', id }])` returns `[{ op, chunks }, { op, existed }]` |
| Python | `warm.write_batch([{"op": "memorize", "document": {...}}, {"op": "delete", "id": "..."}])` returns the same dicts |

- Every operation is validated before any is applied; one invalid operation fails the whole
  batch with nothing written. A failed commit leaves the in-memory state unchanged.
- Operations apply in order and the last write to an id wins. Delete then memorize leaves the
  new document; memorize then delete leaves none; memorizing an id twice leaves the second; a
  repeated delete reports `existed: false`. Results come back one per operation, in order:
  chunk count for a memorize, `existed` for a delete.
- 1..=4096 operations and at most 32 MiB (33554432 bytes) of encoded journal frame, plus the
  document batch limits (8 MiB of source, 4096 chunks). Beyond any of them the call fails with a
  batch-size error naming the limit and writes nothing.
- One batch is one journal frame and one durable sync, not one per operation.
- A read-only store fails with `read_only`.
- On disk the batch is one ordinary journal frame (`HMS-TXN-2` or `HMS-TXN-3`, whichever the
  store writes) holding the mutations in order. Those frames already carry any mix of document
  and delete mutations, so no format changes and any release that reads the store reads batches.

Opening a store beside a running writer (`open_read_only`, `refresh`, `close`) is described in
[one writer, many readers](concurrency.md).

The memory API (`remember`, `recall`, `forget`, `explain`) is described in
[concepts](../concepts.md#memory-api). Stability promises per surface are in
[security](../security.md#stability-and-compatibility).

## Master key location

An encrypted store keeps its master key in `<store>/master.key` unless told otherwise. Precedence: the
explicit option, then `WARM_MASTER_KEY_PATH` (legacy `HMS_MASTER_KEY_PATH`), then the store directory.

| Surface | Option |
|---|---|
| Rust | `SecurityConfig::master_key_file` |
| Node | `masterKeyPath` in the `WarmMemorySystem` options |
| Python | `Warm(..., master_key_path=...)` |

A new store creates the file (mode 0600 on Unix, parent directories included). The path must name a
file; empty values and directories are rejected. An existing store never gets a fresh key: a missing
key, or one that does not open the store, is an error. Reopen with the same path every time.

## Document search: filters, scores, model, analysis

- **Metadata filters** take a JSON object of top-level keys, ANDed. A value is an exact match, or an
  object with `$eq` or `$in` (an array of at most 1024 scalars). A list-valued field matches when
  any element equals the value, or for `$in` is in the set; a list filter value still compares the
  whole list. Any other operator (`$gt`, `$or`, `$regex`, ...) is an error naming the operator and
  the supported set (`UnsupportedFilterOperator` in Rust; a 400 from `warm-server` and the MCP
  tools). Before this release such filters silently matched nothing.
- **`semanticScore`** is the exact cosine of the hit's stored chunk vector and the query vector,
  whenever a query vector (or a text embedding from an installed embedder) exists. It is `null`
  only without one, or for a chunk stored without a vector.
- **Embedding model**: `WarmCore::embedding_model()`, `WarmMemorySystem.embeddingModel()` and
  `Warm.embedding_model()` return `model`, `revision`, `dimensions`, `normalization`, `metric` and
  `packSha256`. `revision` hashes the model parameters including the pack, so compare it with your
  embedder's to detect a mismatch. `packSha256` is set only while an embedder is installed (the
  store does not persist it) and is otherwise `null`.
- **Analysis**: `stem(word)` and `analyze(text)` (Rust: `writerslogic_warm::{stem, analyze}` and
  `Analyzer::{stem, analyze}`; Node and Python take an optional analyzer name) return the terms the
  indexer counts, so ranking is unchanged.

## Cargo features

`default = []`: nothing is enabled unless you ask. The list is in `Cargo.toml` `[features]`.

| Feature | Turns on | Status |
|---|---|---|
| `security` | AES-256-GCM stores (default for new stores when compiled in), Argon2 passphrase keys, Ed25519 audit logs | stable |
| `provenance`, `provenance-scitt` | COSE_Sign1 statements and DID identities; opt-in SCITT submission (`ureq`, the only network code) | experimental |
| `conversation-memory` | Immutable conversation turns with source provenance | experimental |
| `server` | `warm-server` HTTP service | stable |
| `mcp` | `warm-mcp` over stdio and streamable HTTP | stable |
| `node-api` | The napi bindings published to npm | stable |
| `python` | The `warm-python` wheel (pyo3) | experimental |
| `local-embed`, `local-embed-download` | Pure-Rust embeddings from a sha256-pinned model pack; the second downloads it on first use | stable |
| `local-models` | candle Qwen3 embed, rerank and LLM from local weight directories; needs Rust 1.94 | experimental |
| `private-search` | Query-private scoring over an index someone else holds (LWE) | experimental, research |
| `experimental` and `experimental-*` | Research modules and experiment binaries | experimental, research |
| `cli`, `mimalloc` | Argument parsing for the binaries; the tuned allocator | stable |

Common builds:

```sh
cargo add warm-rust --features security
cargo install --locked --features server,security --bin warm-server warm-rust
cargo build --release --features mcp,security --bin warm-mcp     # add local-models for semantic recall
```

A change that compiles under one feature set only is not done; check the combinations CI builds.
