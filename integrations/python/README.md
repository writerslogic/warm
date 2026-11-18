# writerslogic-warm-integrations

Adapters that put WARM behind the memory, vector-store and retriever interfaces of Python agent and RAG frameworks, over the `warm-server` HTTP service.

## Install

Framework dependency trees conflict: install exactly one extra per environment.

```sh
pip install "writerslogic-warm-integrations[langchain]"   # or another extra from the table
warm-server --input-dim 384                               # the vector length your embeddings produce
```

## Quickstart

```python
from langchain_core.documents import Document
from writerslogic_warm_integrations.core import HashingEmbedder
from writerslogic_warm_integrations.langchain import EmbedderAsLangChain, WarmVectorStore

store = WarmVectorStore(EmbedderAsLangChain(HashingEmbedder(384)), url="http://127.0.0.1:8080", api_key="dev", tenant_id="acme", collection="docs")
store.add_documents([Document(page_content="Rust has ownership and borrowing.", metadata={"topic": "code"})], ids=["rust"])
print(store.similarity_search("borrowing", k=4, filter={"topic": "code"}))
```

Run `warm-server --no-auth --input-dim 384` (or set `WARM_API_KEY=dev` and pass `api_key="dev"`) first. `HashingEmbedder` is lexical and offline; pass a real embedding model for semantic search. One runnable offline example per adapter is in `examples/`.

## Options

Every adapter accepts the same WARM settings as keyword arguments or `config=WarmConfig(...)`:

| Name | Type | Default | Meaning |
|---|---|---|---|
| `url`, `api_key`, `timeout`, `transport` | str, str, float, httpx transport | local server | Connection to `warm-server` |
| `tenant_id` | str | default tenant | Separate store per tenant |
| `collection` | str | none | Namespace inside a tenant |
| `embedder` / the framework's embeddings | `Embedder` | `HashingEmbedder(input_dim or 384)` | Encoder; `text_queries=True` lets a model-enabled server embed text |
| `max_fetch` | int | 100 | Result ceiling; match the server's `--max-top-k` |
| `overfetch` | int | 5 | Accepted for compatibility |

## Adapters

| Extra | Host (tested version) | Page |
|---|---|---|
| `langchain` | langchain-core 1.6.12 | [LangChain](../../docs/integrations/frameworks/langchain.md) |
| `langgraph` | langgraph 1.2.16 | [LangGraph](../../docs/integrations/frameworks/langgraph.md) |
| `llamaindex` | llama-index-core 0.14.25 | [LlamaIndex](../../docs/integrations/frameworks/llamaindex.md) |
| `haystack` | haystack-ai 3.4.0 | [Haystack](../../docs/integrations/frameworks/haystack.md) |
| `dspy` | dspy 3.4.1, 3.5.0 | [DSPy](../../docs/integrations/frameworks/dspy.md) |
| `openai-agents` | openai-agents 0.24.0 | [OpenAI Agents](../../docs/integrations/frameworks/openai-agents.md) |
| `pydantic-ai` | pydantic-ai-slim 2.58.0 | [Pydantic AI](../../docs/integrations/frameworks/pydantic-ai.md) |
| `smolagents` | smolagents 1.26.0 | [smolagents](../../docs/integrations/frameworks/smolagents.md) |
| `litellm` | litellm 1.103.10 | [LiteLLM](../../docs/integrations/frameworks/litellm.md) |
| `crewai` | crewai 1.15.28 | [CrewAI](../../docs/integrations/frameworks/crewai.md) |
| `autogen` | autogen 0.7.5 | [AutoGen](../../docs/integrations/frameworks/autogen.md) |
| `ag2` | ag2 1.1.2 | [AutoGen and AG2](../../docs/integrations/frameworks/autogen.md) |
| `semantic-kernel` | semantic-kernel 1.45.0 | [Semantic Kernel](../../docs/integrations/frameworks/semantic-kernel.md) |
| `mem0` | mem0ai 2.2.1 | [mem0](../../docs/integrations/frameworks/mem0.md) |
| `letta` | letta-client 1.12.1 | [Letta](../../docs/integrations/frameworks/letta.md) |

## Compatibility

Python 3.11+ (tested 3.12), `warm-sdk` >= 0.1.3, `warm-server` of the same WARM release. Tests: `WARM_SERVER_BIN=/path/to/warm-server ./run-matrix.sh [extra ...]` (one isolated environment per extra).

## Reference

[Integrations overview](../../docs/integrations/index.md), [verification matrix and shared limits](../../docs/integrations/frameworks/MATRIX.md).
