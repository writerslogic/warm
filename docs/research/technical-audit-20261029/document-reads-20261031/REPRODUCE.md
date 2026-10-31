<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Fallible document read validation

Run from this source revision with Rust 1.99.0, one Rust process at a time,
`CARGO_TARGET_DIR=/Volumes/A/holographic-memory/target`, and a worktree-specific
existing `TMPDIR`. No providers, model downloads, or paid services are used.

```sh
cargo +1.99.0 test --locked --lib document -- --test-threads=1
cargo +1.99.0 test --locked --lib core::documents::listing -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp --bin warm-mcp --test allocation_bounds -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp --test document_checkpoint --test document_chunks --test forget_equivalence --test server_api -- --test-threads=1
cargo +1.99.0 test --locked --features server --bin warm-server error::tests -- --test-threads=1
cargo +1.99.0 clippy --locked --all-features --all-targets -- -D warnings
```

`contracts.log.gz` retains the original two eager-hydration assertion failures;
record equivalence passed. The corrected tests separately verify checkpoint
preservation and force a rebuild before retaining the post-rebuild oracle checks.
`final-contracts.log.gz` records nine checkpoint and five chunk tests passing,
then ends during forgetting tests: the recovered session/process exited without
a terminal result. `remaining-contracts.log.gz` completes forgetting and HTTP.

`checkpoint-decode-baseline.log.gz` reproduces the listing panic on a CRC-valid
checkpoint with an invalid record source flag. From the final source, reverse
the decompressed `checkpoint-decoder-fix.patch.gz` and run only
`document_reads_report_checkpoint_decode_errors_without_panicking` in the
`document_checkpoint` target to reproduce; reapply the patch afterward. The
final checkpoint log also verifies recovery after restoring the valid checkpoint.
This regression is distinct from retained-text bit flips and truncation.

Logs describe their particular source stage. The final strict Clippy covers all
features and targets; runtime cases are bounded to the commands above. The
external-index diagnostic is intentionally ignored. Metadata-only public APIs
and other infallible convenience APIs are not qualified by this change. No
performance, universal snapshot completeness, or competitive quality claim is made.
