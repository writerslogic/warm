# @writerslogic/warm-llamaindex

A LlamaIndex.TS `BaseVectorStore` and `BaseKVStore` on the WARM native engine, in process.

## Install

```sh
npm install @writerslogic/warm-llamaindex llamaindex @llamaindex/core
```

The native engine `writerslogic-warm` is a peer that npm 7+ installs for you; `@writerslogic/warm-core` comes as a dependency. From source: [JS core](../core/README.md#install).

## Quickstart

```js
import { Document, Settings, VectorStoreIndex, storageContextFromDefaults } from 'llamaindex'
import { BaseEmbedding } from '@llamaindex/core/embeddings'
import { HashingEmbedder, WarmVectorStore } from '@writerslogic/warm-llamaindex'

const h = new HashingEmbedder(384) // lexical; use any LlamaIndex embedding for semantic search
class HashEmbedding extends BaseEmbedding { async getTextEmbedding(t) { return h.encodeQuery(t) } }
const embedModel = (Settings.embedModel = new HashEmbedding())
const vectorStore = await WarmVectorStore.create({ path: './memory', embedModel, embeddingSpace: { model: 'hashing', revision: 'v1' } })
const storageContext = await storageContextFromDefaults({ vectorStore })
const index = await VectorStoreIndex.fromDocuments([new Document({ text: 'Rust has ownership and borrowing.' })], { storageContext })
console.log((await index.asRetriever({ similarityTopK: 1 }).retrieve('borrowing'))[0].node.text)
```

Runnable with a mock LLM, filters and KV store: `examples/quickstart.mjs`.

## Options

`WarmVectorStore.create(options)`:

| Name | Type | Default | Meaning |
|---|---|---|---|
| `embedModel` | `BaseEmbedding` | none | Index embedding model; without it pass `embeddingDimensions` (384) |
| `embeddingSpace` | `{model, revision}` | `llamaindex-embeddings` / `unversioned` | Identity of the encoder, recorded in the store; set both when persisting |
| `path` | string | current directory | Storage directory; always set it |
| `collection` | string | none | Namespace inside one store |
| `lexicalWeight`, `semanticWeight`, `fusion` | number, number, `blend` or `rrf` | engine defaults | Ranking for `hybrid` queries |
| `overfetch`, `maxFetch`, `mmrPrefetchFactor` | number | 5, 1000, 4 | Filter candidate pool; per-query ceiling; MMR prefetch |
| `warmConfig` | object | none | Native config, e.g. encryption at rest |

## Compatibility

`llamaindex` 0.12.1 with `@llamaindex/core` >=0.6.22 <0.7 (keep one copy), `writerslogic-warm` >=0.6.5, Node 20+ (tested 26.8). 20 tests.

## Reference

[LlamaIndex.TS](https://github.com/writerslogic/warm/blob/main/docs/integrations/frameworks/llamaindex-ts.md)
