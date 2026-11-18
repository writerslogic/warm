# @writerslogic/warm-mastra

`WarmVector`, a Mastra `MastraVector` on the WARM native engine, in process. Plugs into Mastra Memory semantic recall and RAG tools.

## Install

Source workspace only (not published); setup in [JS core](../core/README.md#install). Peer: `@mastra/core`.

## Quickstart

```js
import { WarmVector } from '@writerslogic/warm-mastra'

const vector = new WarmVector({ id: 'warm', dimension: 4, path: './memory' })
await vector.createIndex({ indexName: 'notes', dimension: 4 })
await vector.upsert({ indexName: 'notes', vectors: [[1, 0, 0, 0], [0, 1, 0, 0]], metadata: [{ text: 'cats', topic: 'pets' }, { text: 'rockets', topic: 'space' }], ids: ['cats', 'rockets'] })
console.log(await vector.query({ indexName: 'notes', queryVector: [1, 0, 0, 0], topK: 1, filter: { topic: 'pets' } }))
```

Use as `new Memory({ vector, embedder, options: { semanticRecall: true } })`. Offline example: `examples/vector.mjs`.

## Options

`new WarmVector(options)`:

| Name | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | Vector store id |
| `dimension` | number | 1536 | Vector length of every index; fixed for the life of the store |
| `path` | string | current directory | Storage directory; always set it |
| `hypervectorDimensions` | number | 4096 | Engine hypervector size |
| `maxFetch`, `overfetch` | number | engine defaults | Per-query ceiling; candidate pool for client-side filters |
| `warmConfig` | object | none | Native config, e.g. encryption at rest |

Cosine metric only; sparse vectors are rejected.

## Compatibility

`@mastra/core` >=1.78.0 <2 (tested 1.78.0, `@mastra/memory` 1.38.0), `writerslogic-warm` >=0.6.5, Node 20+ (tested 26.8). 15 tests.

## Reference

[Mastra](https://github.com/writerslogic/warm/blob/main/docs/integrations/frameworks/mastra.md)
