<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Erasure

`forget` deletes with `purge: true` by default. When it returns, no file in the store
directory holds the deleted text, and if the process dies at any point during it, the next
open finishes the job before returning the store.

## What remains with `purge: false`

A logical forget scrubs the text segment (`documents.text`) and removes the facts from every
query, and leaves the deleted frames in the append-only journal segments
(`vectors_data.bin/gen-*/seg_*.bin`). `tests/erase.rs::a_logical_forget_leaves_the_text_only_in_the_journal_segments`
asserts that these are the only files that hold the text, and that a later purge removes it.
Historical views ([TIME-TRAVEL.md](TIME-TRAVEL.md)) read these frames.

## What the default `forget` is tested to do

| Claim | Test |
|---|---|
| No file under the store directory holds the text (a 10-byte fragment of a unique token) after `forget`, and after reopen | `tests/erase.rs`, `tests/purge.rs` |
| Same for a superseded chain and for replaced text | `tests/erase.rs`, `tests/purge.rs` |
| A crash at every durability point of the purge, then a reopen: a document the store no longer holds is in no file; an acknowledged forget removed everything; `purge.pending` is gone | `tests/erase.rs::crash` (`failpoints`; Linux, macOS, Windows in CI) |
| The bytes of an ext4 image (file data and the filesystem journal) hold no marker after a purge and after a purge cut by a crash and finished at reopen; a control forgotten with `purge: false` is still found, so the scan can see a remnant | `scripts/ci/raw-image-erase.sh` with `tests/raw_image.rs` (Linux CI only) |

The scan of the raw image covers filesystem-level remanence on that filesystem: freed blocks,
the ext4 journal and slack. It does not cover a solid-state drive's flash translation layer,
which remaps writes, so an overwritten page can survive in an unmapped cell until the drive
erases it; copy-on-write and snapshotting filesystems (APFS clones and snapshots, btrfs, ZFS);
swap and hibernation images; backups; or bytes a caller already holds.

## A pending purge blocks open

If an open finds `purge.pending`, it completes the purge before returning the store. That is
deliberate: the store must not be handed out while deleted text is still on disk. If the
compaction cannot run (for example the disk is full), the open fails with the cause until the
condition is fixed. Known gaps: a scrubbing compaction that starts between another thread's
`mark_purge_pending` and its delete commit can remove that marker, and the server and MCP
delete paths leave a harmless marker (one extra compaction at the next open) when nothing was
deleted.

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
