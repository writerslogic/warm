<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Memory API checked-read follow-up

Baseline: `740e93e2a91900d7c0e0aa4163f593c458d96987`. This branch is independent
of pending MCP PR #371. These are source observations; runtime reproductions
and implementation validation are still required.

- `core/engine/memory_access.rs::document_snapshot` and `document_parts` still
  use the infallible checkpoint hydration helper and `store.chunk_text`.
  Memory get, history, explain, remember preflight, forgetting and survivor
  bridging call these helpers. The older retained-text helper panics on read
  failure; it must not be described as proven to silently return missing text.
- `DocumentView::text` is infallible, even inside `try_scan_documents`.
  `try_summaries(true)` therefore has an unchecked retained-text read.
- `scan_documents` expects a successful `try_scan_documents`, while the latter
  can fail during checkpoint hydration. `Index::rebuild`, `ensure_fresh` and
  `planning_revision` currently cannot propagate such errors. The index is
  rebuilt while its write lock is held.
- `has_document` hydrates the whole checkpoint before checking presence even
  though `DocumentStore::contains_key` already checks checkpoint ids and dead
  bits. It is used by HTTP quota preflight and Memory supersession/history.
- Point-read callbacks currently run under the Memory index read lock via
  `with_snapshot`. Any replacement must avoid holding that lock during text
  I/O, preserve revision validation/retry limits and publish derived index
  state only after successful reads. Metadata-only selection and preflight
  must not introduce unnecessary retained-text reads.

Existing checked document/chunk capture and identity validation should be
reused for point reads. Memory facts contain one chunk; survivor rewrites also
need that chunk's embedding. Existing get/history/recall/explain/maintenance
APIs already return Result, so propagate errors without unnecessary public
return-type changes. Keep the explicit infallible summaries convenience
wrapper's documented panic contract separate from the fallible API.

Acceptance includes damaged selected text (CRC mismatch and truncation),
repair, malformed checkpoint metadata, normal absence, metadata-only operations,
no partial survivor rewrite on read failure, revision/concurrency regressions,
memory API/HTTP tests, checkpoint tests, and strict scoped/final gates. No
performance gain, complete memory-read coverage, or production-readiness claim
is established by these source observations.

## Reproduced retained-text failures

The new regression reproduces eight panics: get, history, explain and
try_summaries(true), each with a CRC mismatch and truncation. Restoring the
cache makes every operation succeed and missing-record lookup remains None.
The run used Rust 1.99.0 with server,mcp features; the log and baseline-only
regression patch are preserved beside this document. Malformed-checkpoint
reproduction is still running.

## Checkpoint baseline and implementation underway

The malformed-checkpoint case reproduced five hydration panics (Memory get,
history, recall, metadata summaries, and presence lookup). After restoring the
checkpoint, Memory get returned the correct record but unnecessarily hydrated
it; the new preservation assertion failed. The complete log is preserved.

The current uncommitted candidate replaces metadata scans with checked
projections, propagates index rebuild failures without publishing partial state,
and uses checked point reads outside the Memory index lock. Survivor parts
capture the first chunk and embedding together, read text outside engine locks,
and reuse existing identity checks with the same three-attempt bound. Presence
and text-segment sync avoid unnecessary hydration. The infallible chunk_text
wrapper had no callers after these conversions; its history and callers were
checked before removal, while try_chunk_text remains in use.

Initial all-feature/all-target compilation passed with only the obsolete
wrapper warning; the wrapper has since been removed. Full memory, scoped-scan,
checkpoint, HTTP, concurrency and final Clippy validation is not yet complete.

The first candidate passed 19 Memory library tests, three access tests and
all ten checkpoint tests. The expanded run, with conversation-memory enabled,
passes 26 matching library tests (one existing external-index census diagnostic
ignored) and all five admin tests. It includes the new 257-record checkpoint/
overlay scan and replacement/deletion retry tests. An initial test compilation
failed because the project lock wrapper has no try_write; the test now acquires
its existing exclusive write guard at the capture hook, retaining the assertion
that document reads release their locks before I/O. That failed attempt is kept.

Presence-call-site review confirmed that deep format verification explicitly
uses verified_document_count. Admin verification now does so explicitly too,
so removing hydration from has_document cannot weaken its validation. The
malformed-checkpoint regression also requires admin verification to return the
decoder error without panicking. The final integration, updated checkpoint and
Clippy gates remain pending.

All runtime suites passed: 26 library tests (one existing diagnostic ignored),
five admin tests, 64 integration tests (allocation 6, conversation 15, forgetting
6, Memory API 29, HTTP 5, native workflow 3), and the focused checkpoint test
with explicit admin verification. The full ten-test checkpoint suite passed
earlier. Strict Clippy reported type complexity in a new test operation table;
the operation type now has an alias in both regression tables, without changing
assertions or behavior. Clippy and those two focused regressions are rerunning.

Final local gate: strict all-feature/all-target Clippy passes after the test
type aliases. Both focused corruption regressions pass again; all assertions
and retry limits are unchanged. Scoped rustfmt and diff checks pass. Exact-head
remote CI and merge remain required; see validation.json for hashes and results.
