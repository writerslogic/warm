---
description: Choose between portable HTTP clients, the embedded Rust or Node engine, and MCP for editor memory.
search:
  boost: 2
---

# Choose an integration

Start with where your application runs and who should own the store.

| Your task | Start here | Where the engine runs |
|---|---|---|
| Give an editor or coding agent persistent memory | [MCP and editor setup](../integrations/editors/README.md) | A local `warm-mcp` process, or a shared MCP HTTP server |
| Embed document search in a Rust application | [Rust quickstart](../quickstart/index.md) | Inside your application |
| Add memory to JavaScript or TypeScript | [Portable `warm-sdk` client](install.md) | A separately running HTTP server |
| Embed document search in Node | [Optional `writerslogic-warm` engine](install.md) | A native Rust module inside Node |
| Access a store from Python or several processes | [Python SDK and HTTP server](../SERVER.md) | A server you run; Python is the client |
| Search vectors or experiment with phase hypervectors in Python | [Python runtime](../api/python/vsa.md) | The `warm-python` extension |

## Which package?

| Package | Imports as | Runs the engine | Needs | Choose it for |
|---|---|---|---|---|
| `warm-sdk` 0.2.2 (npm) | `WarmClient` | No: HTTP client for `warm-server` | Node 18+, Fetch and Web Streams; no native dependencies | Portable JavaScript and TypeScript; does not start the server |
| `writerslogic-warm` 0.7.2 (npm) | `WarmMemorySystem`; `/semantic`, `/memory`, `/vectra` | Yes, inside Node | Native module (optional platform packages) | Embedded engine in Node; install it explicitly |
| `warm-sdk` (PyPI) | `writerslogic_warm_sdk` | No: HTTP client for `warm-server` | A running server; same API key, tenant and embedding dimensions on both sides | Python or several processes sharing a store; does not start a server or download a model |
| `warm-python` (PyPI) | `writerslogic_warm` | Yes, in-process | Native wheel | Native vector search and phase-vector algebra; `Warm` (document search, memory API) for in-process use |

The two JavaScript packages are separate, not subpaths of one SDK. Versions, source builds and
`warm-python` availability: [Install](install.md).

## Which retrieval path?

| You have | Use | Notes |
|---|---|---|
| Text only | Document search | Stemmed BM25 needs no model. |
| Text and embeddings you supply | Hybrid document search | Fuses BM25 with exact cosine. |
| Facts that change | [Memory API](../memory-api/README.md) | Assertions, replacements, history. |
| An editor or agent host | MCP (`remember`, `recall`, `search`, ...) | Read the [MCP tool reference](../integrations/mcp/README.md) before enabling write operations. |

A similarity score is a ranking signal, not a probability that a statement is true. Keep source
text and inspect returned evidence before using it in an answer.

## Who opens the store, and what stays private

- An embedded application owns its local store. Multiple clients connect to one server instead
  of opening the same writable directory in independent processes.
- MCP offers stdio for a single client and HTTP for shared access; its namespace settings
  determine which memories clients share.
- The `security` feature supports encrypted stores. The Python HTTP server has different storage
  guarantees ([server limitations](../SERVER.md)).
- Embeddings, sketches and vector masking are not encryption: see the
  [privacy limits](../concepts/privacy-limits.md) before choosing a deployment.

Next: [install](install.md), then the [quickstart](../quickstart/index.md).
