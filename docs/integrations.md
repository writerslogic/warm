# Integrations

Adapters that put WARM behind the interfaces of agent frameworks, editors and Claude, the language
bindings, and the importers. Source and per-package READMEs are under
[`integrations/`](https://github.com/writerslogic/warm/tree/main/integrations) and
[`bindings/`](https://github.com/writerslogic/warm/tree/main/bindings). "Tested" means the adapter's
test suite passes against the listed host version; it is not a claim about other versions.

Python frameworks reach `warm-server` through `warm-sdk`; JavaScript frameworks run the native
engine in process; editors and the Claude memory tool speak MCP to `warm-mcp`. Install one Python
framework extra per environment, because their dependency trees conflict:
`pip install "warm-integrations[<extra>]"`.

## Python frameworks

Python 3.11+ (tested on 3.12), `warm-sdk` >= 0.1.3.

| Adapter | Host and tested version | Install extra | Status |
|---|---|---|---|
| LangChain | langchain-core 1.6.12 | `langchain` | tested |
| LangGraph | langgraph 1.2.16 | `langgraph` | tested |
| LlamaIndex | llama-index-core 0.14.25 | `llamaindex` | tested |
| Haystack | haystack-ai 3.4.0 | `haystack` | tested |
| DSPy | dspy 3.4.1 and 3.5.0 | `dspy` | tested |
| OpenAI Agents | openai-agents 0.24.0 | `openai-agents` | tested |
| Pydantic AI | pydantic-ai-slim 2.58.0 | `pydantic-ai` | tested |
| smolagents | smolagents 1.26.0 | `smolagents` | tested |
| LiteLLM | litellm 1.103.10 | `litellm` | tested |
| CrewAI | crewai 1.15.28 | `crewai` | tested |
| AutoGen | autogen 0.7.5 | `autogen` | tested |
| AG2 | ag2 1.1.2 | `ag2` | tested (tools only) |
| Semantic Kernel | semantic-kernel 1.45.0 | `semantic-kernel` | tested |
| mem0 | mem0ai 2.2.1 | `mem0` | tested (shim) |
| Letta | letta-client 1.12.1 | `letta` | tested (no live Letta server) |

## JavaScript and TypeScript

Native engine in process; Node 20 or newer, `warm-memory` >= 0.6.5.
Standalone examples of the embedded `WarmMemorySystem` surface (search, triplets, sequences,
analogies, external embeddings) are listed in [examples/README.md](https://github.com/writerslogic/warm/blob/main/examples/README.md).

| Adapter | Host and tested version | Package | Status |
|---|---|---|---|
| Core | none | `warm-core` | tested |
| LangChain.js | @langchain/core 1.2.19 | `warm-langchain` | tested |
| LlamaIndex.TS | llamaindex 0.12.1 | `warm-llamaindex` | tested |
| Mastra | @mastra/core 1.78.0 | `warm-mastra` | tested |
| OpenAI Agents JS | @openai/agents 0.20.0 | `warm-openai-agents` | tested |
| Vercel AI SDK | ai 7.0.146 | `warm-vercel-ai` | tested |

## MCP, editors and Claude

All of these speak to `warm-mcp`.

| Adapter | Host | Install | Status |
|---|---|---|---|
| warm-mcp launcher | any MCP client | `npx -y warm-mcp` or `uvx warm-mcp` | tested |
| Claude memory tool | Anthropic `memory_20250818` (Python, TypeScript) | `pip install warm-claude-tool` or `npm install warm-claude-tool` | tested |
| Claude Code | plugin | `claude plugin install` from the repo marketplace | plugin validated, headless session run |
| Codex CLI | 0.163.0 | `codex mcp add` | CLI verified, no model session |
| Gemini CLI | 0.64.0 | `gemini mcp add` or extension | CLI verified, no model session |
| VS Code | 1.141.0 | `code --add-mcp` | config verified |
| Cursor, Windsurf, Zed, Cline, Continue, Claude Desktop | current docs | copy the config from `integrations/editors/<host>/` | config-validated only |

## Language bindings

| Binding | Source | Status |
|---|---|---|
| Rust | `warm-rust` on crates.io | published |
| Embedded Node | `warm-memory` on npm | published; one package with prebuilt binaries for eight platforms |
| JavaScript and TypeScript client | `warm-sdk` on npm, `bindings/js/warm-sdk` | published; needs `warm-server` |
| Python client | `warm-sdk` on PyPI, `bindings/python/warm-sdk` | published; needs `warm-server` |
| Python native | `warm-python` on PyPI | published; in-process `Warm` class from 0.7.0 |
| C | `bindings/c` | stable ABI (`hms1_*`); local build |
| Go, Java, .NET | `bindings/go`, `bindings/java`, `bindings/dotnet` | wrappers over the C ABI; local build |
| Swift, Kotlin | `bindings/swift`, `bindings/kotlin` (UniFFI) | experimental; local build |
| WebAssembly | `bindings/wasm` | experimental; local build |

## Importers

`warm-import` (installed with `warm-sdk[import-<source>]`) copies records from another store into a
`warm-server`. It streams, resumes from a checkpoint, is idempotent, and ends with a fidelity
report. Start with `--dry-run`; `warm-import list` prints the sources and
`warm-import <source> --help` lists each source's options and credential variable.

```sh
uv add 'warm-sdk[import-chroma]'
export WARM_API_KEY=...
warm-import chroma --path ./chroma-db --warm-url http://127.0.0.1:8080 --tenant acme --report migration.json
```

Vector handling is chosen with `--vectors`: `auto` (carry source vectors of the server dimension, encode text otherwise), `carry` (source
vectors verbatim), `reencode` (use the server's model), `project` (seeded random projection to the
server dimension) or `none` (text only).

| Kind | Sources |
|---|---|
| Vector databases | Chroma, Qdrant, Pinecone, Weaviate, Milvus, pgvector, Redis, MongoDB, LanceDB, Vespa |
| Search engines | Elasticsearch, OpenSearch |
| Indexes and files | FAISS, hnswlib, Annoy, sqlite-vec, plain files, Markdown |
| Frameworks and memory systems | LangChain, LlamaIndex, mem0, Letta, Zep, OpenAI vector store |

Other migrations: the `vectra` selector in the embedded Node package reads a Vectra index
(`vectra.d.ts`), and `integrations/organizational-knowledge` imports curated multi-application
documents.
