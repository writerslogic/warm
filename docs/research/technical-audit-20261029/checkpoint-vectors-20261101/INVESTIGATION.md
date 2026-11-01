<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Checkpoint vector maintenance

Baseline: `d24cdb307ba0ca2bc1474e0063ebd58202f04180` (PR #372).

Source-supported findings:

- `documents::image::representable` only admits lexical chunks with no embedding,
  vector indices or word positions. `WarmCore::checkpoint_accepts` already relies
  on this invariant: every embedding and pending encoding belongs to the live
  overlay. Overlay documents retain inline text until hydration.
- `document_embedding_count`, `document_index_trained`, `vector_index_status`,
  `rebuild_vector_index_locked` and `maintain_indices` nevertheless call the
  private infallible `documents()` hydration helper. `train_dense_index` hydrates
  fallibly. Both dense and vector captures only use embedded live documents.
- `compaction_snapshot` explicitly hydrates fallibly before capture. Its later
  deletion check redundantly uses the infallible helper. `open_checkpoint` is the
  only installer of a serving image and runs while constructing the engine;
  compaction does not switch an existing engine back to checkpoint serving.
- The helper's callers and history were checked before removal. Explicit
  hydration remains in operations that require the complete lexical store,
  including verification and compaction.

Chosen mechanism: capture/status/train directly from the existing live store,
using the established checkpoint invariant. Changing these status APIs to Result
and hydrating fallibly would preserve unnecessary full-corpus work and would
couple embedding-only operations to unrelated checkpoint metadata. Introducing
another embedding mirror would duplicate ownership and synchronization.

Regressions cover a CRC-valid malformed source flag in a closed checkpoint,
zero and one live embedding, vector thresholds 0/1/2 and dense thresholds
127/128/129, replay, replacement/deletion, forced hydration and result equivalence.
No mapped checkpoint is edited. Compilation or source inspection alone is not a
passing regression or a latency/RSS measurement.

Separate finding, outside this candidate: `DocumentStore::stripped` still uses
`expect` when appending retained text. `commit_payload_at_revision` appends the journal before `apply_mutations`, and
increments the revision only after that function returns. A panic during the
append-to-text step can therefore bypass the typed committed-error wrapper and
revision advancement (source inference, not yet reproduced). `insert_batch`
collects stripped documents before installing their lexical owners, whereas the
shard installation already precedes it. A dedicated write-path fault-injection
and recovery investigation is required; this is not fixed or measured here.

Measured baseline: all seven operations panic for both zero and one embedding
(14 caught panics). The corrected valid-checkpoint fixture fails because status
and maintenance hydrate the store at zero embeddings. The candidate removes the
private infallible helper after replacing all its callers; both targeted regressions pass. The broader local gate so far passes
26 integration tests and 37 library tests (one existing diagnostic ignored
because it requires WARM_MEMORY_INDEX); compaction and Clippy are still pending.

Write-path follow-up constraints from source: `install_documents` is also called
between mixed mutations and repeated document IDs; replay uses the same
`apply_mutations` machinery. The writer lease serializes transactions while
allowing reads until publication, but checkpoint hydration currently synchronizes
through its own mutex and document lock. Any pre-journal text staging must bind
the segment identity across hydration/compaction and preserve inline journal
payloads. A fallback to inline text must account for partially written cache
bytes during erasure. These are design constraints, not an implemented fix.

All 12 compaction tests pass, including interruption and concurrent-tail recovery.
The first strict Clippy run rejected `rows <= document_count - 1`; the fixture
now uses the equivalent `rows < document_count` assertion. No threshold or
expected result changed. Strict Clippy and the two focused regressions are
rerunning after that test-only rewrite.

Final local gate: strict all-feature/all-target Clippy passes, and both focused
regressions pass again after the equivalent assertion rewrite. Scoped rustfmt
and diff checks pass. Source hashes match the recorded candidate. No production
code changed after the broader 26 integration, 37 library and 12 compaction
passes. The one ignored census diagnostic remains explicitly excluded.
