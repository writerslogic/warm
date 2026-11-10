# warm-sdk

The portable JavaScript and TypeScript HTTP client for WARM. Connect to a separately running WARM server from Node.js 18+, browsers, Bun, Deno, or a worker runtime with the standard Fetch and Web Streams APIs. The package has no runtime dependencies, native binaries, Node built-in imports, or model downloads.

Version 0.2.0 is prepared for publication. Until [release availability](https://github.com/writerslogic/warm/blob/main/docs/RELEASING.md) confirms publication, build it from a repository checkout:

```sh
npm --prefix bindings/js/warm-sdk ci
npm --prefix bindings/js/warm-sdk run build
```

Then, from your application's directory:

```sh
npm install /absolute/path/to/warm/bindings/js/warm-sdk
```

After publication is verified, the registry install is `npm install warm-sdk@0.2.0`.

```ts
import { WarmClient } from 'warm-sdk'

const warm = new WarmClient({ url: 'http://localhost:8080' })
const saved = await warm.memory.remember('The project uses SQLite for local storage.', {
  subject: 'project-storage',
  metadata: { project: 'notes' },
})
const matches = await warm.memory.recall('What stores the project data?', { explain: true })
const stored = await warm.memory.get(saved.id)
```

CommonJS is also supported:

```js
const { WarmClient, WarmError } = require('warm-sdk')
```

Run the service separately using the [server guide](https://github.com/writerslogic/warm/blob/main/docs/SERVER.md). Browser requests also require the server deployment to permit the application's origin. This client does not start a server or provide embeddings, an LLM, a vector mask, or an embedded store.

## Connection and cancellation

`new WarmClient({ url, apiKey, tenantId, timeoutMs, fetch })` accepts:

- `url`: defaults to `http://localhost:8080`. HTTP and HTTPS are accepted, including a path prefix. Userinfo, query strings, and fragments are rejected.
- `apiKey`: sent as `Authorization: Bearer …`.
- `tenantId`: sent as `X-Tenant-ID`; 1–64 ASCII letters, digits, underscores, or hyphens.
- `timeoutMs`: a positive integer, default `30000`, covering the request and response body.
- `fetch`: an optional standard Fetch-compatible implementation. It must honor manual redirects and cancellation and return an unread `Response` body.

Every method accepts an optional `AbortSignal`. Memory methods with domain options accept `signal` in that options object; other methods accept a final `{ signal }` argument.

```ts
const abort = new AbortController()
const pending = warm.memory.recall('project storage', { signal: abort.signal })
abort.abort()
await pending.catch(error => {
  if (error instanceof Error) console.log(error.name)
})
```

Redirects are rejected, including redirects to the same origin. Cookies are not sent. Requests are not retried, including mutations whose result is uncertain after a connection failure. There is no connection pool to close; the supplied Fetch implementation owns its connections.

## Document search

| Method | Result |
|---|---|
| `addDocuments(documents, requestOptions?)` | `{ added }`; an empty list is a local no-op |
| `deleteDocument(id, requestOptions?)` | `{ deleted }` |
| `readDocumentChunk(id, index = 0, requestOptions?)` | Retained chunk, byte span, document metadata and opaque revision |
| `stats(requestOptions?)` | Tenant, document/vector counts, capacity and index status |
| `maintainDocumentIndex(requestOptions?)` | Document and vector index maintenance status |
| `query(input, requestOptions?)` | `{ matches: [{ id, text, metadata, score }] }` |

`DocumentInput` contains `id`, optional `text`, `vector`, and `metadata`. Import ownership also supports `importIdentity` and `importAdoptLegacy`, with the server's matching metadata stamp requirement. A vector must match the server's configured dimension. Omitting it requires text and a server configured to embed that text.

```ts
const result = await warm.query({
  queryVector: embedding,
  topK: 3,
  filter: { project: 'notes' },
  index: 'exact',
})
```

`QueryInput` also supports `query`, `today`, `documentIds`, and `ef`. Text queries require the server's optional model stages; this package supplies no models. The SDK maps public `queryVector`, `topK`, and `documentIds` to the core API's `query_vector`, `top_k`, and `document_ids` fields. Default `topK` is 3. The server controls dimension, quotas, configured maximum results, and available query modes.

IDs are encoded as a single path segment. The dot-only document IDs `.` and `..` cannot be addressed safely by Fetch URLs and are rejected by path-based methods; memory methods carry IDs in JSON and preserve them.

## Temporal memory

| Method on `warm.memory` | Result |
|---|---|
| `capabilities(contractVersion = 1, requestOptions?)` | Negotiated memory capabilities |
| `remember(text, options?)` | One `Remembered` record |
| `rememberMany(facts, requestOptions?)` | Atomic batch of `Remembered` records |
| `recall(query, options?)` | Ranked `Recalled[]` |
| `explain(query, id, options?)` | Retrieval and temporal `Explanation` |
| `get(id, requestOptions?)` | `FactRecord` or `null` |
| `history(id, requestOptions?)` | Supersession chain as `FactRecord[]` |
| `forget(scope, options?)` | `ForgetReport` |
| `applyForgetting(options?)` | Configured forgetting policy's report |
| `consolidate(options?)` | Configured consolidation policy's report |

Remember options include `id`, `eventTime`, `validFrom`, `validTo`, `subject`, `supersedes`, `entities`, `metadata`, `embedding`, and `sourceUri`. Times are signed Unix milliseconds represented by safe integer numbers. `Date` and string timestamps are not accepted. A supplied validity interval must have `validFrom < validTo`.

Recall/explain options include `k`, `asOf`, `knownAt`, `history` (`current`, `all`, `historical`), `subject`, `entities`, `filter`, `embedding`, `lexicalWeight`, `semanticWeight`, `minSemanticScore`, `fusion` (`blend`, `rrf`), `candidateLimit`, and `explain`. Omitted retrieval settings remain server defaults. Responses retain the wire's camelCase names, explicit `null` values, and omitted optional fields.

```ts
const preview = await warm.memory.forget(
  { kind: 'ids', ids: [saved.id] },
  { dryRun: true },
)
```

Forget scopes are discriminated by `kind`: `ids`, `subject`, `entity`, `ingestedBefore`, `eventBefore`, or `matching`. The time scopes take `time`; matching takes an exact metadata `filter`. Explicit `forget` defaults to `dryRun: false`, `includeChain: false`, and `purge: false`. `applyForgetting` defaults to a dry run. `consolidate` defaults to `dryRun: true` and `allowDestructive: false`. Policies execute on the server; the default policies select no destructive work.

## Validation and errors

Inputs must be finite JSON: cyclic values, bigint, accessors, sparse arrays, and non-JSON object instances are rejected before sending. IDs, text, metadata, vectors, counts, times, and enumerated options are validated against their applicable contracts. Request and decoded response bodies are limited to 16 MiB, with a 64-level JSON nesting limit. Text and individual metadata are limited to 64 KiB; batches to 4096 entries. The server may impose lower deployment-specific limits.

Local invalid inputs throw `TypeError` or `RangeError`. `WarmError` carries `code`, `status` (also `statusCode`), and optional `requestId`. HTTP errors retain bounded, sanitized structured server messages; raw HTML, request credentials, and underlying network exception contents are not attached. Transport codes include `aborted`, `timeout`, `network_error`, `redirect_error`, `invalid_response`, `request_too_large`, and `response_too_large`. Successful responses are checked against their documented shapes. A timeout or lost response does not prove a mutation was rolled back.

## Optional embedded engine

Applications that need a local native store can install the separate `writerslogic-warm` package explicitly. It has its own engine version and platform binaries. It is not a dependency or optional dependency of this HTTP client.

```sh
npm install writerslogic-warm
```

See the [embedded engine package](https://www.npmjs.com/package/writerslogic-warm) for its distinct API and supported platforms.
