<img src="https://raw.githubusercontent.com/writerslogic/warm/main/assets/logo/warm-logo.png" alt="WARM logo" width="96" align="left">

# warm-memory: embedded WARM for Node.js

Local document search and agent memory backed by the WARM Rust engine, running inside your Node
process. It does not start a server or make service calls for ordinary document search.

```sh
npm install warm-memory
```

One package carries prebuilt native bindings for macOS (arm64, x64), Linux (arm64, x64; glibc and
musl) and Windows (arm64, x64); the loader picks the one for the running machine, so nothing is
downloaded or compiled at install time. Check the
[available npm versions](https://www.npmjs.com/package/warm-memory?activeTab=versions).
To build from the repository root with Rust installed:

```sh
npm ci
npm run build
```

## Package names

`warm-memory` replaces `writerslogic-warm` (0.7.2 and earlier) with the same `/semantic`, `/memory`
and `/vectra` entry points, store directories, and embedding/security configuration; change the
package name in `require`/`import` and `package.json`. The portable HTTP client is the separate
`warm-sdk` package, which talks to a running `warm-server`. The MCP launcher is `warm-mcp`.

## Search documents

```js
const { WarmMemorySystem } = require('warm-memory')
const { DocumentMemory } = require('warm-memory/semantic')

async function main() {
  const memory = new DocumentMemory(new WarmMemorySystem(4096, './memory-store'))
  await memory.memorize({
    id: 'backups',
    text: 'Restore deleted documents from verified backups.',
    sourceUri: 'operations.md',
  })
  const [hit] = await memory.search('restore backups')
  console.log(hit.documentId, hit.sourceUri, hit.text)
  await memory.flush()
}

main().catch(error => { console.error(error); process.exitCode = 1 })
```

Search uses local BM25 by default. You can supply embeddings for hybrid search; the optional
`@huggingface/transformers` integration loads a model from a local directory. CommonJS, ESM, and
TypeScript declarations are included. See the
[Node integration guide](https://github.com/writerslogic/warm/blob/main/docs/integrations.md)
and [API documentation](https://writerslogic.com/warm/docs/latest/).

## MCP clients

[`warm-mcp`](https://www.npmjs.com/package/warm-mcp) is a separate
launcher for the MCP server. Install or run that package if you need a server for an MCP client:

```sh
npx -y warm-mcp
```

This `warm-memory` package embeds the engine in a Node.js application. It does not expose
the MCP command.

Apache-2.0 · [Source](https://github.com/writerslogic/warm) ·
[Privacy](https://github.com/writerslogic/warm/blob/main/docs/security.md)
