<img src="https://raw.githubusercontent.com/writerslogic/warm/main/assets/logo/warm-logo.png" alt="WARM logo" width="96" align="left">

# writerslogic-warm: embedded WARM for Node.js

Local document search and agent memory backed by the WARM Rust engine. The package
selects a native binding for your operating system and architecture. It does not start a server or
make service calls for ordinary document search.

The primary JavaScript client is the separate portable `warm-sdk` HTTP package. Choose this
`writerslogic-warm` package when the engine should run inside Node. For a published engine
release, install it explicitly:

```sh
npm install writerslogic-warm@0.6.8
```

Check the [available npm versions](https://www.npmjs.com/package/writerslogic-warm?activeTab=versions).
To build from the repository root with Rust installed:

```sh
npm ci --omit=optional
npm run build
```

The native binding is distributed through an optional platform package, so keep npm's optional
dependencies enabled. The release targets are macOS (arm64, x64), Linux (arm64, x64; glibc and musl),
and Windows (arm64, x64).

## Package names

Existing embedded consumers keep `writerslogic-warm`, `/semantic`, `/memory` and `/vectra`
imports, store directories, and embedding/security configuration. The portable `warm-sdk`
exports `WarmClient` and uses a separately running HTTP server; adopting it requires an
explicit API migration. The separately versioned MCP launcher remains `writerslogic-warm-mcp`.

## Search documents

```js
const { WarmMemorySystem } = require('writerslogic-warm')
const { DocumentMemory } = require('writerslogic-warm/semantic')

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
[Node integration guide](https://github.com/writerslogic/warm/blob/main/docs/INTEGRATION.md)
and [API documentation](https://writerslogic.com/warm/docs/latest/).

## MCP clients

[`writerslogic-warm-mcp`](https://www.npmjs.com/package/writerslogic-warm-mcp) is a separate
launcher for the MCP server. Install or run that package if you need a server for an MCP client:

```sh
npx -y writerslogic-warm-mcp
```

This `writerslogic-warm` package embeds the engine in a Node.js application. It does not expose
the MCP command.

Apache-2.0 · [Source](https://github.com/writerslogic/warm) ·
[Privacy](https://github.com/writerslogic/warm/blob/main/docs/PRIVACY.md)
