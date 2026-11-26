# warm-claude-tool (TypeScript)

Backs Anthropic's client-side memory tool (`memory_20250818`) with WARM: Claude's `/memories` directory is stored in a `warm-mcp` server and every file is searchable by meaning.

## Install

```sh
npm install warm-claude-tool @anthropic-ai/sdk   # plus a warm-mcp binary: npx -y writerslogic-warm-mcp
```

## Quickstart

<!-- quickstart-check: skip: requires API key (ANTHROPIC_API_KEY) -->
```ts
import Anthropic from "@anthropic-ai/sdk";
import { WarmMemoryTool } from "warm-claude-tool";

const memory = await WarmMemoryTool.connect({ connection: { command: "warm-mcp" } }); // or { url: "http://127.0.0.1:8765/mcp" }
const client = new Anthropic();
const final = await client.beta.messages.toolRunner({
  model: "claude-sonnet-5-5",
  max_tokens: 1024,
  messages: [{ role: "user", content: "Remember that Acme Corp prefers email follow-ups." }],
  tools: [memory.asTool(), memory.recallTool()],
});
await memory.close();
```

Needs `ANTHROPIC_API_KEY`, so it is not run by the docs check; the tool itself is tested offline against a real `warm-mcp` in `test/`.

## Options

| Name | Type | Default | Meaning |
|---|---|---|---|
| `connection.command`, `args`, `env` | string, string[], record | `warm-mcp` | Start `warm-mcp` over stdio |
| `connection.url`, `token` | string, string | none, `WARM_MCP_TOKEN` | Connect to a shared `warm-mcp --transport http` |
| `namespace` | string | none | One store per user (server needs `--namespace-arg`) |
| `maxFileBytes` | number | server limit | Lower the per-file size cap |
| `maxViewChars` | number | 16000 | Longest `view` result before paging with `view_range` |
| `editRetries` | number | 25 | Compare-and-set retries when another writer edits the same file |

## Compatibility

Node 18+, `@anthropic-ai/sdk` >= 0.135.0 (tested 0.135.0), `@modelcontextprotocol/sdk` 1.32.1, a `warm-mcp` server. 29 tests against a real `warm-mcp`.

## Reference

Commands, safety rules and semantic recall: [Anthropic memory tool on WARM](https://github.com/writerslogic/warm/blob/main/docs/integrations.md). Python sibling: [`../python`](../python/README.md).
