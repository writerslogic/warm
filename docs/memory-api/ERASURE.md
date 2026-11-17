<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Erasure

`forget` purges by default. Two modes, chosen by `synchronous`:

| Mode | Option | When `forget` returns | Text is in no file of the store |
|---|---|---|---|
| Deferred (default) | `purge: true`, `synchronous: false` | The delete and a durable `purge.pending` ticket are on disk. | After the next scrubbing compaction ends: `compact` or `compact_scrubbing`, the call that makes the 64th waiting purge, or the next open. One such compaction erases every deferred delete. `purged` is `false` until then. |
| Synchronous | `purge: true`, `synchronous: true` | A scrubbing compaction has finished. | On return. `purged` is `true`. |
| Logical | `purge: false` | The delete is committed; the text segment is scrubbed. | Not until some later compaction (see below). |

The guarantee of the default mode, exactly: after `forget` returns, a crash at any point leaves
the deleted facts deleted and a `purge.pending` ticket on disk; the next open that can run a
compaction erases the text before handing the store out. Until a scrubbing compaction ends, the
deleted text is still in the journal segments (and a historical read can return it); `purge_status()`
reports how many purges wait. Callers that need the text gone before the call returns pass
`synchronous: true` (`?synchronous=true` on `DELETE /api/v1/documents/{id}`); the MCP `forget`
tool is always synchronous (one compaction per call, however many ids it removes).

Why deferred is the default: a scrubbing compaction rewrites the store, so a synchronous forget
costs time that grows with the store (see "Cost" below).

## The ticket ledger

`purge.pending` is a ledger, not a flag. Each purge takes a ticket (a number) before its first
delete commits and the file is rewritten durably; a scrubbing compaction removes exactly the
tickets whose deletes had committed before it took its snapshot. A purge that finishes therefore
never removes the ticket of another purge whose delete has not committed
(`tests/erase.rs::a_finished_purge_leaves_the_ticket_of_another_in_flight`). Concurrent
synchronous purges share compactions: a purge whose ticket another thread's compaction already
cleared returns without compacting again.

## What remains with `purge: false`

A logical forget scrubs the text segment (`documents.text`) and removes the facts from every
query, and leaves the deleted frames in the append-only journal segments
(`vectors_data.bin/gen-*/seg_*.bin`). `tests/erase.rs::a_logical_forget_leaves_the_text_only_in_the_journal_segments`
asserts that these are the only files that hold the text, and that a later purge removes it.
Historical views ([TIME-TRAVEL.md](TIME-TRAVEL.md)) read these frames.

## What the default `forget` is tested to do

| Claim | Test |
|---|---|
| After `forget` with `synchronous: true`, and after reopen, no file under the store directory holds the text (a 10-byte fragment of a unique token) | `tests/erase.rs`, `tests/purge.rs` |
| Same for a superseded chain and for replaced text | `tests/erase.rs`, `tests/purge.rs` |
| A crash at every durability point of the purge, then a reopen: a document the store no longer holds is in no file; an acknowledged forget removed everything; `purge.pending` is gone | `tests/erase.rs::crash` (`failpoints`; Linux, macOS, Windows in CI) |
| A purge waiting at a full disk does not block open, is reported pending, and is finished by a later open | `tests/purge_disk_full.rs` |
| A deferred forget leaves the text in the journal until `compact`, reopen or the 64th waiting purge, which erase it | `tests/erase.rs::a_deferred_purge_is_durable_and_erased_by_compaction_open_or_batch` | |
| The bytes of an ext4 image (file data and the filesystem journal) hold no marker after a purge and after a purge cut by a crash and finished at reopen; records that were not forgotten are found in the image, so the scan can see live text (a control forgotten with `purge: false` is reported but not required: the first CI run found no remnant of it in the image, which is not explained yet) | `scripts/ci/raw-image-erase.sh` with `tests/raw_image.rs` (Linux CI only) |

The scan of the raw image covers filesystem-level remanence on that filesystem: freed blocks,
the ext4 journal and slack. It does not cover a solid-state drive's flash translation layer,
which remaps writes, so an overwritten page can survive in an unmapped cell until the drive
erases it; copy-on-write and snapshotting filesystems (APFS clones and snapshots, btrfs, ZFS);
swap and hibernation images; backups; or bytes a caller already holds.

## A pending purge at open

An open that finds `purge.pending` runs the scrubbing compaction before returning the store.
If it cannot (the disk is full, an I/O error), the open still succeeds: every acknowledged
document is readable, the deleted facts stay deleted, the tickets stay on disk, a warning is
logged, and `WarmCore::purge_status()` reports `pending > 0` with the error. The store is then
not purged: the deleted text is still in the journal and `forget` does not claim otherwise. A
later open, `compact_scrubbing` or `compact` with space available finishes it
(`tests/purge_disk_full.rs`, which injects ENOSPC at the first durability point of the purge).
The alternative, refusing to open, would make a full disk lock the user out of data that is
intact.

The server and MCP delete paths can leave a ticket (one extra compaction at the next open)
when a call fails between marking and deleting.

## Cost

`examples/forget_cost.rs`, release build, wl2 (22-thread Linux host, ext4, timing lock held, no
other load), 200 single-document forgets per cell, then 100 documents per call (the batch rows
are two calls, so they show the order of magnitude only). Milliseconds.

| Store size | Mode | Median | p99 | 200 deletes, total |
|---|---|---|---|---|
| 1,000 | logical (`purge: false`) | 3.2 | 4.9 | 0.6 s |
| 1,000 | synchronous purge | 77.7 | 100.0 | 15.6 s |
| 1,000 | deferred (default) | 9.5 | 76.0 | 2.1 s |
| 10,000 | logical | 3.2 | 5.9 | 0.7 s |
| 10,000 | synchronous purge | 202.3 | 253.8 | 40.4 s |
| 10,000 | deferred (default) | 9.6 | 175.1 | 2.4 s |
| 100,000 | logical | 3.4 | 14.9 | 0.7 s |
| 100,000 | synchronous purge | 1,926.9 | 2,021.9 | 374.5 s |
| 100,000 | deferred (default) | 10.1 | 1,022.9 | 5.2 s |

A synchronous purge costs a compaction per delete, so it grows linearly with the store (about
19 ms per 1,000 documents). The deferred mode pays the durable ticket (about 6 ms over a logical delete)
per delete and one compaction per 64 purges: its p99 is the call that runs that compaction (3 of
every 200 deletes), and it does not grow in the median. One call that forgets 100 documents costs
78 / 189 / 1,029 ms synchronously and 13 / 15 / 31 ms deferred (1k / 10k / 100k documents).
Raw rows: `benchmarks/results/forget_cost_v1.json` (`sync` there is the synchronous purge,
which was the default before the deferred mode).

## Directory durability on Windows

`sync_directory` opens the directory with `FILE_FLAG_BACKUP_SEMANTICS` and `GENERIC_WRITE` and
calls `FlushFileBuffers` on it. Microsoft's `FlushFileBuffers` reference documents the flush
for files, volumes and pipes and says nothing about directory handles, so this is a best effort:
the guarantee that a new file or rename survives power loss rests on NTFS metadata journaling
and on the flush of the file's own data, not on a documented directory flush. A filesystem that
refuses the flush (access denied, not supported) is logged and skipped. Unix uses `fsync` on the
directory, which POSIX specifies. A purge's ordering (the ticket is durable before the delete,
the scrub finishes before the ticket is removed) holds on Windows for a killed process, which
the crash tests exercise on Windows; for power loss it holds only as far as NTFS's journal does.

## Media-level erasure needs crypto-erase

The only software answer to the flash layer is to keep the data encrypted under a key that is
destroyed on delete. The branch `feat/encrypted-by-default` has `src/core/sealed`: a block
container with per-segment data keys, a key table (`keys.tbl`), and
`SealedStore::purge_segment`, which drops the segment's key, re-wraps the remaining keys and
leaves the segment unreadable after a crash at any step. It is not connected to the engine
(nothing outside `src/core/sealed` uses it). Until it is, a delete is a file-layer erase.
What remains for "delete destroys the key" to be true:

1. Route every persisted byte of text and vectors (journal arena segments, `documents.text`,
   the checkpoint image, vector snapshots and graph files, index caches) through sealed blocks
   with one data key per segment generation.
2. Make a purge write the surviving state to a new generation under a new key, make the key
   table durable (file and directory sync), then call `purge_segment` on the old one; on open,
   collect leftovers of an interrupted purge (the `purge.pending` marker added here is the
   trigger).
3. Rotate the master key after a purge, or keep wrapped keys out of backups and snapshots: an
   old copy of `keys.tbl` on flash still unwraps the old data key while the master key is the
   same.
4. A plaintext store (the opt-out) cannot be crypto-erased; `forget` there stays a file-layer
   erase and the documents must say so.
5. Tests: scan the raw image of an encrypted store for plaintext and for the destroyed data
   key, and a crash at every durability point of the key-table update.
