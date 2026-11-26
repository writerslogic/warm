# Quickstart examples

Ingest, search, compare with SQLite FTS5, encrypt, remember changing facts. The code behind the [five-minute quickstart](../../docs/quickstart.md): one corpus and one query set drive every language, CI runs each on every pull request, and the docs page includes these files rather than copying them.

| File | Run from the repository root |
|------|------------------------------|
| `rust/` | `cargo run --release --manifest-path examples/quickstart/rust/Cargo.toml` |
| `node/quickstart.mjs` | `npm ci --omit=optional && npm run build && node examples/quickstart/node/quickstart.mjs` (Node 24+) |
| `python/quickstart.py` | `WARM_API_KEY=quickstart uv run --no-project --with warm-sdk python examples/quickstart/python/quickstart.py` against a running `warm-server --input-dim 384` |
| `js-client/quickstart.mjs` | `WARM_URL=http://127.0.0.1:8080 WARM_API_KEY=quickstart node examples/quickstart/js-client/quickstart.mjs` against the same server, with `bindings/js/warm-sdk` built |

Build the server once with `cargo build --release --features server,security --bin warm-server`. Compatibility: Rust 1.89+, Node 24+ (embedded) or 18+ (client), Python 3.11+.

## Data

- `corpus.jsonl`: the 30 articles of the Universal Declaration of Human Rights (1948), taken
  from the English text on Wikisource, which reproduces the United Nations document. It is a
  United Nations document issued with a UN symbol and is in the public domain worldwide under
  UN Administrative Instruction ST/AI/189/Add.9/Rev.2.
- `queries.json`: eight questions, each with the article that answers it. They were written once
  and are not tuned; the quickstart is a demonstration, not a benchmark.
- `expected-search.txt`, `expected-search-python.txt`: the result tables the programs print.
  CI diffs the programs' output against them.

## The encryption check

The Rust and Node programs scan every file of an encrypted store and assert that none holds the
passage text, `documents.text` included (see
[security](../../docs/security.md)).
