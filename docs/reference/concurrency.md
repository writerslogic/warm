# One writer, many readers

A store directory has one writer at a time. Any number of other engines, in any process, can
open the same directory read-only while the writer runs. Nothing waits for anything else.

| Open | Rust | Node | Python |
|---|---|---|---|
| Writer | `WarmCore::new` | `new WarmMemorySystem(...)` | `Warm(...)` |
| Reader | `WarmCore::open_read_only` | `WarmMemorySystem.openReadOnly(...)` | `Warm.open_read_only(...)` |
| Pick up new commits | `refresh()`, `has_newer_commits()` | `refresh()` | `refresh()` |
| Release | `close(self)` | `close()`, then `closed` | `close()`, `with Warm(...) as w:` |
| Errors | `StoreLockedError`, `ReadOnlyError` | codes `store_locked`, `read_only`, `store_closed` | `StoreLockedError`, `ReadOnlyError`, `StoreClosedError` (all `ValueError`) |

## Writer

The writer holds an exclusive lock on `<store>/.hms.lock` for its lifetime. A second writer
fails at open, without waiting, with an error naming the holder:

```
WARM store /data/mem is already open by another engine (pid 4121); concurrent writers are not
supported (open the store read-only to read it alongside the writer)
```

`close()` flushes (journal, indices, audit, a due checkpoint) and releases the lock; the lock
is released even if the flush fails, and the error says what did not persist. It is safe to call
twice in Node and Python. After `close()` every call raises a clear error. In Rust `close`
takes `self`, so use after close does not compile. The Node and Python `close()` waits up to
10 seconds for calls still running; if they do not finish it throws and the store stays open,
because releasing the lock under a running write would let a second writer in.

## Reader

A reader takes no lock and writes nothing. It opens the writer's files where they are and
reads them, the way a SQLite WAL reader does: no copy of the store is made, so opening and
refreshing cost no disk space and a refresh costs only what the writer appended. Every
mutation, compaction and checkpoint on a reader returns `ReadOnlyError`. The store must exist:
a read-only open never creates one, and it creates, truncates, removes and renames nothing in
the store directory. A reader of a read-only directory works.

What a reader holds and how it follows the writer:

| Writer file | Reader | Following the writer |
|---|---|---|
| Journal segments (`vectors_data.bin/.../seg_N.bin`) | one open handle per segment, read by position, nothing mapped | `refresh()` walks forward from the last frame it applied; the committed end is the last whole frame whose CRC32 matches, and a frame still being written ends the walk |
| `CURRENT` and the generation it names (compaction) | handles to the generation it opened | `refresh()` sees `CURRENT` changed and opens the new generation once |
| Document checkpoint (`documents.ckpt`) | read once into memory at open, then closed | with the generation |
| `documents.text` | never opened: it is a cache the writer truncates and rebuilds, so a reader keeps chunk text in memory | n/a |
| Dense-vector graph (`dense-vectors/`, `dense-ids.log`) | opened in place: the base files are immutable generations behind `CURRENT`, and the update log is read up to its last whole batch | the new log tail is applied; a consolidation is read once |
| Sparse-vector log (`sparse-docs.wsp`) | read in place | the new records are parsed from the last one applied, after checking that record's checksum is still where it was; a shorter file or a changed checksum (the writer rewrote it) means a whole re-read |
| Experience store | read in place: snapshot plus journal prefix, no recovery writes | re-read when its files change (retried until they hold still across a read) |
| Index caches (`nsg_index*`, `ivf_index*`, `indices.json`) | read if they match the journal it opened; never written | a reader keeps what it trains in memory |
| Keys (`master.key`, `key.check`) | read where they are | n/a |
| Audit, signing and provenance logs | not opened: they record what a writer does | n/a |

`refresh()` returns false when nothing was committed. After a compaction it opens the store
afresh (that reads the compacted generation once, so it costs the size of the store, like any
open). Every other refresh applies only the frames added since, and costs what is new whatever
the size of the store (a unit test pins the bytes read; a release run measured the median
refresh at 3.8 ms, 4.0 ms and 2.7 ms for 21 MB, 172 MB and 1.7 GB stores, with the writer's
index folding disabled so the generation stayed put).

How often a generation changes: only an explicit `compact`, a purge, a `flush` or `close` that
finds a checkpoint due, and index folding once the in-memory index passes `index_memory_budget`
(default 64 MiB). An ingest of about 400 MB at the default configuration published no new
generation. A reader that sees one reopens, which costs the size of the store; the MCP server
runs that reopen on one call at a time and serves the other calls from the engine it has, and
the Node and Python layers swap the engine only after the new one is open. `has_newer_commits()` is the
check without the apply: it proves the new frames whole, so its cost is the size of what is new.
The engine type also offers `refresh_in_place(&self)`, which returns `Refresh::{Unchanged,
Advanced, Reopen}` and is what the Node, Python and MCP layers call: they keep their engine for
the first two and swap it for the third.

Mutable side files, and why they are read in place rather than rebuilt from the journal: the
dense-vector graph and the experience store are not in the journal, so a reader cannot rebuild
them from it. Neither is edited in place in a way a reader cannot follow: the dense graph's base
is immutable and replaced by `CURRENT` (a reader maps those base files, since a base never changes while it is current; the writer unlinks an old base best-effort and collects any it could not remove later, so on Windows a base a reader still maps is removed after the reader lets go, and it is unlinked, not scrubbed), and its log and the experience journal are append-only
with CRC-checked frames, so a reader reads a prefix of whole records. If a reader cannot open
the dense graph it logs a warning and answers `query_vector` from the sparse path, as a store
without one does.

## What a reader can observe

Every journal-backed state a reader sees is a state the writer committed, never a mix of two:
documents, memories, relations and sparse vectors all live in the journal, and one `commit`
(one `memorize_documents` batch, one `remember`) is one journal frame. The sub-stores the
writer updates after the journal commit (the dense graph, the sparse log, the experience store)
can lag or lead the journal a reader holds, exactly as a crash would leave them. Open already
handles that: a dense hit whose id the reader's journal does not hold is dropped, and dense
search falls back to the sparse path for a vector the graph lacks. This rests on the commit
protocol, which the tests below exercise:

- The journal is append-only. A frame counts only if its CRC32 matches and it lies below the
  committed end, and frames are written in order, so the prefix below the committed end is a
  committed state. The committed end is a 20-byte word (`committed_end`, next to the segment
  files: segment, offset, CRC32), written by the writer only after the frame's sync succeeded
  and never scanned past by a reader. A store from before the word has no such file and is
  read to its last whole frame; a writer that predates the word does not maintain it.
- Sealed segments and generation files never change. A reader walking into a segment sealed
  since its last refresh moves to the next one.
- A compaction publishes a new generation by replacing `CURRENT`, then deletes the old one. A
  reader's open handles keep the old generation readable after the delete (a Unix unlinked
  file; on Windows every handle is opened with `FILE_SHARE_READ | FILE_SHARE_WRITE |
  FILE_SHARE_DELETE`, so the writer's rename and delete are not refused, and nothing is
  mapped, since a mapped file cannot be deleted on Windows). The reader goes on serving what
  it applied until it refreshes.

Limits, stated plainly:

- A reader holds the whole applied state in memory: documents (text included), vectors and
  indexes. Open replays the journal, so it costs the size of the store in time and memory;
  only refresh is incremental. A reader does not serve from a mapped checkpoint, because a
  scrubbing compaction overwrites the checkpoint file in place.
- A commit whose durable sync fails is rolled back by the writer by rewriting the frame's
  bytes. A reader never scans past the committed-end word, which the writer advances only
  after the sync, so it never applies that frame. A reader whose recorded position is not a
  frame boundary below the committed end (the log was truncated under it) reopens on its next
  refresh instead of stalling.
- Erasure. A scrubbing compaction (`forget` with a purge) zero-fills the old generation
  through the same inode before unlinking it, so on Unix the erased bytes are gone from the
  disk at once, including from a reader's open handle (a read through it then fails its
  checksum). What a reader has already applied lives in its memory until it refreshes (it
  opens the new generation and drops the old state and handles) or closes. The bound: erased
  data is gone from every process once every reader has refreshed or closed. On Windows a
  reader's open handle can keep the old generation's directory from being removed until the
  reader lets go; the files are scrubbed first, and the writer collects the directory at its
  next compaction.
- A reader holds no lock, so it cannot stop a writer from compacting or purging, and it does
  not see uncommitted work.
- Provenance, signing and audit are the writer's: a read-only engine has no provenance
  manager and no audit log.

## Locking semantics

The lock is `std::fs::File::try_lock` on `.hms.lock`, never a blocking call.

| | Unix (Linux, macOS) | Windows |
|---|---|---|
| Primitive | `flock(LOCK_EX \| LOCK_NB)` (Android: the same through `libc`) | `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK \| LOCKFILE_FAIL_IMMEDIATELY)` |
| Kind | advisory, per open file description | mandatory, per handle |
| Two handles in one process | conflict | conflict |
| Holder pid in the error | read from the file | usually unavailable: reads of a locked file are denied, so the error says "holder unknown" |
| Process death | the OS drops the lock | the OS drops the lock |
| Hard links, symlinks, FIFOs | refused (the lock file must be a regular file with one link) | links refused |

`flock` locks are advisory and unreliable on some network file systems; keep stores on local
disks. Readers do not depend on locks at all, which is why they work the same on every
platform. The lock tests use two handles in one process, which both primitives reject, and
real child processes. The Windows column and the Windows reader share modes are exercised by
the `test (windows-latest)` CI job only (the step "Reader share modes" and the full suite);
they were not run on the machine this was written on.

## Command line and MCP

- `warm-mcp --read-only` now also opens every store read-only, so it runs beside a writer and
  picks up its commits: the first call after a commit applies the new frames in place (after
  a compaction it opens the store afresh). It refuses a namespace whose store does not exist instead of
  creating it. Without the flag `warm-mcp` is a writer and a second one fails with
  `store_locked`. A downstream app that ingests with a CLI should run its server with
  `--read-only`, or `close()` the store before the ingest.
- Any tool that only reads (query, consistency checks) should call `open_read_only`; ingest and
  anything else that writes still needs the writer lock.
- `warm-admin inspect` and `verify`, and `warm-format verify`, never took the writer lock and
  still do not.

## Tests

`tests/shared_readers.rs` re-executes the test binary as real processes: a writer committing
document pairs while three readers refresh in a loop and check that each pair is visible whole
or not at all; a reader open after `SIGKILL` of the writer mid-commit (the shared store is
byte-identical before and after the open, and the next writer recovers); a refresh seeing new
commits across a compaction; a second writer refused with the holder's pid while readers open;
`close()` releasing the lock in a process that stays alive; the dense graph followed in place;
and a deterministic replay of one commit's exact bytes into a store prefix by prefix, where
every incomplete prefix reads as the old state and `has_newer_commits` stays false until the
frame is whole.

`src/core/engine/reader.rs` holds the tests of the in-place design:

- a reader holding handles while the writer compacts and deletes the old generation (the
  reader still reads it; a later compaction clears it once the reader is gone);
- a scrubbing compaction reaching a reader's open handle (Unix);
- a segment renamed and deleted under a reader handle, which on `windows-latest` is the
  share-mode contract;
- refresh cost: the same new commit costs the same bytes read beside a store of 100 documents
  and one forty times larger.

`cargo test --release --test reader_scale -- --ignored --nocapture` measures refresh time
beside a 10 MB and a large (default 1 GB) journal; set `WARM_READER_SCALE_MB` to change it.
