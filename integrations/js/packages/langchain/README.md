# @writerslogic/warm-langchain

A LangChain.js `VectorStore`, retriever support and chat message history on the WARM native engine, in process.

## Install

Source workspace only (not published); setup in [JS core](../core/README.md#install). Peer: `@langchain/core`.

## Quickstart

```js
import { Document } from '@langchain/core/documents'
import { HashingEmbedder, WarmVectorStore } from '@writerslogic/warm-langchain'

const h = new HashingEmbedder(384) // lexical; use any LangChain Embeddings for semantic search
const embeddings = { embedDocuments: (t) => h.encodeDocuments(t), embedQuery: (t) => h.encodeQuery(t) }
const store = await WarmVectorStore.create(embeddings, { path: './memory', embeddingSpace: { model: 'hashing', revision: 'v1' } })
await store.addDocuments([new Document({ pageContent: 'Rust\nOwnership and borrowing.', metadata: { topic: 'code' } })], { ids: ['rust'] })
const docs = await store.similaritySearch('borrowing', 4, { topic: 'code' })
console.log(docs.map((d) => d.id))
```

Runnable with chat history and no API key: `examples/quickstart.mjs`.

## Options

`WarmVectorStore.create(embeddings, options)`:

| Name | Type | Default | Meaning |
|---|---|---|---|
| `path` | string | current directory | Storage directory; always set it |
| `embeddingSpace` | `{model, revision}` | `langchain-embeddings` / `unversioned` | Identity of the encoder, recorded in the store; set both when persisting |
| `collection` | string | none | Namespace inside one store |
| `lexicalWeight`, `semanticWeight`, `fusion` | number, number, `blend` or `rrf` | engine defaults | Hybrid ranking |
| `titleWeight`, `bm25K1`, `bm25B` | number | engine defaults | BM25 tuning |
| `overfetch`, `maxFetch` | number | 5, 1000 | Candidate pool for client-side filters; per-query ceiling |
| `warmConfig` | object | none | Native config, e.g. encryption at rest |

## Compatibility

`@langchain/core` >=1.2.19 <2 (tested 1.2.19), `writerslogic-warm` >=0.6.5, Node 20+ (tested 26.8). 27 tests.

## Reference

[LangChain.js](https://github.com/writerslogic/warm/blob/main/docs/integrations/frameworks/langchainjs.md)
