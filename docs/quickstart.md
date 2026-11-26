# Quickstart

Install WARM for your runtime, then store and recall one fact. Every example uses a lexical
index, so no embedding model is needed.

| Runtime | Package | Needs a server |
|---|---|---|
| Rust | `warm-rust` | no, in process |
| Embedded Node | `warm-memory` | no, in process |
| Python | `warm-sdk` | yes, `warm-server` |
| JavaScript or TypeScript client | `warm-sdk` (npm) | yes, `warm-server` |
| MCP client | `warm-mcp` (npm or PyPI) | no, the launcher runs it |

Rust 1.89 or newer builds the engine. Nothing is enabled by default; see the
[feature table](reference/api.md#cargo-features).

## Rust

```sh
cargo add warm-rust --features security
```

```rust
use writerslogic_warm::{DocumentInput, SearchOptions, WarmCore};

// A store is a directory; it reopens with the same arguments.
let warm = WarmCore::new(4096, Some("./store".into()), None)?;
warm.memorize_documents(vec![DocumentInput {
    id: "udhr-14".into(),
    text: "Everyone has the right to seek and to enjoy asylum from persecution.".into(),
    source_uri: Some("udhr#14".into()),
    version: None, metadata: None, chunk_words: None, overlap_words: None,
    store_text: None, embeddings: None, title_weight: None,
}])?;
let hits = warm.search_documents("asylum from persecution", &SearchOptions::default())?;
// Each hit has document_id, score, start_byte, end_byte and the passage text.
```

A complete program that compares the results with SQLite FTS5 is in
[`examples/quickstart/rust`](https://github.com/writerslogic/warm/tree/main/examples/quickstart/rust).
With the `security` feature a new store is encrypted by default; see
[security](security.md#encryption-at-rest).

## Embedded Node

```sh
npm install warm-memory
```

```js
const { WarmMemorySystem } = require('warm-memory')
const { DocumentMemory } = require('warm-memory/semantic')

const memory = new DocumentMemory(new WarmMemorySystem(4096, './store'))
await memory.ingest([{ id: 'udhr-14', text: 'Everyone has the right to seek and to enjoy asylum from persecution.' }])
const [hit] = await memory.search('asylum from persecution')
console.log(hit.documentId, hit.startByte, hit.endByte, hit.score, hit.text)
```

## Python and JavaScript clients

Both clients talk to `warm-server`. Build and start it:

```sh
cargo install --locked --features server,security --bin warm-server warm-rust
WARM_API_KEY=change-me warm-server --input-dim 384
```

```sh
uv pip install warm-sdk        # Python
npm install warm-sdk           # JavaScript
```

```python
from writerslogic_warm_sdk import WarmClient

client = WarmClient(url="http://127.0.0.1:8080", api_key="change-me", tenant_id="demo")
client.memory.remember_many([{"id": "udhr-14", "text": "Everyone has the right to seek asylum."}])
print(client.memory.recall("asylum", k=3))
```

```js
import { WarmClient } from 'warm-sdk'

const memory = new WarmClient({ url: 'http://127.0.0.1:8080', apiKey: 'change-me', tenantId: 'demo' }).memory
await memory.remember('Alice lives in Paris', { id: 'home-1', subject: 'alice/city' })
await memory.recall('where does Alice live')
```

The server is described in [server](server/index.md). Runnable versions are in
[`examples/quickstart`](https://github.com/writerslogic/warm/tree/main/examples/quickstart).

## MCP

```sh
npx -y warm-mcp          # stdio, for clients that start a local command
uvx warm-mcp                          # the same server from PyPI
```

Client configuration for Claude Code, Codex CLI, Gemini CLI, VS Code, Cursor and others is in
[`integrations/editors`](https://github.com/writerslogic/warm/tree/main/integrations/editors).

## Facts that change

The memory API keeps a newer fact from destroying an older one:

```js
await memory.remember('Alice lives in Paris',  { id: 'home-1', subject: 'alice/city', validFrom: 1577836800000 })
await memory.remember('Alice lives in Berlin', { id: 'home-2', subject: 'alice/city', validFrom: 1735689600000, supersedes: ['home-1'] })
await memory.recall('where does Alice live')                         // Berlin
await memory.recall('where does Alice live', { asOf: 1622505600000 }) // Paris
```

Next: [concepts](concepts.md), [API overview](reference/api.md), [FAQ](reference/faq.md).
