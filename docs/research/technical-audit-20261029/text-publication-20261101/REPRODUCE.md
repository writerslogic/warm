<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Retained-text publication failure reproduction

Baseline: `d24cdb307ba0ca2bc1474e0063ebd58202f04180` (PR #372's tested source).
Apply decompressed `baseline-regression.patch.gz` to an isolated checkout.
Use Rust 1.99.0, two compiler workers, a worktree-specific `CARGO_TARGET_DIR`
and an existing worktree-specific `TMPDIR`. Historical logs retain the target
settings of their original execution.

```sh
cargo +1.99.0 test --locked --features server,mcp,conversation-memory,security --lib failed_text_append_reports_committed_state_and_reopens -- --test-threads=1
```

The baseline catches four real append panics with plaintext/encrypted stores and
unique/repeated document IDs. Each leaves the revision stale and skips later
journaled mutations. It independently verifies journal recovery after reopening.
Reads and a rejected append positively control the read-only handle fixture.
The original four cases pass with the candidate. Further acceptance tests cover
the final pending batch and unchanged document owners after a staged no-text
replacement is followed by a failing retained-text write.

The full local gate runs sequentially (see terminal logs and validation records
for actual results, not this command list):

```sh
cargo +1.99.0 test --locked --features server,mcp,conversation-memory,security --lib core::engine::mutation::tests -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory,security --lib core::documents:: -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory,security --lib core::engine::compaction::tests -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory,security --test document_checkpoint --test forget_equivalence --test allocation_bounds --test memory_api --test native_memory_workflow -- --test-threads=1
cargo +1.99.0 clippy --locked --all-features --all-targets -- -D warnings
```

The cache is derived from the durable journal. A committed live-publication error
requires reopening or reconciliation before retrying. These tests establish
error propagation, revision advancement, later mutation attempts and recovery
for the injected file-capability failure. They do not establish atomic complete
in-memory publication under I/O failure, every possible partial-write failure,
hardware power-loss durability, or a performance gain.

For the replacement-scrub baseline, apply `append-candidate.patch.gz` to the
baseline commit, then apply `scrub-regression.patch.gz`. This is an explicitly
recorded intermediate source state. Run:

```sh
cargo +1.99.0 test --locked --features server,mcp,conversation-memory,security --lib failed_replacement_scrub_publishes_owners_and_reports_committed_state -- --test-threads=1
```

All four plaintext/encrypted × same/different-chunk-count cases panic in that
source state and recover after reopening. The corrected candidate passes them
and verifies coherent live owner/index publication despite the reported scrub
failure. The initial broad gate was stopped to address this discovered path;
`interrupted-mutation.log.gz` is not a passing suite.

The rebased recovery gate uses `server,security` with a private target directory.
[Its validation record](../restore-retry-20261101/validation.json) lists every
focused reproduction command, exact source and executable hashes, terminal test
counts, strict Clippy result and compressed logs. It verifies 39 tests across
publication failures, retained owners, deferred encoding, idempotency limits,
restore rollback, all checkpoint cases and real HTTP restore/retry behavior.
The final integrated platform gate is separate.
