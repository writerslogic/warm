<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Experience snapshot (`experience.wexp`)

Written by `Experience::save` (feature `experience`). One file in a directory the caller
chooses. It is not part of a document store's directory or checkpoint.

## Header

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | magic `WEXP` |
| 4 | 1 | version, currently 1 |
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
| `episodes` | id, goal, outcome, and steps: step index, root node, template, frame, optional action-label symbol, and `[region, count]` pairs |

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

The file is written to a temporary file in the same directory, synced, renamed over the old file
and the directory synced (`durable_file::atomic_write`). A crash leaves the previous snapshot or the
new one.
