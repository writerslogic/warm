# @writerslogic/warm-core

Shared record API of the WARM JS adapters: `remember`, `recall`, `get`, `list` and `forget` over the in-process native engine.

## Install

```sh
npm install @writerslogic/warm-core writerslogic-warm
```

From source (repository root): `npm ci --omit=optional && npm run build && npm ci --ignore-scripts --prefix integrations/js && npm run build --prefix integrations/js`.

## Quickstart

```js
import { WarmStore } from '@writerslogic/warm-core'

const store = new WarmStore({ path: './memory' })
await store.remember([{ id: 'a', text: 'Ada wrote the first program', metadata: { topic: 'history' } }])
const hits = await store.recall('first program', { k: 3, filter: { topic: 'history' } })
console.log(hits.map((h) => [h.id, h.score]))
await store.close()
```

## Options

`new WarmStore(options)`:

| Name | Type | Default | Meaning |
|---|---|---|---|
| `path` | string | current directory | Storage directory; always set it |
| `dimensions` | number | 4096 | Hypervector dimensions of a created store |
| `embedder` | `Embedder` | `HashingEmbedder(384)` | Encoder (lexical by default) |
| `collection` | string | none | Namespace for ids and queries |
| `maxFetch` | number | 1000 | Ceiling on passages per query |
| `warmConfig` | object | none | Native config, e.g. encryption at rest |

More (BM25, fusion, chunking, `rerank`): see the reference.

## Compatibility

`writerslogic-warm` >= 0.6.5 (peer), Node 20+ (tested 26.8), TypeScript 5.9. 62 tests.

## Reference

[JS core](https://github.com/writerslogic/warm/blob/main/docs/integrations/frameworks/js-core.md)
