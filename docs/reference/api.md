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

The memory API (`remember`, `recall`, `forget`, `explain`) is described in
[concepts](../concepts.md#memory-api). Stability promises per surface are in
[security](../security.md#stability-and-compatibility).

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
