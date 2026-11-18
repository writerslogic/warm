# @writerslogic/warm-openai-agents

A `Session` and memory tools for the OpenAI Agents SDK (JS) on the WARM native engine, in process.

## Install

```sh
npm install @writerslogic/warm-openai-agents @openai/agents zod
```

The native engine `writerslogic-warm` is a peer that npm 7+ installs for you; `@writerslogic/warm-core` comes as a dependency. From source: [JS core](../core/README.md#install).

## Quickstart

<!-- quickstart-check: skip: requires API key (offline: examples/quickstart.mjs) -->
```js
import { Agent, run } from '@openai/agents'
import { WarmSession, WarmStore, createMemoryTools } from '@writerslogic/warm-openai-agents'

const store = new WarmStore({ path: './memory' })
const session = new WarmSession({ store, sessionId: 'ada-chat' }) // conversation memory
const tools = createMemoryTools(store, { scope: { user: 'ada' } }) // long-term memory the model can call
const agent = new Agent({ name: 'assistant', instructions: 'Use memory tools to remember and recall facts.', tools })
await run(agent, 'Remember that I like dark roast coffee.', { session }) // needs OPENAI_API_KEY
```

No API key: `examples/quickstart.mjs` uses a scripted model.

## Options

`createMemoryTools(store, options)` returns `memory_remember`, `memory_recall`, `memory_forget`:

| Name | Type | Default | Meaning |
|---|---|---|---|
| `prefix` | string | `memory_` | Tool name prefix |
| `k`, `maxK` | number | 4, 20 | Default and maximum recall results |
| `scope` | record | none | Tags added to every note and required by recall and forget, one store for many users |

`new WarmSession({ store, sessionId })` implements the SDK `Session`; any session length.

## Compatibility

`@openai/agents` >=0.20.0 <0.21.0 (tested 0.20.0), `zod` ^4, `writerslogic-warm` >=0.6.5, Node 20+ (tested 26.8). 15 tests.

## Reference

[OpenAI Agents SDK, JS](https://github.com/writerslogic/warm/blob/main/docs/integrations/frameworks/openai-agents-js.md)
