# Quickstart

Five minutes, one small public corpus: **ingest**, **search**, **compare** with SQLite FTS5,
**encrypt**, and **remember** facts that change.

| Path | Runtime | Needs |
|---|---|---|
| [Portable client](#portable-javascript-client) | JavaScript over HTTP (`warm-sdk`) | A running `warm-server` |
| [Corpus comparison](#corpus-comparison-with-the-engine) | Rust, Python (HTTP) or embedded Node | Rust toolchain; Node 24 for embedded Node |

Not installed yet: [install](../get-started/install.md). Unsure which runtime: [choose](../get-started/choose.md).

## Portable JavaScript client

From a checkout, build the client (needs Node 20.19 or newer; the built client runs on Node 18)
and build the server from the same checkout:

```sh
npm ci --prefix bindings/js/warm-sdk
npm run build --prefix bindings/js/warm-sdk
cargo install --locked --path . --features server,security --bin warm-server
WARM_API_KEY=quickstart warm-server --input-dim 384
```

Leave the server running (in an application, `npm install warm-sdk` skips the client build). In another terminal, from the same checkout:

```sh
WARM_URL=http://127.0.0.1:8080 WARM_API_KEY=quickstart \
  node examples/quickstart/js-client/quickstart.mjs
```

The example needs Node 18 or newer, the built client, no native addon and no embedding model. It
creates a fresh tenant by default (`WARM_TENANT_ID` selects an authorized one); unique record IDs
and an exact metadata filter isolate each run. It remembers one fact, checks recall and get, then
purges that fact and checks its absence.

```js
--8<-- "examples/quickstart/js-client/quickstart.mjs:ingest"
--8<-- "examples/quickstart/js-client/quickstart.mjs:hit"
--8<-- "examples/quickstart/js-client/quickstart.mjs:forget"
```

Expected final line: `JavaScript client quickstart passed: remember, recall, get, forget`.
In an application, import `WarmClient` from `warm-sdk` instead of the relative path. Client API:
[README](https://github.com/writerslogic/warm/blob/main/bindings/js/warm-sdk/README.md).

## Corpus comparison with the engine

The corpus is the 30 articles of the Universal Declaration of Human Rights, a United Nations
document in the public domain (`examples/quickstart/corpus.jsonl`). The queries are eight
questions in `examples/quickstart/queries.json`, each with the article that answers it.

!!! info "This page cannot rot"
    Every code block below is included from `examples/quickstart/`. CI runs those files on every
    pull request and checks their output against the tables shown here.

## Set up

=== "Rust"

    ```sh
    git clone https://github.com/writerslogic/warm
    cd warm
    cargo run --release --manifest-path examples/quickstart/rust/Cargo.toml
    ```

    The example crate depends on the library by path with the `security` feature, plus `rusqlite`
    (bundled SQLite). In your own project: `cargo add warm-rust --features security`.

=== "Python"

    The Python package is an HTTP client for `warm-server`. Start the server, then run the script:

    ```sh
    git clone https://github.com/writerslogic/warm
    cd warm
    cargo install --locked --path . --features server,security --bin warm-server
    WARM_API_KEY=quickstart warm-server --input-dim 384 &
    WARM_API_KEY=quickstart uv run --no-project --with warm-sdk \
        python examples/quickstart/python/quickstart.py
    ```

    `--no-project` stops uv from compiling this checkout's native package. The comparison uses the
    memory API, which ranks with the same stemmed lexical search as the Rust and Node examples, so it
    needs no embedding model. The masked-vector step uses `toy_embed`, a small hashed bag of words
    that is only a stand-in: use a real embedding model for semantic search. The in-process
    `writerslogic_warm.Warm` class needs `warm-python` 0.7.1 or later.

=== "Embedded Node"

    ```sh
    git clone https://github.com/writerslogic/warm
    cd warm
    npm ci --omit=optional && npm run build
    node examples/quickstart/node/quickstart.mjs
    ```

    The native module is built from source (Rust toolchain required). The script uses `node:sqlite`: Node 24 or newer.

## 1. Ingest

A store is a directory and reopens with the same arguments. A document has an ID, text, a source
URI, and optional metadata and embeddings. Text is split into chunks (256 words by default) and
written in one atomic batch.

=== "Rust"

    ```rust
    --8<-- "examples/quickstart/rust/src/main.rs:ingest"
    ```

=== "Python"

    ```python
    --8<-- "examples/quickstart/python/quickstart.py:ingest"
    ```

=== "Embedded Node"

    ```js
    --8<-- "examples/quickstart/node/quickstart.mjs:ingest"
    ```

## 2. Search, and compare with SQLite FTS5

The baseline is the same corpus in an FTS5 virtual table ranked by `bm25`. A bare multi-word FTS5
query means AND, which would unfairly return nothing for most of these questions, so the
baseline ORs the words.

=== "Rust"

    ```rust
    --8<-- "examples/quickstart/rust/src/main.rs:compare"
    ```

=== "Python"

    ```python
    --8<-- "examples/quickstart/python/quickstart.py:compare"
    ```

=== "Embedded Node"

    ```js
    --8<-- "examples/quickstart/node/quickstart.mjs:compare"
    ```

Output (identical in Rust and Node, which call the same engine; CI diffs it):

```text
--8<-- "examples/quickstart/expected-search.txt"
```

The Python script prints this table, which differs because `toy_embed` and the engine's text
analysis are not the same thing:

```text
--8<-- "examples/quickstart/expected-search-python.txt"
```

Eight hand-written questions on 30 short passages show how the API behaves; they are not a
benchmark and say nothing about relevance at scale. Default text search is stemmed BM25, the same
family as FTS5, so keyword-style queries largely agree. With embeddings supplied, WARM fuses exact
cosine with BM25, which is where it can beat a lexical engine on paraphrases. Public-dataset
results, including where WARM loses: [benchmarks](../PUBLIC-BENCHMARKS.md).

Each hit also carries the passage text, its byte range in the source and the score, so you can
show a snippet and link back to the source:

=== "Rust"

    ```rust
    --8<-- "examples/quickstart/rust/src/main.rs:hit"
    ```

=== "Python"

    ```python
    --8<-- "examples/quickstart/python/quickstart.py:hit"
    ```

=== "Embedded Node"

    ```js
    --8<-- "examples/quickstart/node/quickstart.mjs:hit"
    ```

## 3. Encrypt

=== "Rust"

    ```rust
    --8<-- "examples/quickstart/rust/src/main.rs:encrypt"
    ```

=== "Python"

    ```python
    --8<-- "examples/quickstart/python/quickstart.py:encrypt"
    ```

=== "Embedded Node"

    ```js
    --8<-- "examples/quickstart/node/quickstart.mjs:encrypt"
    ```

Rust and Node enable AES-256-GCM with an Argon2id-derived key (the `security` feature, which the
npm build includes). The script checks that a wrong passphrase cannot open the store and the right
one can, and scans every file in the store, text segment included, for a known passage. Use
`encryptionPassphraseEnv` (Node) or `encryption_passphrase_env` (Rust) rather than a literal
passphrase. What encryption does not cover (file names, sizes, access patterns):
[privacy limits](../concepts/privacy-limits.md).

The Python client cannot encrypt: `warm-server` has no encryption at rest. Client-side vector
masking with `zero_trust_key` is obfuscation; the server still learns every pairwise similarity
and stores text and metadata in plaintext ([server limits](../SERVER.md)).

## 4. Remember what changed

The memory API keeps old facts as history, lets a newer fact replace an older one, and answers
"what was true as of" a date.

```rust
--8<-- "examples/quickstart/rust/src/bin/memory.rs:memory"
```

Output (CI diffs it):

```text
--8<-- "examples/quickstart/expected-memory.txt"
```

`forget` and `explain` complete the set: [memory API](../memory-api/README.md).

## 5. Use real embeddings

The Python script uses a toy hashed embedding. For semantic search with no API key and no
service, build with the `local-embed` feature and set `text_encoder: TextEncoder::LocalEmbedV1`;
search then fuses cosine with BM25. See [local embeddings](../embeddings/README.md). To rank with
learned-sparse vectors, see [learned-sparse ranking](../guides/sparse-ranking.md).

## Where next

- [Offline agent demo](offline-agent.md): a personal agent memory with no embeddings and no network.
- [Concepts](../concepts/index.md): what is lossy, and the tested capacity range.
- [Production readiness](../production-readiness.md): storage format, locking, migration, limits.
