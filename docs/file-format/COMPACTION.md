<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Background compaction

`WarmCore::start_compaction` in Rust and `WarmMemorySystem.startCompaction` in Node run the
same compaction as `compact()`, on a dedicated worker thread. They return a job with
`progress()`, `cancel()` and `wait()`. The on-disk format is unchanged: the job writes the same
`gen-<pid>-<nanos>/` generation and the same `CURRENT` pointer that [JOURNAL.md](JOURNAL.md)
describes.

## Lock protocol

The job holds an `Arc` of the engine. The engine's exclusive store lock (`.hms.lock`) is
therefore held until the worker exits. Dropping the Rust handle cancels the job and joins it.
`wait()` returns only after the worker has released the engine. Once both have happened,
closing the engine and reopening the path succeeds.

Locks are taken in this order:

1. The compaction mutex. A job uses `try_lock`, so a job that finds it held finishes as
   `busy` without queuing. `compact()` blocks behind a running job.
2. The writer lease (`mutation_gate.maintenance()`).
3. The shard, document and arena locks.

No lock is held while the CPU throttle sleeps or while a budget is checked.

## Stages

Each stage named in the table is a crash point: `WARM_TEST_COMPACTION_PHASE=<stage>` exits the
process at it, and the recovery tests interrupt each one.

| Stage | Work | Writer lease | Foreground writes | Foreground reads | Cancel |
|---|---|---|---|---|---|
| snapshot | Removes stale unreferenced `gen-*`, clones live state | held | wait | run | honoured |
| `staging` | Writes frames into the new generation | free | run (old tail) | run | honoured per frame |
| `caught-up` | Two unlocked tail replays | free | run | run | honoured |
| `staged` | Final tail replay; generation synced | held | wait | run | ignored from here |
| `published` | `CURRENT` replaced | held | wait | wait for the swap | ignored |
| `mapped` | Arena swapped to the new generation, old removed | held | wait | run | ignored |
| `released` | Lease released; indices not yet persisted | free | run | run | ignored |
| `finished` | Indices and audit record persisted | free | run | run | ignored |

The point of no return is the final writer lease. Before it, a cancellation, budget or failure
outcome removes the staging generation. From it on, the job always publishes. A cancel that
arrives late is recorded as `cancelRequested` on a `completed` report. A job never reports
`cancelled` for a generation it published. A crash before `published` leaves the old
generation live, and the next compaction removes the stale one. A crash after it opens the new
generation with every acknowledged write.

## Budgets

- **CPU.** One worker thread. With `maxCpuFraction = f`, the worker sleeps `t·(1/f − 1)` after
  every slice `t` of staging or replay work. Wall time is at least thread CPU time, so this
  bounds the job's CPU share from above. Snapshot and publication are never throttled, because
  they hold the lease.
- **Memory.** `maxMemoryBytes` bounds the estimated size of the cloned snapshot: inline record
  size plus vector indices and ids. It is checked as each record is cloned, so the job stops
  at the first record past the limit, before anything is staged. Documents are shared
  (`Arc`), not copied, so each counts at pointer size.
- **Temporary disk.** `maxTempDiskBytes` bounds the staging generation's allocated file
  length. Before every frame, the job predicts the post-write allocation with the arena's own
  growth rule and refuses the write if that exceeds the limit. Sealed segments count at full
  segment size.

## Outcomes

| Status | Meaning |
|---|---|
| `completed` | Published |
| `cancelled` | Cancelled before the point of no return |
| `budget_exceeded` | A budget stopped the job; the resource is named |
| `busy` | Another compaction held the mutex |
| `no_progress` | The new generation would not be smaller (`requireReclaim`, on by default) |
| `failed` | An error, or a panic in the worker |

`failed` with `published = true` means the swap committed and a later step failed, the same
case as `CommittedMutationError`. Every outcome except `completed` leaves exactly one `gen-*`,
the live one.

## Accounting

- `bytesWritten` is the new generation's journal bytes (staged frames plus replayed tail) plus
  the checkpoint.
- `diskBytesBefore` and `diskBytesAfter` are the summed file lengths of the old and new
  generation directories.
- `reclaimedBytes = max(0, before − after)`.
- `writeAmplification = bytesWritten / reclaimedBytes`, the bytes written per byte freed. It is
  null when nothing was freed.
- `staleBytesRemoved` counts leftovers of earlier interrupted runs.
