<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Checkpoint vector maintenance reproduction

Apply the decompressed `baseline-regression.patch.gz` to baseline
`d24cdb307ba0ca2bc1474e0063ebd58202f04180` in an isolated worktree.
Use Rust 1.99.0, `CARGO_TARGET_DIR=/Volumes/A/holographic-memory/target`,
an existing worktree-specific TMPDIR and one local Rust invocation at a time.

```sh
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --test document_checkpoint checkpoint_vector_ -- --test-threads=1
```

The malformed-source fixture rewrites the checkpoint only while the engine is
closed, recomputes its envelope CRC, then exercises seven embedding operations
with zero and one live embedding. It verifies the malformed document still fails
a checked read, the unrelated live record remains readable, and repair survives
reopen. No test claims corrupt metadata is valid or skips full verification.
The valid fixture covers both index thresholds at limit−1, limit and limit+1,
tail replay, explicit maintenance, replacement/deletion and forced hydration.

The initial compile attempt collided with an existing `embedded` config helper;
its log is retained. The corrected fixture reuses that config and names its
input helper separately. This compilation failure is not baseline behavior. The first valid-checkpoint
fixture also attempted an invalid empty ingest; it was corrected to leave the
zero-embedding store untouched. The separate preservation log then reproduced
unnecessary checkpoint hydration. All 14 malformed-checkpoint panics were
already reproduced by the unchanged corruption fixture.

Final gates (see `validation.json` for actual terminal results):

```sh
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --test document_checkpoint --test forget_equivalence --test allocation_bounds --test store_lifecycle -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --lib document_ -- --test-threads=1
cargo +1.99.0 test --locked --features server,mcp,conversation-memory --lib core::engine::compaction::tests -- --test-threads=1
cargo +1.99.0 clippy --locked --all-features --all-targets -- -D warnings
```

No measured latency, allocation, RSS or hardware durability gain is claimed.

Final strict Clippy requested the equivalent assertion rewrite from
`rows <= document_count - 1` to `rows < document_count`. The original Clippy log
is retained, strict Clippy passes after the rewrite, and both focused regressions
pass again. The broader suites had passed before that test-only change.
