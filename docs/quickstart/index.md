# Quickstart

Five minutes, one small public corpus: **ingest** documents, **search** them,
**compare** with SQLite FTS5, **encrypt** the store, and **remember** facts that change. Pick Rust, Python or embedded Node.

The portable JavaScript quickstart below uses `warm-sdk` over HTTP. The corpus comparison
uses Rust, Python or the optional embedded `writerslogic-warm` Node engine.

## Portable JavaScript client

From a checkout, build the client (needs Node 20.19 or newer; the built client runs on Node 18)
and install a prebuilt server:

```sh
npm ci --prefix bindings/js/warm-sdk
npm run build --prefix bindings/js/warm-sdk
curl -fsSL https://raw.githubusercontent.com/writerslogic/warm/main/scripts/install-warm-server.sh | sh
WARM_API_KEY=quickstart warm-server --input-dim 384
```

In an application, skip the build: `npm install warm-sdk`.

Leave the server running. In another terminal, from the same checkout:

```sh
WARM_URL=http://127.0.0.1:8080 WARM_API_KEY=quickstart \
  node examples/quickstart/js-client/quickstart.mjs
```

The example needs Node 18 or newer and uses the built client directly. It creates a fresh
tenant by default; `WARM_TENANT_ID` can select an authorized tenant instead. Unique record IDs
and an exact metadata filter isolate each run. It remembers one fact, checks recall and get,
then purges only that fact and checks its absence. It needs no native addon or embedding model.

```js
--8<-- "examples/quickstart/js-client/quickstart.mjs:ingest"
--8<-- "examples/quickstart/js-client/quickstart.mjs:hit"
--8<-- "examples/quickstart/js-client/quickstart.mjs:forget"
```

Expected final line: `JavaScript client quickstart passed: remember, recall, get, forget`.
In an application that installed the package, import `WarmClient` from `warm-sdk` instead of
the checkout's relative path. See the [exact client API](https://github.com/writerslogic/warm/blob/main/bindings/js/warm-sdk/README.md)
and [source installation](../get-started/install.md).

## Corpus comparison with the engine

The corpus is the 30 articles of the Universal Declaration of Human Rights, a United Nations
document in the public domain (`examples/quickstart/corpus.jsonl`). The queries are eight
questions in `examples/quickstart/queries.json`, each with the article that answers it.

!!! info "This page cannot rot"
    Every code block below is included from `examples/quickstart/`, and CI runs those files on
    every pull request and checks their output against the tables shown here. If a snippet stops
    working, the build fails before the page does.

## Set up

=== "Rust"

    ```sh
    git clone https://github.com/writerslogic/warm
    cd warm
    cargo run --release --manifest-path examples/quickstart/rust/Cargo.toml
    ```

    The example crate depends on the library by path, with the `security` feature, plus
    `rusqlite` (bundled SQLite) for the comparison. In your own project:
    `cargo add warm-rust --features security`.

=== "Python"

    The Python package is an HTTP client for `warm-server`. Start the server, then run the script:

    ```sh
    git clone https://github.com/writerslogic/warm
    cd warm
    curl -fsSL https://raw.githubusercontent.com/writerslogic/warm/main/scripts/install-warm-server.sh | sh
    WARM_API_KEY=quickstart warm-server --input-dim 384 &
    WARM_API_KEY=quickstart uv run --no-project --with warm-sdk \
        python examples/quickstart/python/quickstart.py
    ```

    `--no-project` stops uv from compiling this checkout's native package. The comparison uses the
    memory API, which ranks with the same stemmed lexical search as the Rust and Node examples, so it
    needs no embedding model. The masked-vector step uses `toy_embed`, a small hashed bag of words
    that is only a stand-in: use a real embedding model for semantic search. The in-process
    `writerslogic_warm.Warm` class is not in the 0.6.8 wheel; it ships with the next release.

=== "Embedded Node"

    ```sh
    git clone https://github.com/writerslogic/warm
    cd warm
    npm ci --omit=optional && npm run build
    node examples/quickstart/node/quickstart.mjs
    ```

    The `writerslogic-warm` native module is built from source (Rust toolchain required). The script uses
    `node:sqlite`, so it needs Node 24 or newer.

## 1. Ingest

A store is a directory; it persists there and reopens with the same arguments. Each document has
an ID, its text, a source URI, and optional metadata and embeddings. Text is split into chunks
(256 words by default) and written in one atomic batch.

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

Read this honestly. Eight hand-written questions on 30 short passages show how the API behaves.
They are not a benchmark and say nothing about relevance at scale. WARM's default text search is
stemmed BM25, the same family as FTS5, so on keyword-style queries they largely agree. Supply embeddings (see below) and WARM fuses exact
cosine with BM25, which is where it can beat a lexical engine on paraphrases. Measured retrieval
results on public datasets are on the [benchmarks](../PUBLIC-BENCHMARKS.md) page, including where
WARM loses.

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

Rust and Node enable AES-256-GCM with an Argon2id-derived key (the `security` feature, which
the npm build includes). The script then proves two things: a wrong passphrase cannot open the
store, and the right one can. Prefer `encryptionPassphraseEnv` (Node) or
`encryption_passphrase_env` (Rust) over a literal passphrase in real code.

The script also scans every file in the encrypted store for a known passage and asserts none
contains it, the text segment included. Read the [privacy limits](../concepts/privacy-limits.md)
for what encryption does not cover (file names, sizes, access patterns).

The Python client cannot encrypt, because `warm-server` has no encryption at rest. It offers
client-side vector masking with `zero_trust_key`, which is obfuscation: the server still learns
every pairwise similarity and stores text and metadata in plaintext
([server limits](../SERVER.md)).

## Where next

- [Offline agent demo](offline-agent.md): a personal agent memory with no embeddings and no network.
- [Concepts](../concepts/index.md): what is lossy, and the tested capacity range.
- [Production readiness](../production-readiness.md): storage format, locking, migration, limits.

## 4. Remember what changed

Agents need facts that change. The memory API keeps old facts as history, lets a newer fact
replace an older one, and answers "what was true as of" a date.

```rust
--8<-- "examples/quickstart/rust/src/bin/memory.rs:memory"
```

Output (CI diffs it):

```text
--8<-- "examples/quickstart/expected-memory.txt"
```

`forget` and `explain` complete the set; see the [memory API](../memory-api/README.md).

## 5. Use real embeddings

The Python script above uses a toy hashed embedding. For semantic search on your machine with no
API key and no service, build with the `local-embed` feature and set
`text_encoder: TextEncoder::LocalEmbedV1`: chunks and queries are embedded by a bundled model and
search fuses cosine with BM25. Options, model choice and offline packs are in
[local embeddings](../embeddings/README.md).
