<img src="https://raw.githubusercontent.com/writerslogic/warm/main/assets/logo/warm-logo.png" alt="WARM logo" width="96" align="left">

# WARM

**Local memory for AI agents and applications.**

WARM (WritersLogic Adaptive Retention & Memory) gives agents persistent memory, full-text and
vector search, and source provenance in one embeddable Rust engine. Use it inside your application,
through HTTP, or from an MCP client, with storage on infrastructure you control.

[![CI](https://img.shields.io/github/actions/workflow/status/writerslogic/warm/ci.yml?branch=main&label=CI)](https://github.com/writerslogic/warm/actions/workflows/ci.yml)
![MSRV 1.89](https://img.shields.io/badge/MSRV-1.89-informational.svg)
[![OpenSSF Scorecard](https://img.shields.io/ossf-scorecard/github.com/writerslogic/warm)](https://securityscorecards.dev/viewer/?uri=github.com/writerslogic/warm)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/13977/badge)](https://www.bestpractices.dev/projects/13977)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![WARM MCP server: quality and maintenance score on Glama](https://glama.ai/mcp/servers/writerslogic/warm/badges/score.svg)](https://glama.ai/mcp/servers/writerslogic/warm)


[Website](https://writerslogic.com/warm) ·
[Docs](https://writerslogic.com/warm/docs/) ·
[Get started](#get-started) ·
[Standards and provenance](docs/reference/standards.md) ·
[Evaluation](#evaluation) ·
[Capabilities](#what-ships-today) ·
[Limits](#limits) ·
[Contribute](CONTRIBUTING.md)

<br clear="left">

## What it is

WARM is a Rust library that stores text, vectors and facts in a
directory on your machine and finds them again. It ships with optional Node.js, C, WASM and UniFFI
bindings, an HTTP server with portable JavaScript and Python clients, and an MCP server.
The embedded engine makes no service calls or telemetry requests: the `offline` job in [ci.yml](.github/workflows/ci.yml) runs the default-feature test suite in a network namespace without external interfaces and checks that the default dependency tree has no HTTP, TLS or socket client. HTTP clients connect to the
server you configure; model stages are opt-in. Capabilities and limits: [What ships today](#what-ships-today).

## Who it's for

| You are building | What you get |
|------------------|--------------|
| An agent or assistant that needs memory | A memory API with forgetting that removes facts from the store, and an MCP server for editors and agent hosts |
| A local-first or offline app | Embedded search and storage in one directory, with no server process and optional encryption |
| A service that needs vector or text search | `warm-server` over HTTP with tenants and scoped keys, or the library in-process |

## Get started

| You are using | Install | Next |
|---|---|---|
| An editor or agent host (MCP) | `npx -y writerslogic-warm-mcp` (or `uvx warm-mcp`; add `--transport http` for `http://127.0.0.1:8765/mcp`) | [MCP integration](docs/integrations/mcp/README.md) |
| Rust | `cargo add warm-rust` | below |
| Node, embedded engine | `npm install writerslogic-warm@0.6.8` | below |
| JavaScript or TypeScript, HTTP client | `npm install warm-sdk@0.2.1` | [client API](bindings/js/warm-sdk/README.md) |
| Python, HTTP client | `uv pip install warm-sdk` | [Python quickstart](examples/quickstart/python/quickstart.py) |
| A server for the HTTP clients | `cargo install --locked --features server --bin warm-server warm-rust` | [server docs](https://writerslogic.com/warm/docs/latest/server/) |

All options, source builds, version requirements and the HTTP-client-or-embedded choice:
[Install](docs/get-started/install.md) and [Choose an integration](docs/get-started/choose.md).
WARM runs locally or on your own infrastructure. This repository contains no hosted service.
WritersLogic sells WARM Sync, a separate encrypted multi-device sync service with its own client
package; the engine here does not contact it.

To try the offline memory demo from source:

```sh
git clone https://github.com/writerslogic/warm
cd warm
cargo run --release --example demo-offline-agent
```

**Rust.** The crate is `warm-rust`; the library is imported as `writerslogic_warm`. MSRV is Rust 1.89; `local-models` and `--all-features` need 1.94 or newer.

```rust quickstart-check
use writerslogic_warm::{DocumentInput, SearchOptions, WarmCore};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A store is a directory; it persists there and reopens with the same arguments.
    let dir = std::env::temp_dir().join(format!("warm-quickstart-{}", std::process::id()));
    let warm = WarmCore::new(4096, Some(dir.display().to_string()), None)?;
    warm.memorize_document(DocumentInput {
        id: "backups".into(),
        text: "Restore deleted documents from verified backups.".into(),
        source_uri: Some("operations.md".into()),
        version: Some("1".into()),
        metadata: None,
        chunk_words: None, overlap_words: None, store_text: None, embeddings: None, title_weight: None,
    })?;
    let hits = warm.search_documents("restore backups", &SearchOptions::default())?;
    println!("{} {:?}", hits[0].document_id, hits[0].source_uri);
    drop(warm);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
```

**Node.js** (`writerslogic-warm`; the portable client is a separate package with a different API; `listDocuments` supports scoped pruning, see the [integration guide](docs/integrations/organizational-knowledge.md)):

```js quickstart-check
const { WarmMemorySystem } = require('writerslogic-warm')
const { DocumentMemory } = require('writerslogic-warm/semantic')

async function main() {
  const dir = require('node:fs').mkdtempSync(require('node:path').join(require('node:os').tmpdir(), 'warm-readme-'))
  const memory = new DocumentMemory(new WarmMemorySystem(4096, dir))
  await memory.memorize({
    id: 'backups', text: 'Restore deleted documents from verified backups.',
    sourceUri: 'operations.md', version: '1', metadata: { project: 'alpha' },
  })
  const [hit] = await memory.search('restore backups', { filter: { project: 'alpha' } })
  console.log(hit.documentId, hit.sourceUri, hit.text)
  await memory.flush()
}
main().catch(error => { console.error(error); process.exit(1) })
```

The [five-minute quickstart](https://writerslogic.com/warm/docs/latest/quickstart/) adds a SQLite
FTS5 comparison and encryption in Rust, Python and Node; CI runs that code on every pull request.

<a id="headline-results"></a>
<a id="where-its-behind"></a>

## Evaluation

[Recorded results and limitations](docs/benchmarks/EVALUATION.md) cover memory quality, text
retrieval and vector search. Each result retains its workload, hardware, model, source revision
and known losses. The [full comparison and reproduction protocol](docs/benchmarks/README.md)
provide the evidence for evaluating WARM on your own workload.

## What ships today

| Capability | Status | Documented limit |
|------------|--------|------------------|
| Hybrid document search (BM25 + exact cosine, filters, byte offsets) | Ships | Exact scan of eligible chunks; benchmark your corpus before assuming capacity ([guide](docs/production-readiness.md)) |
| Boolean lexical search (`+required`, `-excluded`, quoted phrases) | Ships | Flat queries only, no nesting; an empty or only-excluded query matches nothing ([design](docs/benchmarks/lexical/DESIGN.md)) |
| Atomic replace/delete, crash-safe journal, verified compaction, process lock | Ships | One writer per store; compaction is not secure erasure |
| Vector index (`qgraph` vertex graph, memory-mapped) | Ships | Builds slower than hnswlib at M=16 and M=32 on SIFT-1M on a Core Ultra 9 185H laptop (same session); faster on a GCP Ice Lake VM, where the WARM build is a separate-session rerun ([results](docs/benchmarks/EVALUATION.md)); behind ScaNN and faiss-ivfpqfs on single-core ann-benchmarks at recall 0.95 for WARM revision `19f4371`: SIFT 2,819 against 3,787 QPS ([`ann_benchmarks_harness_wl02_v1.json`](benchmarks/results/ann_benchmarks_harness_wl02_v1.json)); that revision is superseded and the competitors have not been rerun against the current one |
| Updatable vector index (durable delta for inserts and deletes) | Ships | Single writer; format in [VECTOR-UPDATES.md](docs/file-format/VECTOR-UPDATES.md) |
| Query-aware graph routing (trained on sample queries) | Ships (opt-in) | Needs representative training queries; tested on VIBE llama and yi ([report](benchmarks/results/query_aware_graph_v1/README.md)) |
| Memory API: remember, recall, forget, explain | Ships | Forgetting removes a fact from the store; it does not cover what a model inferred ([docs](docs/memory-api/README.md)) |
| Conversation memory (`conversation-memory`) | Ships | Bounded context with exact source provenance; native recall not yet benchmarked on LongMemEval end to end (the LongMemEval result in [EVALUATION.md](docs/benchmarks/EVALUATION.md) is a research pipeline with external Qwen3 models, not this API) |
| Meaning memory: sparse hypervectors with bind, bundle and permute, triples, multi-hop, Hopfield cleanup | Ships (runtime opt-in) | Tested on a set of 2,000 synthetic plus 120 real facts ([`research_bench_16384_256.json`](benchmarks/results/research_bench_16384_256.json)) |
| Union-bundle membership (`experimental`) | Research | At D = 16,384: positive gap to N = 500 (4% false positives), tested to 2,500 ([`benchmark_scaling_results.json`](benchmarks/results/benchmark_scaling_results.json)) |
| Vector API (`memorize_vector`, `query_vector`) | Ships | Backed by the updatable vertex graph; glove-100 recall@10 0.859 at 2,092 QPS on one thread of an Apple M4 under load, on or below the hnswlib and FAISS HNSW curves in the same file, with a 478 s build ([`public_glove-100-angular.json`](benchmarks/results/public_glove-100-angular.json), `hms` row). Vectors written before the graph existed stay on the sparse path until re-memorized; not built for encrypted stores |
| Encrypted stores (`security`) | Ships | AES-256-GCM, Argon2id; covers the log, index caches and the text segment. Metadata, file names and sizes are not encrypted |
| Signed audit log, credential-gated access, COSE_Sign1 over Ed25519, `did:key`/`did:jwk` identities, SCITT-style signed statements (`provenance`) | Ships | Host must authenticate DIDs; PoSME, RATS, `did:web` not implemented. Tests: `core::provenance::cose::tests::sign_verify_roundtrip`, `core::provenance::did::tests::did_key_roundtrip`, `tests/scitt_receipt_interop.rs`. Manifests use C2PA-style labels and are not C2PA Content Credentials ([standards](docs/reference/standards.md)) |
| On-device models (`local-models`: Qwen3 embed/rerank/LLM) | Ships | Local weights only; never downloads |
| `warm-server` HTTP API, multi-tenant (`server`) | Ships | Single node; TLS, per-tenant scoped keys ([guide](docs/SERVER.md)) |
| MCP server (`mcp`: `warm-mcp`, stdio and HTTP) | Ships | Encryption needs `security`; semantic recall needs `local-models` |
| JavaScript/TypeScript client (`warm-sdk` 0.2.1) | Source and release builds | Portable HTTP client; requires a separately running server |
| Embedded Node.js (`writerslogic-warm` 0.6.8) | Source and release builds | Eight optional native platform packages; separate from the client |
| C, Go, .NET, Java, Kotlin, Swift and WebAssembly bindings | Source available | Require local builds; the language packages and native bundles are not yet published ([runtime guides](docs/runtimes)) |
| Python | HTTP client (`warm-sdk`); native vector indexes and phase vectors (`warm-python`) | No in-process document search |
| Query-private search (`private-search`) | Experimental | For searching an index someone else holds; hides the query vector from the scorer only ([notes](docs/PRIVATE-SEARCH.md)) |

Optional engine capabilities use Cargo features or runtime configuration;
[Cargo features](docs/reference/features.md) lists the build choices.

The [compatibility policy](docs/STABILITY.md) defines API stability, deprecation windows and
storage-format migration rules.

## Architecture

Diagram and module map: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Limits

| Limit | Detail |
|---|---|
| Not anonymous | Sketches and term counts are not encryption; `storeText: false` keeps term counts, IDs and embeddings and is not a privacy control. Client-side vector masking is obfuscation. [Privacy limits](docs/concepts/privacy-limits.md), [PRIVACY.md](docs/PRIVACY.md) |
| Document search is conventional | It does not use the hypervectors; the holographic parts are the algebra and the meaning memory. [Concepts](docs/concepts/holographic.md) |
| One writer per store | A second writing process fails at open with a lock error. A store is a directory; `WarmCore::new(.., None, ..)` uses the current directory |
| Capacity numbers are points | Stated only over the tested range. [Capacity](https://writerslogic.com/warm/docs/latest/concepts/capacity/) |
| Benchmarks are single machines | Conditions are in each results file; the single-core ann-benchmarks run and the VIBE high-recall points are losses ([Where it's behind](docs/benchmarks/EVALUATION.md#where-its-behind)) |

Report vulnerabilities privately via [SECURITY.md](SECURITY.md).

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md): setup, the checks CI runs, DCO sign-off (`git commit -s`, no
CLA). Start with a [good first issue](https://github.com/writerslogic/warm/labels/good%20first%20issue)
or [Discussions](https://github.com/writerslogic/warm/discussions). See the
[Code of Conduct](CODE_OF_CONDUCT.md). Benchmark changes regenerate the results file they touch;
the README cites files, not remembered figures.

## License

Apache-2.0. See [LICENSE](LICENSE). Use, modify and embed it in open-source or proprietary software, including network services, with no copyleft obligation. The historical [v0.6.1 source](https://github.com/writerslogic/warm/tree/v0.6.1) remains available under its [AGPL-3.0-or-later license](https://github.com/writerslogic/warm/blob/v0.6.1/LICENSE).
