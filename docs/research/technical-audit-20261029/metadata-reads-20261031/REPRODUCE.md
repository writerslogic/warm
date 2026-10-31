<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Checked metadata projection validation

Baseline: `a02a4b50a0a578b2761c2050f5c3e65ee7e27639` (PR #369).
Apply the decompressed `baseline-regression.patch.gz` and
`recovery-regression.patch.gz` to an isolated checkout of that base to reproduce
the metadata getter/listing and vector-index recovery panics. The malformed
checkpoint has a recomputed CRC and an invalid document source flag; it is
created while the store is closed. No live mmap contents are edited.

Use Rust 1.99.0, one local Rust process at a time, the shared target directory
`/Volumes/A/holographic-memory/target`, and an existing worktree-specific `TMPDIR`.

```sh
cargo +1.99.0 test --locked --features server,mcp --test document_checkpoint document_reads_report_checkpoint_decode_errors_without_panicking -- --test-threads=1
cargo +1.99.0 test --locked --features server --bin warm-server checkpoint_decode_failure_keeps_recovery_rows_and_dirty_marker -- --test-threads=1
```

The final source runs the complete vector-index tests and the affected contracts:

```sh
cargo +1.99.0 check --locked --all-features --all-targets
cargo +1.99.0 test --locked --features server,mcp --bin warm-server vindex::tests -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp --bin warm-mcp --test allocation_bounds --test document_checkpoint --test server_api -- --test-threads=1
cargo +1.99.0 clippy --locked --all-features --all-targets -- -D warnings
```

Recovery coverage puts a stale row before the unreadable document and verifies
that the error preserves both the valid log bytes and dirty marker. Restoring
the checkpoint permits normal reconciliation: the stale row is removed, the
matching version is retained, and the dirty marker is cleared. Metadata tests
retain cursor/limit, missing-record, allocation and cold-checkpoint assertions.
This does not qualify other infallible engine helpers, arbitrary concurrent
filesystem tampering, hardware power loss, or universal format conformance.
