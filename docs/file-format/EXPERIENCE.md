<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Experience snapshot (`experience.wexp`)

Written by `Experience::save` into a directory the caller chooses, and by a store (`WarmCore`)
into its own directory with the journal below. Always built; the `experience` feature name is an
accepted no-op.

## Header

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | magic `WEXP` |
| 4 | 1 | version, currently 3; versions 1 and 2 also open |
| 5 | 3 | zero |
| 8 | 8 | payload length, little-endian |
| 16 | 4 | CRC32 (IEEE) of the payload, little-endian |
| 20 | n | payload |

A reader rejects a wrong magic or version, a length that is not the file's remaining bytes or is
over 1 GiB, and a checksum mismatch. A damaged file is an error; it is never read as an empty store.

## Payload

CBOR of one map with these arrays. Each array is indexed by slot, and a freed slot is `null`, so
identifiers (symbols, nodes, templates, frames, regions, episodes) are the same after a reload.

| Field | Entry |
|---|---|
| `symbols` | UTF-8 string |
| `nodes` | `[role, name, props, children]`: three symbol slots and child node slots |
| `templates` | `[url template symbol, title template symbol]` |
| `frames` | sorted, distinct line symbols of one observation |
| `regions` | breadcrumb, page title, URL template, rendered text, and 1 to 12 variants (each the node slots of the run of sibling subtrees) |
| `episodes` | id, goal, outcome, and steps: step index, root node, template, frame, optional action-label symbol, and `[region, count]` pairs; version 2 adds `handles`, the sorted `[handle, label]` pairs of the episode's latest observation; version 3 adds `controls` to a step that has form controls (absent when it has none) |
| `seq` | version 2: the last journal sequence number folded into this snapshot (0 from `Experience::save`) |

Version 2 adds `seq` and `handles`; a version 1 file has neither, opens with sequence 0 and empty
handle maps, and is rewritten as version 2 by the next checkpoint. A writer never produces version 1.

Version 3 adds the form controls of a step (see [Form state](../concepts/experience-core.md#form-state)):
each a map with `role`, `name`, the optional state strings `value`, `checked`, `selected`,
`expanded` and `pressed`, the optional flags `required`, `disabled` and `readonly`, up to 25
`options` (`label`, `selected`) and `moreOptions`, the count of options beyond them. A writer
produces version 3 only when some step has controls and version 2 otherwise, so a store that
never stores controls writes the same bytes as before and stays readable by older releases; once
it does, an older release refuses the file instead of dropping the controls. A loader refuses
`controls` in a file older than version 3, more than 120 controls on a step, more than 25 options
on a control, a control string over the configured string bound and more than 1 MiB of control
text on a step (the sum of the string bytes, plus one per option).

## Derived state

Not stored, rebuilt by replaying the steps in order on load: reference counts, structural ids,
region occurrence lists, the template inventories and observation counts (frame log), the effect
table, step flags, and the two lexical indexes. Because the replay uses the same code as ingest, a
reloaded store saves to the same bytes.

## Load validation

A loader checks that every referenced slot is live, node children form no cycle, no tree is deeper
than the configured depth, frames are sorted and distinct, step indices increase within an episode,
episode ids are unique, every region has an occurrence and every stored entry is referenced. Any
violation fails the load.

## Atomicity

Each file is written to a temporary file in the same directory, synced, renamed over the old file
and the directory synced (`durable_file::atomic_write`). A crash leaves the previous snapshot or the
new one; in a store the journal and the checkpoint sequence above make the pair recoverable.

## Journal (`experience.wexj`) and store layout

In a store directory: `experience.wexp` (snapshot), `experience.wexj` (journal), and during a
checkpoint `experience.wexp.next`. In an encrypted store the snapshot file is the whole snapshot
sealed with the store's key and each journal record body is sealed; there is no plaintext form.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | magic `WEXJ` |
| 4 | 1 | version, currently 1 |
| 5 | 3 | zero |
| 8 | n | records |

Record: `u32` body length, `u32` CRC32 (IEEE) of the body, body. The body is CBOR of
`{seq, op}` (sealed when the store is encrypted); `op` is `Ingest` (an observation, with optional
goal and outcome), `IngestForm` (an observation with the form controls parsed from its
accessibility-tree text, as in the snapshot), `Describe` (episode, goal, outcome) or `Purge`
(episode). An observation is journaled without its accessibility-tree text. Replay checks the
controls of an `IngestForm` record against the same bounds as the snapshot loader. A reader that
predates `IngestForm` refuses such a record rather than dropping it; the journal version stays 1,
since a store without controls writes the same records as before. Sequence numbers
start after the snapshot's and increase by one.

Write: apply the change in memory (validation first), append the record, `fdatasync`, return. A
failed append is cut back and the in-memory state is reloaded from the files.

Checkpoint: write `experience.wexp.next` atomically; overwrite the old snapshot with zeros and
unlink it; rename `.next` over it and sync the directory; overwrite the journal with zeros,
unlink it and create an empty one. `Purge` always checkpoints.

Open, in order: a `.next` file is whole (it was written atomically), so it replaces the snapshot;
the snapshot is read (a damaged one is an error); journal records at or below its `seq` are
skipped and the others replayed in order; a missing or zeroed journal means a checkpoint stopped
after publishing its snapshot; a record that is cut short at the end of the file, or whose
checksum fails there, is dropped and the file truncated, because it was never acknowledged; a
failed checksum or a gap in the sequence anywhere else is an error. If the journal held a
`Purge` or records the snapshot already covers, open finishes with a scrubbing checkpoint, so no
forgotten text outlives the call that forgot it.

Tested by process death at every durability point of a workload of writes, checkpoints and
purges (`tests/experience_crash.rs`): the recovered state equals the state after the acknowledged
operations, or after those plus the one in flight.
