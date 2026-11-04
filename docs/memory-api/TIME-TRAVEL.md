<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Time travel over the journal

A historical view is read-only. It shows the document and raw-vector state that the journal
held at an earlier point. It is opt-in: no existing query reads history, and `search_documents`,
`recall` and the memory API's `as_of`/`known_at` keep their current behavior. Recall in
LongMemEval was lower with as-of retrieval (recall_any@5 0.76 against 0.88 without it), so
history is something a caller asks for and never a default.

Rust API (`WarmCore`): `journal_position`, `history_bounds`, `history_view`,
`history_view_with`, `history_view_at_time`, `resolve_history_time`, `history_revisions`,
`set_history_cache_budget` and `clear_history_cache`.
The types are `JournalPosition`, `HistoryView`, `HistoryOptions`, `HistoryRevision` and
`HistoryError`, all exported from the crate root.

## Positions

A **position** is `(generation, offset)`. `generation` names the journal generation, the
directory that [`CURRENT`](../file-format/JOURNAL.md#compaction) points to (or the store
directory for a journal that has never been compacted). `offset` is a global journal offset that
falls on a frame boundary. The state at a position is the result of applying, in order, every
frame that starts before `offset`. Each frame is one transaction, so a position never splits a
transaction.

- Valid offsets are `0` and every offset at which a frame ends (that is, where the next frame
  starts, after any skip to the next segment). `journal_position()` returns the current end.
- An offset inside a frame, or at the unused tail of a non-final segment, fails with
  `HistoryError::NotFrameBoundary`.
- An offset past the current end fails with `PastEnd`. The view does not wait for future
  writes.
- A position from another generation fails with `GenerationNotRetained`. Offsets restart in
  every generation, so an old position is never reinterpreted against new data.

**Oldest retained position.** In a journal that has never been compacted, history starts at
offset `0`, the empty store, and survives restarts. A compaction writes the live state as a
run of synthetic frames and then the tail written meanwhile. A reader cannot see where those
synthetic frames end, so in a compacted generation history starts at the end the generation had
when this engine opened or published it. Anything earlier fails with `BeforeRetained`.
`history_bounds()` reports `(oldest, latest)`. Any compaction ends the retained history: an
explicit `compact`, a purge, governance, or the checkpoint that closing a store writes when
`document_checkpoint` makes one due.

A generation that begins with a document checkpoint (`documents.ckpt`) has no replayable
history. The checkpoint's text lives in `documents.text`, and a later delete zeroes that text,
so the state between the checkpoint and the delete cannot be rebuilt exactly. Every history
call on such a generation fails with `CheckpointBase` and never returns approximate state.

## As of a wall-clock time

Frames carry no timestamp. The `seq` field is advisory, and journal readers ignore it. The only
wall-clock time the journal records is the `ingest` time in a memory-API stamp
(`mem1:` version or `_mem` metadata; see [README.md](README.md#concepts)). That time comes from
the memory API's injectable `Clock`. Audit timestamps use a different clock and are not used.

- The **stamp** of a frame is the largest `ingest` among the stamped documents it writes. A
  frame without one is *unstamped*.
- The **effective time** of a stamped frame is the running maximum of the stamps up to and
  including it. A clock that steps backwards (skew between processes, an NTP correction, a
  `ManualClock` set back) cannot move a frame earlier than one that the journal already
  ordered before it. A clock that jumps forward pushes every later frame to at least that
  time. Those frames stay reachable by position, but not by any time before the jump.
- **Anchors** are the end of each stamped frame, at its effective time, plus position `0` at
  minus infinity in a never-compacted journal.
- `as of T` resolves to the anchor with the latest position whose time is `<= T`. Unstamped
  frames after that anchor are excluded, because nothing in the journal proves they were
  written by `T`. Unstamped frames before the anchor are included, because they precede a frame
  stamped `<= T` in commit order.
- If no anchor has a time `<= T` (for example a compacted generation, or a store with no
  memory-API writes), the call fails with `NoTimeAnchor`. If the anchor is older than the oldest
  retained position, the call fails with `BeforeRetained`. Neither case falls back to the
  current state.

`resolve_history_time(T)` returns the position and the anchor's effective time, so a caller
can see what the time mapped to. This is knowledge time (when the store learned something),
not valid time. To ask what a fact said about the world at a time, use `recall` with `as_of`.

## Intervals

`history_revisions(from, to, limit)` lists every revision that is live at any position in
`[from, to]`. Both ends must be valid, retained positions in the live generation, with
`from <= to`. A revision is one write of a document (or of a raw vector) and lasts until the
next write or delete of the same id. Each entry carries:

- `kind` (`document` or `vector`), `id`, and `version` (for documents);
- `written`: the position after the frame that wrote it, or `null` when that frame ends at or
  before the oldest retained position of a compacted generation (its real write is in a
  generation that is gone);
- `ended`: the position after the frame that replaced or deleted it, when that is `<= to`;
- `deleted`: whether `ended` was a delete.

A revision written and replaced inside one transaction is never live at a position and is not
listed. More than `limit` revisions (at most 100 000) fail with `TooManyRevisions` and never
return a truncated list. To read a revision's content, open a view at its `written` position.

## Deletes and purges

- **A delete is history, not erasure.** Without a purge, the journal keeps a deleted document's
  frame until the next compaction ([README.md](README.md#forgetting)). A view at a position
  before the delete returns that document, with its text and embeddings, exactly as the store
  held it. This also applies to facts that `forget` removed without `purge`. A caller that must
  not see forgotten content in history must purge.
- **A purge removes the history.** A purge is a scrubbing compaction. It publishes a new
  generation that holds only the live state and overwrites the old generation with zeros.
  Purged text and embeddings are never returned at any position, including positions from
  before the purge, for three reasons:
  1. a position from before the purge names the old generation and fails with
     `GenerationNotRetained`;
  2. no position in the new generation can show the purged documents, because its frames never
     held them, and positions inside the compaction's synthetic prefix are not retained;
  3. a `HistoryView` built before the purge holds decoded state in memory. Every read takes the
     compaction lock and checks that the view's generation is still live, so after any
     compaction the view fails with `Compacted` and returns nothing more. A view is built
     under the same lock, so a compaction cannot swap the generation while frames are being
     read. Views kept under an opt-in reuse budget (see [Cost and limits](#cost-and-limits))
     are dropped
     before any compaction publishes its generation.
- Bytes a caller already received before the purge are outside any API's control, as in
  [README.md](README.md#forgetting).

## What a view contains

- **Documents**: `Document`, `DeleteDocument` and `EncodeDocument` replayed with the store's
  own rules. The revisions live at the position are then indexed in one bulk insert into a
  private in-memory document store, which reaches the same answers as installing them frame
  by frame. `get_document`, `list_documents`,
  `document_count` and `search_documents` read it. Search uses the built-in lexical and exact
  dense path, the same ranking code the live store uses. It applies no trained dense index,
  installed model stages or local text embedder; pass `SearchOptions::embedding` for semantic
  search. At the current position, results match a live `search_documents` that uses that
  path.
- **Raw vectors**: `Vector` and `Delete` (with the derived children that `Delete` removes),
  through `get_vector` and `vector_ids`.
- **Not replayed**: meaning memory (atoms, composites, triples, rules), relations, and the
  audit queue.

## Cost and limits

Building a view decodes every frame from offset `0` to the position, one frame at a time, and
validates each mutation with the same checks that replay uses. A frame that is damaged, fails
to decrypt or fails validation fails the whole call. No partial view is returned. The
document revisions live at the position are then indexed in one bulk insert: the index states
in between are never built. Memory is the decoded state at the position plus one frame, and
then the view's index. `HistoryOptions::max_state_bytes` (default 1 GiB) bounds the estimated
state, and a state that exceeds it at any frame fails with `StateTooLarge`. A time query
decodes the journal twice: once to resolve the time and once to build the view (the second
decode is skipped when the resolved position is kept under the opt-in reuse below). Building a view never
writes anything. It does not touch the live documents, shards, text segment, indices or audit
queue, and it blocks a compaction only while it builds or reads.

**Reuse (opt-in, off by default).** By default nothing outlives the caller's views: every
open builds, and a dropped view frees its state. `set_history_cache_budget(bytes)` opts in:
the store then keeps the last 4 views it built within `bytes` of estimated state, and opening
a view at one of those positions shares it instead of building. `0` turns reuse off again and
releases every kept view. Positions are immutable within a generation, so a shared view is
the state a build would produce. Consequences of opting in:

- That state, plus each kept view's index (not counted in the estimate and not measured),
  stays resident after the caller drops its views, until a compaction publishes (every
  compaction, a purge included, empties the cache before publishing), the caller calls
  `clear_history_cache`, or the budget is set to `0`.
- Reopening a kept position does not read the journal again, so damage to the journal after
  the first build is not detected at that position. Any position not kept is built and
  validated as above.
- `max_state_bytes` is still enforced on a reused view: it fails with `StateTooLarge` exactly
  when building it under that bound would.

Measured cost against a current-state query, from `benchmarks/results/history_view_cost_v2.json`
(`examples/history_cost.rs`, Apple M4, shared machine with load averages 6 to 13, 31 paired
repetitions, median of the per-repetition ratios). The corpus is ingested in batches of 100,
followed by single-document updates of a tenth and deletes of a twentieth. *Default* is the
shipped setting: no reuse budget, so every open builds the view from the journal. Each open is
followed by one search. Only these two scales were tested:

| ingested | journal bytes | live search p50 | default open + search at the end p50, factor | default at the middle p50, factor | search on a built view, factor |
|---|---|---|---|---|---|
| 1 000 | 172 637 | 63 us | 9.7 ms, 155x | 5.1 ms, 83x | 0.42x |
| 10 000 | 1 739 295 | 141 us | 71.8 ms, 523x | 35.2 ms, 247x | 0.61x |

Against the frame-at-a-time builder (`benchmarks/results/history_view_cost.json`, whose
factors were 813x and 158x at 1 000 and 4 246x and 790x at 10 000, end and middle), the
paired factor of a default open is 5.26x and 1.89x lower at 1 000 and 8.12x and 3.2x lower at
10 000 (`versus_v1` in the v2 file). The gain is largest where the journal holds superseded
revisions: the end position follows the single-document update and delete frames (1 500 at
10 000) that the old builder indexed one at a time, while the middle position is batched
ingest only, so its remaining cost is decoding and validating the frames plus one bulk index
of the survivors. A build still grows with the journal replayed.

**Opt-in reuse, measured.** With `set_history_cache_budget(256 MiB)`, reopening a kept
position costs about as much as a search: open + search over live search is 1.21x and 1.02x
(end, middle) at 1 000 and 0.97x and 0.88x at 10 000, from the same file. These figures apply
only when a caller opts in, keeps the memory resident, and reopens a kept position, which
skips the journal re-read and its damage check (see Reuse above).
