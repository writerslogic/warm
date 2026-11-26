# @writerslogic/warm-vercel-ai

Memory tools and recall middleware for the Vercel AI SDK on the WARM native engine, in process. Works with any model provider.

## Install

```sh
npm install @writerslogic/warm-vercel-ai ai zod
```

The native engine `warm-memory` is a peer that npm 7+ installs for you; `@writerslogic/warm-core` comes as a dependency. From source: [JS core](../core/README.md#install).

## Quickstart

```js
import { WarmStore } from '@writerslogic/warm-core'
import { memoryTools } from '@writerslogic/warm-vercel-ai'

const store = new WarmStore({ path: './memory' })
const { remember, recall } = memoryTools(store) // pass as `tools` to generateText or ToolLoopAgent
const ctx = { toolCallId: '1', messages: [] }
await remember.execute({ text: 'My favorite editor is Neovim' }, ctx)
console.log(await recall.execute({ query: 'favorite editor' }, ctx))
```

With a model, add `memoryMiddleware(store, { k: 3 })` through `wrapLanguageModel({ model, middleware })`. Offline agent with a mock model: `examples/agent.mjs`.

## Options

| Export | Option | Default | Meaning |
|---|---|---|---|
| `memoryTools(store, o)` | `scope`, `k`, `names`, `readOnly` | none; `k` 4; `remember`/`recall`/`forget`; `false` | `remember`, `recall`, `forget` tools; scope is merged into writes and ANDed into recall and forget |
| `memoryMiddleware(store, o)` | `k`, `filter`, `scope`, `minSemanticScore`, `maxChars`, `format`, `queryFrom` | `k` 4, `maxChars` 4000, `queryFrom` last user message, rest none | Recalls for each model call and appends memories to the last user message |
| `aiSdkEmbedder(model, o)` | `model`, `revision`, `dimensions` | none | Any AI SDK embedding model as the store encoder |
| `aiSdkReranker(model)` | | | Any AI SDK reranking model as the rerank hook |

## Compatibility

`ai` >=7.0.0 <8 (language model spec v4; tested 7.0.146), `zod` ^3.25.76 or ^4, `warm-memory` >=0.7.4, Node 20+ (tested 26.8). 5 tests.

## Reference

[Vercel AI SDK](https://github.com/writerslogic/warm/blob/main/docs/integrations.md)
