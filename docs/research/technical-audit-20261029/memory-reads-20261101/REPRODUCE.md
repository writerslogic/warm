<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Checked Memory API reads

Baseline: `740e93e2a91900d7c0e0aa4163f593c458d96987`. Apply the decompressed
`baseline-regression.patch.gz` in an isolated checkout to reproduce the retained
text and checkpoint failures. The retained-text regression catches eight panics,
repairs the cache, then verifies successful reads and normal absence. The
checkpoint regression corrupts a source flag with a valid CRC while the store
is closed, reproduces five panics, and checks repair and checkpoint preservation.
No live checkpoint mapping is modified.

Use Rust 1.99.0, the shared target directory
`/Volumes/A/holographic-memory/target`, an existing worktree-specific TMPDIR,
and one local Rust build/test process at a time.

```sh
cargo +1.99.0 test --locked --features server,mcp --lib fallible_memory_reads_report_damaged_retained_text_and_recover -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp --test document_checkpoint document_reads_report_checkpoint_decode_errors_without_panicking -- --test-threads=1
```

Final candidate validation includes conversation callers, Memory HTTP behavior,
native workflow contracts, forgetting equivalence, and allocation limits:

```sh
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --lib memory_ -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --lib core::admin::tests -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --test memory_api --test memory_api_server --test native_memory_workflow --test conversation_memory --test forget_equivalence --test allocation_bounds -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --test document_checkpoint document_reads_report_checkpoint_decode_errors_without_panicking -- --test-threads=1
cargo +1.99.0 clippy --locked --all-features --all-targets -- -D warnings
```

The full ten-test checkpoint suite was also run with `server,mcp`. The final
focused checkpoint run adds the explicit admin verification assertion. The
`memory_` library filter includes an existing ignored census diagnostic requiring
`WARM_MEMORY_INDEX`; that external diagnostic is not part of this local fixture
validation. Check logs and `validation.json` for terminal outcomes rather than
treating this command list as proof that every gate passed.

The replacement/deletion hook acquires the document store's exclusive guard and
mutates the document between capture and read. It verifies coherent text,
metadata and embedding after retry, ordinary deletion, and failure on the third
repeated replacement. The survivor regression verifies that a failed read leaves
the revision, documents and links unchanged, and repair permits the same forget
to succeed. A separate case verifies metadata-only planning and explicit deletion
of a document whose cached text is damaged, including zeroed retained bytes.

These are local correctness and recovery fixtures, not measurements of latency,
RSS, concurrent index-rebuild cost or universal corruption tolerance. Other
infallible engine accessors remain outside this change. No paid providers,
downloaded models or deployment are involved.
