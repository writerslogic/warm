# demo-offline-agent

An assistant memory with no embedding model, network or API key: batch ingest, metadata filter, atomic replace, forget, two-hop relations, restart. Deterministic, so CI diffs the run against [`expected-output.txt`](expected-output.txt).

```sh
cargo run --release --example demo-offline-agent
```

| Item | Value |
|------|-------|
| Source | [`main.rs`](main.rs) |
| Needs | Rust 1.89+, default features |
| Output | [`expected-output.txt`](expected-output.txt) |

Walkthrough and adaptation notes: [docs/quickstart.md](../../docs/quickstart.md).
