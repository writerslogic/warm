<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Retained-text publication failures

Baseline: `d24cdb307ba0ca2bc1474e0063ebd58202f04180` (PR #372).
The paused gate completed: three deferred-cache regressions, 58 integration tests
and strict all-feature/all-target Clippy passed on the recorded corrected source.
[Resumed validation](resumed-validation.json) preserves those logs and verifies
all six historical source hashes. Five files retain those hashes after rebase;
`image.rs` additionally incorporates the merged #373 hydration-helper removal.
The rebased candidate also passes 39 focused publication, deferred-cache,
checkpoint, restore and retry tests, plus strict scoped Clippy; see
[combined validation](../restore-retry-20261101/validation.json). This includes
all 13 checkpoint tests after #373 and real HTTP restore/idempotency checks.
The final integrated CI gate remains required. Builds use two compiler workers
and one test thread.

The notes below retain the investigation sequence, including earlier hypotheses,
incomplete runs and the subsequently corrected production-caller build failure.

Verified source facts:

- `DocumentStore::stripped` calls `expect` on `TextSegment::append`.
  `insert_batch` strips the whole batch before installing its document owners.
- `commit_payload_at_revision` appends the journal before `apply_mutations`, then
  advances the revision and wraps failures with `CommittedMutationError` only
  after `apply_mutations` returns. Its panic path therefore bypasses that code.
- Document chunk vectors are installed before the batched document-store write.
  `install_documents` also flushes before mixed mutations and repeated IDs.
- The existing committed-error contract says to reopen or reconcile before
  retrying. The existing failed-scrub regression requires remaining mutations
  and revision advancement despite a reported post-journal I/O failure.
- Replay uses the same mutation application path; canonical rebuild and
  checkpoint hydration also call document insertion. Returning a Result there
  must preserve their original live state when constructing a replacement fails.

Prepared test: swap the segment for a real read-only file handle, preserving
contents, offsets, encryption and scrub state. Reads must still work; an actual
append must fail (positive control). Commit two replacement documents followed
by a vector and deletion, catch any panic, check the typed committed outcome,
revision and later effects, then reopen and verify complete journal recovery.
The fixture covers plaintext and encryption when the security feature is enabled.
It does not inject a pretend error into the production write branch or alter file
permissions globally. The intended assertions allow the documented need to
reopen after a committed publication failure; they do not claim transactional
in-memory publication across a failed cache write.

Candidate direction, pending reproduction: propagate fallible insertion through
existing Result paths and accumulate installation failures while continuing the
journaled batch. No inline fallback that could hide a failed append or leave
untracked partial bytes after a successful erase. Pre-journal staging is a larger
alternative: it must bind the segment identity across hydration/compaction while
preserving inline journal payloads; the writer lease permits reads and hydration
uses a different lock. That architectural change is not yet justified.

Next: wait for the other worktree's Rust gate to terminate, run the regression
with Rust 1.99.0 and features server,mcp,conversation-memory,security, preserve
its terminal result and patch, then decide the implementation from evidence.
No paid services, model downloads or weakened assertions/deadlines are allowed.

Caller follow-up: production `insert_batch` is used by hydration, mutation
publication, canonical rebuild and sparse-slot reindexing. The last case reinserts
already-stored chunks and must remain independent of text I/O. `insert` is used
in production by MemoryDocumentStore's already-fallible memorize and import methods; its
other callers are fixtures. A shared infallible indexing step after checked text
staging can serve reindexing without an I/O panic or a fabricated success.

The regression includes both mixed mutations and a repeated document ID. An error
flushing earlier pending documents must not skip processing the repeated-ID
mutation or the later vector/deletion. The existing apply loop already collects
fallible side effects; any refactor must preserve this continuation contract.

Implementation options still unverified: return Result from `stripped` and
`insert_batch`, staging all retained-text appends before publishing document
owners. Extract the existing indexing step so sparse-slot reindexing performs no
text I/O; preserve its existing packed-term sharing behavior. MemoryDocumentStore
can use checked batch insertion through its existing Result API; any convenience
infallible `insert` retained for fixtures must be cfg(test)-only. Hydration and
canonical rebuild must propagate failure before assigning the replacement store.
Move repeated-ID flushing into the outer apply loop if needed so an earlier
flush failure is accumulated while the current and later mutations still run.
A committed error explicitly requires reopen/reconciliation; do not claim the
failed in-memory document publication itself was atomic or complete.

Confirmed the remaining `insert` production caller is MemoryDocumentStore::import
(line 268 in the baseline), which validates IDs, metadata, counts and vectors
before installation and already returns Result. Preserve that validation and its
one-at-a-time insertion order when switching it to checked insertion. Other
matching `store.insert` sites in document search/vector/listing modules are under
cfg(test). Only two direct insert_batch fixtures need Result handling.

Measured baseline (terminal exit 101): four cases (plaintext/encrypted ×
unique/repeated IDs) panic on the actual read-only append. Each fails to advance
the durable revision and skips the later vector/deletion. The same test verifies
full journal recovery after reopening in every case before reporting these
failures. Positive control confirms old retained text remains readable and the
file handle rejects writes. No production code was changed for this baseline.

Candidate implementation: retained-text staging returns Result before document
owners are installed. Slot reindexing reuses the same packed-term sharing and
pure indexing path without repeating text I/O. MemoryDocumentStore retains its
existing insertion order and validation, using checked insertion through existing
Result APIs; the infallible single-insert helper remains only in test fixtures.
Hydration and canonical rebuild check insertion before publishing replacement
stores. The outer mutation loop accumulates pending-batch errors, including a
repeated-ID flush, continues later mutations and reaches revision advancement and
the typed committed-error wrapper. The four original fault cases now pass.

Added acceptance cases cover the final flush through public memorize_documents
in both encryption modes, successful writes after reopen, and preservation of
old document owners when a no-text replacement stages before a later real append
failure. These added cases have not yet run. Existing assertions and deadlines
are unchanged.

Scope extension before the final gate: source review found the remaining
`scrub_replaced` expect, reached when a replacement retains no text. The broader
local pipeline was deliberately terminated (exit 143) during the mutation suite;
it is incomplete, not passed. The two append regressions had passed in that run.
Its partial log and the exact append-only candidate patch are archived. No
assertion or deadline changed. A new regression will exercise both replacement
paths (same/different chunk counts) with actual read-only scrub failures before
changing that path. Reindexing must use the existing install_new path, which
cannot scrub; ordinary replacement must complete owner publication while
returning a scrub error, and later owners/mutations must still be attempted.

Measured replacement-scrub baseline (terminal exit 101): all four combinations
of plaintext/encrypted storage and one/two replacement chunks panic, and every
case leaves the durable revision stale. Reopening recovers both new documents
and the later raw vector. The exact append-only candidate plus the new regression
patch reconstruct this baseline. Ordinary replacement must report the failed
erase while finishing owner/index publication; its data state can then match
replay even though old cached bytes could not be erased. The error remains
mandatory and no successful erasure is claimed.

The four replacement-scrub cases now pass. Both replacement paths finish owner
publication before returning an erase error, the batch continues other owners,
and the outer transaction reports the typed committed outcome after advancing
its revision. For these scrub-only failures, checked reads/search results and
vector counts match replay. Old cached bytes are not claimed erased after the
reported failure. Slot reindexing now calls the existing install_new directly
with unique stored documents, after unchanged packed-term sharing; it performs
no scrub or append. The complete local gate is restarting on this expanded
candidate; the earlier interrupted run remains incomplete evidence.

The expanded library checks passed 16 mutation, 120 document and 12 compaction
tests (one existing external-index census ignored). Integration compilation then
exposed a missed production caller: install_deferred_encoding still used the
single-insert helper, which unit-test compilation had retained under cfg(test).
The pipeline exited 101 before integration tests or Clippy. The exact earlier
source hashes and passing/failing logs are retained in pre-caller-fix-validation.json.
That already-fallible method now returns checked batch insertion directly;
its identity/fingerprint checks and encoded chunk installation are unchanged.
The next gate includes the existing deferred-cache failure/replay tests and the
integration targets, followed by strict Clippy. No assertions or deadlines changed.

Review of hydration's segment ownership confirmed checkpoint overlays retain
inline text and have no attached segment (engine initialization attaches a text
segment only for Opened::Absent). Creating the hydration segment therefore does
not truncate an active overlay text segment; no additional hydration repair was
justified by that hypothesis.
