<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Historical retrieval tail follow-up

The canonical roadmap retains a +11.5% updated hot-query empirical p99 observation
from three rounds of 32 queries against `a736af02`. The October 31 allocation
comparison used `f28095e0`, so it did not answer that historical comparison.
The predeclared [plan](plan.json) compares both pinned revisions with a frozen
integrated candidate using the same deterministic corpus, exact result oracle and
compiler. The initial current-source placeholder was merged main `75ffaa57`;
the final candidate is selected before its build and before any timing measurement.

The existing probe directly calls the private capture/execution path and separately
checks the public engine lifecycle. Current execution returns Result where the
historical API returned Vec; the harness must normalize only that interface.
The current source has fallible reads and many other fixes, so any observed change
is an aggregate revision comparison, not causal attribution to the allocation fix.
The resumed parallel work uses private Cargo target directories. Historical and
prior source archives can compile while the candidate's reliability checks finish.
Measurements require all unrelated builds/tests to finish and execute one probe
process at a time.

The premeasurement investigation identified fallible result collection as one
possible allocation mechanism. The measured gate below did not require a follow-up
optimization; aggregate revision differences are not attributed to this mechanism.


The user resumed the paused work with explicit parallel-completion authorization.
The harness now records each build's complete plan and permits the two fixed
baselines to precede the final current-SHA choice. That is their only allowed plan
difference; current builds require the exact final plan. Four integrity regressions
verify that workload, acceptance, budget and baseline-revision changes invalidate
builds. Probe, harness, compiler/settings, archived source and executable hashes
remain bound before execution. No timing retries select favorable observations.

The separate [correctness gate](../retrieval-correctness-20261101/validation.json)
verifies finite BM25 scores at supported numeric extremes and complete exact vector
ranking after mutation. Its stale-vector read-lock tradeoff is outside this filtered
hybrid workload; this comparison cannot establish that path's latency or contention.


## Completed comparison

The frozen current source is `f39433e9b3a705827a5fed04a40d6e0442bb98bc`.
All 15 processes completed within the 600-second budget (recorded execution:
61.41 seconds). All **7,680** per-query exhaustive oracle checks passed, and complete
ordered result payloads match across all revisions and rounds. No timing run was
retried and no optimization followed the measurement. Source, executable, harness,
plan and report hashes are retained with the raw per-query samples.

Five process rounds each contain 128 samples per query group. The following values
are medians of the five round-level nearest-rank quantiles, in milliseconds:

| Group | Historical p50 / p99 | Prior p50 / p99 | Current p50 / p99 |
|---|---:|---:|---:|
| initial-rare | 2.117 / 2.695 | 0.905 / 1.275 | 0.910 / 1.281 |
| initial-hot | 1.583 / 1.962 | 1.627 / 2.064 | 1.601 / 1.934 |
| updated-rare | 2.076 / 2.583 | 0.911 / 1.298 | 0.903 / 1.246 |
| updated-hot | 1.569 / 1.961 | 1.636 / 2.047 | 1.608 / 1.994 |

Updated-hot p50/p99 changes are **+2.50%/+1.70%** against the actual historical
baseline, and **−1.68%/−2.59%** against the prior baseline. Neither initial-hot nor
updated-hot trips the frozen sustained-regression rule for p50 or empirical p99
against either baseline. The earlier +11.5% observation is therefore not reproduced
above this study's predeclared threshold. This does not rewrite the historical
samples or establish a population-tail bound: paired current/historical updated-hot
p99 changes range from −1.13% to +6.86%, and current/prior includes a +15.55% round.

Allocation and memory costs remain distinct from timing:

- Updated-hot current allocation count is 156, compared with prior 178 and historical
  151. Every one of 640 current/prior query pairs removes exactly 22 allocations;
  current is still five allocations above the historical baseline.
- Rare-query peak requested bytes are 127,509 initially and 127,445 after updates,
  versus historical 43,645 and 43,581: **83,864 additional bytes** (about 81.9 KiB,
  +192%). The prior baseline already has the same increased peak as current.
  These rare-query timings are about 52–57% lower than historical on this workload,
  with the increased requested-byte peak shown above.
- Hot-query peak requested bytes increase by 64 bytes over historical and match
  prior. These are instrumented incremental requested-byte peaks, excluding
  allocator-internal storage; they are not process RSS.

The host was an Apple M4 with 10 logical cores and 32 GiB RAM; each probe used one
Rayon worker. Parent/worker build and browser tests were stopped during measurement;
ordinary desktop processes remained observable in the retained host snapshot.
The corrected System realloc instrumentation and complete-result retention differ
from the old probes, so compare the three new variants to each other rather than
comparing their absolute values directly with the old measurements. This is a small,
filtered, 16-dimensional synthetic workload, not competitive or production evidence.

## Reproduction

`uv run --no-project --offline python docs/research/technical-audit-20261029/retrieval-tail-20261101/summarize.py`
regenerates `summary.json` from retained raw reports after checking their hashes.
[validation.json](validation.json) records the acceptance decisions and limits;
[execution.json](execution.json) records all process commands and orders.

For a fresh measurement, use a separate checkout with the three recorded commits
available and a fresh target directory. Copy only `plan.json`, `probe.py`,
`retrieval_probe.rs`, `summarize.py` and `test_probe.py` into a new sibling evidence
directory under `docs/research/technical-audit-20261029/`, preserving the recorded
bundle. Run `test_probe.py`, then invoke `probe.py historical`, `probe.py prior` and
`probe.py current` there with `uv run --no-project --offline python`. After other
builds/tests stop, invoke `probe.py run` once and then `summarize.py`. The driver
refuses existing build/run artifacts, verifies source and executable hashes, and
never silently retries a measurement. Preserve any failed run separately.
