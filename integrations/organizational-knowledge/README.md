# Organizational knowledge adapter (JavaScript)

Imports the flat curated documents of the [ToIP EGWG prototype](https://github.com/trustoverip/egwg-reporting-tf-prototype-mcp) into a WARM store through `DocumentMemory`. It is an adapter example, not an agreed ToIP contract, a connector or an MCP backend.

## Install

No package to install: run from a WARM checkout with the Node native binding built (`npm ci && npm run build`, see [Contributing](../../CONTRIBUTING.md#set-up)). Zero dependencies of its own.

## Quickstart

```js
const { WarmMemorySystem } = require('writerslogic-warm')
const { OrganizationalKnowledge, readCuratedFiles } = require('./integrations/organizational-knowledge/index.cjs')

async function main() {
  const knowledge = new OrganizationalKnowledge(new WarmMemorySystem(4096, './toip-js-store'), { namespace: 'toip' })
  await knowledge.importDocuments(await readCuratedFiles(['integrations/organizational-knowledge/example/fixtures']))
  console.log(await knowledge.search('quartzmarker', { k: 5 }))
}
main().catch(error => { console.error(error); process.exitCode = 1 })
```

Or the CLI: `node integrations/organizational-knowledge/example/cli.cjs import ./toip-js-store toip integrations/organizational-knowledge/example/fixtures`.

## Options

| Surface | Name | Default | Meaning |
|---|---|---|---|
| constructor | `namespace` | required | Scopes identities and queries (a query filter, not authorization) |
| constructor | `embedder`, `rerank` | none (local lexical) | `DocumentMemory` hooks; the store must be created in the embedder's space |
| `search` | `k`, `filter` | 10, none | Result count; exact top-level metadata equality |
| `forget` | `apply` | `false` | Preview unless `true` |
| `forgetScope` | `sourceSystem` or `all`, `keep`, `limit` | `limit` 10,000 | Remove or prune a scope; preview unless `apply` |
| CLI | `import`, `search`, `forget`, `forget-all`, `prune` | | `forget-all` and `prune` preview unless `--apply` |

Types: `index.d.cts`.

## Compatibility

Node 20+ (tested 20.20 and 26.8), WARM native binding of the same checkout. Interoperates with the Rust example on one store, one writer at a time.

## Tests

`node --test integrations/organizational-knowledge/test/*.test.cjs` (the embedder test needs `WARM_TEST_ONNX_MODEL_DIR`; the scope tests skip when the binding lacks `listDocuments`).

## Reference

Mapping, behavior, limits and Rust interoperability: [Organizational knowledge](../../docs/integrations.md#javascript-adapter-reference).
