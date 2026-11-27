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

A reader never takes `.hms.lock` and never writes to the store. It copies a consistent
snapshot of the committed state into `<store>/.hms.readers/<id>/` and runs an ordinary engine
over the copy, so recovery work on open (tail repair, caches, checkpoints) cannot touch the
writer's files. Every mutation, compaction and checkpoint on a reader returns `ReadOnlyError`.
The store must exist: a read-only open never creates one. Pass a snapshot directory
(`WarmCore::open_read_only_at`) when the store directory is not writable for the reader.

A reader sees one snapshot until `refresh()`, which returns false when nothing was committed
and otherwise swaps to a newer snapshot; the old one stays usable until the swap completes.
`has_newer_commits()` is the cheap check (a directory listing and a read of the journal's end).
A reader's snapshot directory is removed when it closes or is dropped; directories left by a
killed reader are removed by the next reader or `open_read_only` call after a minute.

## What a reader can observe

Every journal-backed state a reader sees is a state the writer committed, never a mix of two:
documents, memories, relations and sparse vectors all live in the journal, and one `commit`
(one `memorize_documents` batch, one `remember`) is one journal frame. Sub-stores the writer
updates after the journal commit, the dense-vector graph and the experience store, can lag the
journal in a snapshot exactly as a crash would leave them; open already handles that state
(dense search falls back to the journal's sparse path for a vector the graph lacks). This rests
on the commit protocol, which the tests below exercise:

- The journal is append-only. A frame counts only if its CRC32 matches, and frames are written
  in order, so the longest prefix of valid frames is a committed state. A frame still being
  written is dropped from the copy.
- Sealed segments never change. Every other file is replaced atomically (temp file, sync,
  rename) or belongs to a sub-store with its own recovery. The copy lists them before and after
  copying and retries the whole snapshot (8 attempts, backing off) if any changed. They are
  copied before the journal, so none is newer than the journal.
- A compaction publishes a new generation by replacing `CURRENT`. The pointer is read before
  and after the copy; a change retries.
- A writer that commits continuously can make a snapshot of a large dense-vector or experience
  store fail with "kept changing"; retry. Journal commits alone never cause this.

Limits, stated plainly:

- A snapshot costs a copy of the store's files (disk space and open time proportional to the
  store, removed with the snapshot). Reopening after `refresh()` replays the journal again.
  Key files (`master.key`, signing and provenance keys) are not copied: the reader reads them
  from the store, so a store whose key file is unreadable to the reader cannot be opened.
- A commit whose durable sync fails is rolled back by the writer. A reader that copied it in
  that instant can see it until its next refresh. The writer reported that commit as failed.
- Erasure: a scrubbing compaction (`forget` with a purge) zero-fills the old generation. A
  reader's private copy keeps the erased data until it refreshes or closes. Close readers, or
  refresh them, to finish an erasure.
- A reader holds no lock, so it cannot stop a writer from compacting or purging, and it does
  not see uncommitted work.

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
real child processes. The Windows column, and snapshotting a store whose writer holds its
files open on Windows, are exercised by the Windows CI job only; they were not run on the
machine this was written on.

## Command line and MCP

- `warm-mcp --read-only` now also opens every store read-only, so it runs beside a writer and
  picks up its commits: the first call after a commit takes a fresh snapshot (a copy of the
  store, see the limits above). It refuses a namespace whose store does not exist instead of
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
`close()` releasing the lock in a process that stays alive; and a deterministic replay of one
commit's exact bytes into a store prefix by prefix, where every incomplete prefix reads as the
old state and `has_newer_commits` stays false until the frame is whole.
