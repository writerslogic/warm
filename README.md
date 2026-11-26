<img src="https://raw.githubusercontent.com/writerslogic/warm/main/assets/logo/warm-logo.png" alt="WARM logo" width="96" align="left">

# WARM

**Local memory and search for AI agents and applications.**

[![CI](https://img.shields.io/github/actions/workflow/status/writerslogic/warm/ci.yml?branch=main&label=CI)](https://github.com/writerslogic/warm/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/warm-rust)](https://crates.io/crates/warm-rust)
[![npm](https://img.shields.io/npm/v/warm-sdk?label=npm)](https://www.npmjs.com/package/warm-sdk)
[![PyPI](https://img.shields.io/pypi/v/warm-sdk)](https://pypi.org/project/warm-sdk/)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![OpenSSF Scorecard](https://img.shields.io/ossf-scorecard/github.com/writerslogic/warm)](https://securityscorecards.dev/viewer/?uri=github.com/writerslogic/warm)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/13977/badge)](https://www.bestpractices.dev/projects/13977)

[Website](https://writerslogic.com/warm) ·
[Docs](https://writerslogic.com/warm/docs/) ·
[Quickstart](docs/quickstart.md) ·
[Contribute](CONTRIBUTING.md)

<br clear="left">

WARM (WritersLogic Adaptive Retention & Memory) is a Rust engine that stores text, vectors and facts
in a directory on your machine and finds them again. It provides stemmed BM25 and exact-cosine hybrid
document search, a graph vector index, a memory API (`remember`, `recall`, `forget`, `explain`) and a
vector-symbolic fact store. Use it inside your application, over HTTP, or from an MCP client. The
library makes no network calls and sends no telemetry. Document search is conventional hybrid
retrieval; the holographic part is the vector-symbolic layer ([concepts](docs/concepts.md)).

## Install

| You are using | Install |
|---|---|
| Rust | `cargo add warm-rust` (add `--features security` for encrypted stores) |
| Node, embedded engine | `npm install writerslogic-warm` |
| JavaScript or TypeScript, HTTP client | `npm install warm-sdk` |
| Python, embedded engine | `uv pip install warm-python` |
| Python, HTTP client | `uv pip install warm-sdk` |
| HTTP server for the clients | `cargo install --locked --features server,security --bin warm-server warm-rust` |
| MCP client or editor | `npx -y writerslogic-warm-mcp` or `uvx warm-mcp` |

Rust 1.89 or newer builds the crate; `local-models` and `--all-features` need 1.94. Nothing is
enabled by default (`default = []`); the features are listed in the
[API overview](docs/reference/api.md#cargo-features).

## Quickstart

Rust. The crate is `warm-rust`; the library is imported as `writerslogic_warm`.

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

Node.js, embedded engine:

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

Python, against a running `warm-server` (`WARM_API_KEY=change-me warm-server --input-dim 384`):

```python
from writerslogic_warm_sdk import WarmClient

client = WarmClient(url="http://127.0.0.1:8080", api_key="change-me", tenant_id="demo")
client.memory.remember_many([{"id": "backups", "text": "Restore deleted documents from verified backups."}])
print(client.memory.recall("restore backups", k=3))
```

The [quickstart](docs/quickstart.md) covers the JavaScript client, MCP and facts that change over
time. Runnable versions with a SQLite FTS5 comparison are in
[`examples/quickstart`](examples/quickstart).

## Benchmarks

Results are generated by `./benchmarks/competitive/warm-bench table` from the committed files in
[`benchmarks/results/competitive/`](benchmarks/results/competitive) into
[docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md), with every cell indexed to its
source file in [comparison.cells.json](docs/benchmarks/comparison.cells.json). Rows are never
compared across hardware classes, and a win or loss needs a bootstrap interval that excludes
parity. Two single-machine examples from that table (SIFT-1M, 8-core Ice Lake x86_64 VM, recall@10 at
least 0.95): WARM 14,645 QPS against hnswlib 7,427, and p99 96 microseconds against 194. The same
table records where WARM is behind, for example index size on GloVe-100 and text build memory against
SQLite FTS5. See [benchmarks](docs/benchmarks/index.md) for the protocol and the losses.

## Bindings and integrations

| Surface | Package or path |
|---|---|
| Rust | `warm-rust` |
| Embedded Node | `writerslogic-warm` |
| JavaScript and TypeScript client | `warm-sdk` (npm), `bindings/js/warm-sdk` |
| Python client | `warm-sdk` (PyPI), `bindings/python/warm-sdk` |
| Python native | `warm-python` |
| MCP server | `writerslogic-warm-mcp` (npm), `warm-mcp` (PyPI), `integrations/mcp` |
| Python frameworks | LangChain, LangGraph, LlamaIndex, Haystack, DSPy, OpenAI Agents, Pydantic AI, smolagents, LiteLLM, CrewAI, AutoGen, Semantic Kernel, mem0, Letta (`integrations/python`) |
| JavaScript frameworks | LangChain.js, LlamaIndex.TS, Mastra, OpenAI Agents JS, Vercel AI SDK (`integrations/js`) |
| C, Go, Java, .NET, Swift, Kotlin, WebAssembly | `bindings/` (source builds) |
| Importers | `warm-import` for Chroma, Qdrant, Pinecone, pgvector, FAISS and others |

Details and tested versions: [integrations](docs/integrations.md).

## Documentation

[Quickstart](docs/quickstart.md) ·
[Concepts](docs/concepts.md) ·
[API overview](docs/reference/api.md) ·
[Integrations](docs/integrations.md) ·
[Server](docs/server/index.md) ·
[File format](docs/file-format/index.md) ·
[Benchmarks](docs/benchmarks/index.md) ·
[Security and stability](docs/security.md) ·
[FAQ](docs/reference/faq.md) ·
[Changelog](CHANGELOG.md)

## Limits

- One writer per store; a second writing process fails at open with a lock error.
- Sketches and term counts are not encryption, and `storeText: false` is not a privacy control
  ([security](docs/security.md)).
- Capacity numbers are sweeps over a tested range, not guarantees.
- Benchmarks are single machines; conditions are in each results file.

Report vulnerabilities privately through [SECURITY.md](SECURITY.md).

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) covers setup, the checks CI runs and DCO sign-off
(`git commit -s`, no CLA). See the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Apache-2.0. See [LICENSE](LICENSE). Use, modify and embed it in open-source or proprietary software, including network services, with no copyleft obligation. The historical [v0.6.1 source](https://github.com/writerslogic/warm/tree/v0.6.1) remains available under its [AGPL-3.0-or-later license](https://github.com/writerslogic/warm/blob/v0.6.1/LICENSE).
