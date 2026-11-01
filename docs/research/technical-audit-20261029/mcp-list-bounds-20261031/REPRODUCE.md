<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# MCP payload and selected-read validation

Apply `baseline-regression.patch.gz` to an isolated checkout of
`740e93e2a91900d7c0e0aa4163f593c458d96987` to reproduce three Rust failures.
`baseline.log.gz` records the complete baseline run. The earlier standalone
stdio probe and its measured payloads are documented in `INVESTIGATION.md`.
`sdk-regression.patch.gz` contains the final SDK regressions and can be applied
separately to the same baseline (do not also apply the initial SDK edit from
`baseline-regression.patch.gz`). The corruption baseline used the independently
hashed unchanged stdio executable; its failure is in `sdk-corruption-baseline.log.gz`.

Use Rust 1.99.0, one local Rust process at a time, the shared target directory
`/Volumes/A/holographic-memory/target`, and an existing worktree-specific TMPDIR.

```sh
cargo +1.99.0 test --locked --all-features --bin warm-mcp -- --test-threads=1
cargo +1.99.0 build --locked --features mcp,security --bin warm-mcp
cargo +1.99.0 clippy --locked --all-features --all-targets -- -D warnings
```

The official SDK tests use the project's pinned SDK package and Node 22,
matching the existing MCP workflow. With Node 22 selected:

```sh
npm ci --prefix integrations/mcp/tests --ignore-scripts --no-audit --no-fund
WARM_MCP_BIN=/absolute/path/to/warm-mcp WARM_MCP_SECURITY=1 npm test --prefix integrations/mcp/tests
```

The fixtures use small local stores with no providers, downloaded models, or
paid services. The SDK suite also exercises local HTTP, resources, prompts,
namespaces, encryption, and client-visible error handling. Its new corruption
case restores the separate text cache before closing the server, verifies
errors for selected damaged records and checks repair. These tests do not
establish a transactional snapshot across listing pages or universal safety
under concurrent external filesystem modification.
