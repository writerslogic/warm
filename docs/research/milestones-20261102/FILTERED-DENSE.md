# Filtered-query maintenance regression investigation

Exposed DEEP calibration reports 1%-filtered medians of 10.5 ms initially,
0.59 ms after churn, 10.7 ms after maintenance, and 0.56 ms after reopen, with all
quality checks passing. Server `documentIndexTrained` is respectively true, false,
true, false. COCO shows the same direction. These shared-host development timings
are not qualified performance. Receipts: `calibration-warm-{coco,deep}-selection.json`
and `filtered-dense-calibration-states.json`.

Filtered server vector queries use generic document search. A trained dense index
selects the bounding-box tree; otherwise the engine captures eligible documents
and scores their chunks. `DenseIndex::search_where` checks metadata at leaf rows,
after evaluating bounds. Pure vector queries have no lexical terms, so filtered
capture does not build the eligible-owner set through the lexical slot-table
branch. More than 128 changed documents invalidates the tree; calibration churn
changes 200. Maintenance rebuilds it; reopen has no dense tree. These facts explain
route selection, but do not yet prove the cause of the measured latency difference.

The frozen counterfactual holds store, data, query, scoring, filter and payload
constant while toggling only `dense_index` inside an isolated test. It alternates
the existing tree and existing exact postings scan over three repetitions, using
100K exposed COCO/DEEP vectors, first 32 calibration queries, four filters, and
baseline/retrained-after-churn states. Every result field must match. A separate
production-tree call counts metadata checks and exact vector dots. No replicated
tree algorithm or product change is introduced. Held-out `test.f32` was not opened.

Protocol/code: `benchmarks/research/filtered_dense/`. Run:
`/Volumes/A/.hms-bench-cache/warm-filtered-dense-diagnostic-20261103-v1`.
The 444-file source freeze SHA256 is
`e3dcc27e278ad06dd5f7887155e62f1cdc39ed728924eeef1560e9e84fcb1dfe`.
Only a test include was added to isolated source; production search is unchanged.
Runner `mac-dense-diagnostic.py`, launcher PID 27037, waits for the macOS C-family
terminal receipt, which follows the other queued local checks. Receipts:
`filtered-dense-freeze.json`, `filtered-dense-launcher.pid`.
V1 stopped before compilation because its environment omitted the configured
`CARGO_HOME`/`RUSTUP_HOME`, selecting a different installed Rust toolchain. V2
preserved those settings but exposed four missing existing golden fixtures at
library-test compilation. Both failures are retained. V3 restores only those
fixtures; production and diagnostic logic are unchanged. Its 448-file source
freeze is `d709d7175c0fa5e16004d7ee14926a38457359d00c605b2e9bc9f2fa9d824086`.
Run suffix is now `v3`, launcher PID 29635; `filtered-dense-v3-freeze.json` and
`filtered-dense-v3-launcher.pid` identify it. Compilation passed and the one
explicit diagnostic passed all 1,536 paired comparisons in 16 cells at
2026-11-03T12:55:45Z. Every returned field matches bit-for-bit.

| Corpus / filter | Tree median / scan median, baseline | Ratio after churn and retraining |
|---|---:|---:|
| COCO / 10% | 27.87 / 5.55 ms = 5.02x | 4.98x |
| COCO / 1% | 17.68 / 1.05 ms = 16.91x | 16.41x |
| DEEP / 10% | 16.95 / 3.54 ms = 4.79x | 4.78x |
| DEEP / 1% | 9.93 / 0.79 ms = 12.53x | 12.47x |

The direct tree instrumentation visits every COCO row on every query and nearly
every DEEP row. It computes vectors only for eligible rows, but pays tree bounds
and metadata checks across the wider store. Unfiltered exact tree search is also
1.38–1.86x slower than the exact scan here. Empty-filter serving queries already
take the small-candidate fallback and have similar microsecond costs; their
separate forced-tree instrumentation must not be confused with that serving route.
These are paired local development measurements, not competitive qualification.
Receipts: `filtered-dense-v3-{finished,summary}.json`; all raw hashes match the freeze.

This counterfactual establishes a maintenance-triggered route regression on the
exposed workloads. A follow-up must preserve exact scoring, snapshot ownership,
delta exclusion and low-dimensional pruning benefits; disabling all tree search
would not establish a generally better policy.

This is engineering diagnosis, not an invention. Any fix needs semantic regression
checks and new source/build qualification receipts. Existing calibration and
Intel-soak binaries remain frozen.


## Bounded traversal prototype

`BOUNDED-PLAN.json` freezes a work-limited exact traversal: retain the current
eligible document owners during capture, spend at most
`floor(eligible_chunks * (dimensions + 1) / 16)` counted work units in the tree,
and on exhaustion discard its partial heap and scan that same captured snapshot.
It charges dimensions for each bound/dot and one for each metadata predicate;
this is not a wall-time bound. Hybrid, lexical and unfiltered routes are unchanged.

The isolated prototype passes 152 document tests. Three ignored cases are timing
probes: the prior diagnostic, this explicit new probe and the existing lexical
microbenchmark. New tests cover budget boundaries, exact ties, useful pruning,
replacement, deletion, slot reuse, both fusion modes and a snapshot surviving its
store. Product files have not yet changed.

The frozen development experiment compares original capture, bounded capture and
exact scan over the original 16 cells plus four separated low-dimensional cells.
All result fields must match. Filtered corpus medians must stay within 15% of the
scan, 1%-filtered cases must improve at least 8x over the original tree, and the
four favorable analytic cases must retain useful pruning within 15% of original
latency. The controls include full capture costs. No held-out vectors are opened.

Run: `/Volumes/A/.hms-bench-cache/warm-filtered-dense-bounded-20261103-v1`;
launcher PID 34290; source freeze:
`ee833f7b5f6505efd335917c1f8dc655439c5509a7417102df96ad3e83a3e5b1`.
Runner `mac-dense-bounded.py`; patch and freeze receipts are
`filtered-dense-bounded-v1.patch` and `filtered-dense-bounded-v1-freeze.json`.
The experiment completed at 2026-11-03T13:48:36Z and passed every declared gate.
All 3,840 paired result comparisons across 1,920 query rows / 20 cells are exact.
Across baseline and maintained states, 1%-filtered medians improve 19.97–31.66x
against original capture plus tree traversal; 10%-filtered medians improve
4.53–5.71x. Bounded/scan medians range 0.548–1.129. All 384 favorable analytic
queries finish in the tree with useful pruning; their full-query medians grow
6.1–6.9%, within the fixed 15% limit. Unfiltered routes retain their original
behavior, including the measured high-dimensional tree cost. These remain exposed
development measurements, not competitor or held-out performance results.
Receipts: `filtered-dense-bounded-v1-{finished,summary}.json`.

The same production logic is now integrated in `dense.rs` and `search.rs`, with
five focused regression tests in the existing document test modules. A new test
initially supplied only 32 eligible high-score rows for a 100-candidate request,
so exhaustive fallback was correct; its fixture now supplies 128, testing actual
pruning while replacements, deletion and an inserted top-ranked owner coexist.
The initial failure log is retained. The final gate passes 154 document tests
(one explicitly ignored lexical microbenchmark), clippy with warnings denied,
and formatting. No policy or timing threshold was changed after results.

Product source freeze:
`2640b35260db1a316ab5a212d52ce45cee498f8219ae64a9814cdc2eee88f2bb`.
Raw root: `/Volumes/A/.hms-bench-cache/warm-filtered-dense-product-20261103-v1`.
Patch, freeze and transfer receipts: `filtered-dense-product-v1.*` and
`filtered-dense-product-v1-{freeze,transfer}.json`. Linux product v1 passed the
154 document tests, then its server build failed because the sparse archive
omitted existing documentation fixtures. V2 adds all three identified literal
include fixtures without changing production code. It passes all 100 server
tests and builds the release server; the one ignored document test remains the
explicit lexical microbenchmark. Receipt: `linux-dense-product-v2-finished.json`.
V2 source freeze:
`925947bdabe7721d2170bf7c2ba53c88e8b155335eee63834abdd991236d8e87`.
Server SHA256:
`b61f4e70bcddb3673ef551ea921903124d364b6fb34dc48bb01af8642bd58709`.

WARM-only recalibration v5 launched in the arm64 guest at 14:42:18Z, PID 59054,
using unchanged driver v7, the exposed COCO/DEEP cohorts and this new binary.
V3 stopped before queries because the launcher omitted a comparator receipt
environment variable; v4 deployed the prior runner through a stale local path.
Both failures remain preserved. The new launcher explicitly names every path;
competitor selections and held-out inputs are unchanged. Raw guest output:
`evidence/warm-recalibration-v5`; receipt: `warm-recalibration-v5-launched.json`.
Recalibration completed at 14:48:57Z and its response audit at 14:49:26Z.
All 32 selected state/filter cells / 32,000 query responses pass, with independently
recomputed oracle scores identical and maximum returned-score error 2.67e-7.
Unfiltered selections remain COCO graph ef512 and DEEP graph ef256; every filter
uses exact search. The geometric means of exposed state medians change as follows:

| Corpus | Filter | Previous ms | Updated ms | Previous / updated |
|---|---|---:|---:|---:|
| COCO | 10% | 11.804 | 6.784 | 1.740 |
| COCO | 1% | 3.664 | 0.812 | 4.512 |
| DEEP | 10% | 8.135 | 4.431 | 1.836 |
| DEEP | 1% | 2.471 | 0.509 | 4.858 |

These include states where the old route already used a scan, so they are distinct
from the controlled trained-tree comparison above. Unfiltered/empty routes have
no claimed improvement. The strongest calibrated competitor remains faster on
filtered workloads; this does not establish milestone 3. Receipts:
`warm-recalibration-v5-{finished,comparison}.json`,
`warm-recalibration-audit-v5-finished.json`, and per-corpus v5 selections/audits.
No held-out vectors were opened.


## Remaining filtered-query cost

The completed isolated diagnostic separates preparation, owner capture, ranking, text
materialization and capture release on the current bounded route. It also compares
canonical per-row scoring with eight independent dot products interleaved across
rows, preserving every row's coordinate accumulation order and the canonical
empty-sum identity. This is conventional loop scheduling, not an invention claim.
No production scoring path has changed.

Plan: `benchmarks/research/filtered_dense/PHASE-PLAN.json`; source:
`phase_benchmark.rs`; runner: `mac-dense-phase.py`. The declared cohort is eight
cells / 768 query rows: the same two exposed corpora, first 32 calibration queries,
baseline/maintained-churn states, 10%/1% filters, three repetitions. A kernel lead
requires exact score bits and at least 2x median kernel speedup in every cell;
full-query timing and regressions would still be required before promotion.
Raw root: `/Volumes/A/.hms-bench-cache/warm-filtered-dense-phase-20261103-v1`.
Initial preflight compilation rejected hex formatting of the current sha2 output
type; the probe now uses the project's existing byte-wise hex conversion. That
failure is retained in `preflight-v1.log`. The corrected boundary test passes.

All eight cells / 768 queries complete, with 4,221,888 score-bit comparisons and
identical full results against exact scan. The predeclared 2x kernel gate fails
in every cell: interleaving speeds up COCO by 1.12–1.24x but slows DEEP to
0.76–0.83x of scalar throughput. No kernel change is promoted.

Owner capture consumes 27–32% of COCO's query time and 57–62% of DEEP's; ranking
accounts for 68–72% and 37–42%, respectively. Those measurements identify separate
costs to investigate rather than an established end-to-end optimization. This
core fixture uses a repeated 1,024-byte text string, unlike the serving comparison's
padded random text. It excludes HTTP and durability costs and cannot qualify a
serving performance claim. Source, input and output hashes were verified; receipts
are `filtered-dense-phase-v1-{freeze,finished,summary}.json`.

## Candidate: materialize only ranked semantic chunks

The next isolated candidate captures matching document owners once for a pure
semantic filtered query, scores their embeddings in canonical order, and creates
captured chunks only for the top candidate list before invoking the unchanged
fusion logic. It also avoids posting/slot-table capture for untrained pure
semantic scans. The trained tree keeps its bounded attempt and uses this routine
only when that attempt exhausts its budget. Hybrid and unfiltered paths retain
their existing behavior. No result cache or approximate arithmetic is introduced.

This is conventional implementation work, not an original research claim. It is
not yet applied to production. Plan and candidate generator:
`benchmarks/research/filtered_dense/{SEMANTIC-SCAN-PLAN.json,semantic_scan_patch.py}`;
boundary/cohort checks: `semantic_scan_probe.rs`. The declared screen has 32 cells
and 3,072 paired comparisons, including trained and untrained states before/after
churn. Every filtered cell must improve its median by at least 10%, with no more
than 15% regression in unchanged controls, and all complete results must match.
The earlier repeated-text fixture limitation still applies.

Prepared root: `/Volumes/A/.hms-bench-cache/warm-filtered-semantic-scan-20261103-v1`.
The initial preflight exposed relocated module paths in the preserved baseline
copy; only those paths were corrected. The failed compiler log is retained.
Native document and boundary checks pass: 155 tests, with two intentional ignored
benchmarks. The source is frozen at
`076e786177b08f74200ad65ac93965744f320c822f66bfa29fb1db536d744caa`;
its timing run waits for the iOS simulator job to end.
The screen completed at 15:59:28Z. All 3,072 paired results are identical and all
unchanged controls pass, but the overall timing gate fails: all eight trained
nonempty filtered cells improve only 1.008–1.047x, below 1.10x. The eight untrained
filtered cells improve 1.22–1.74x. This candidate is not promoted unchanged.
All 456 source files and output hashes were verified. Receipts:
`semantic-scan-v1-{freeze,finished,summary}.json`.

[MONOTONE-BOUND.md](MONOTONE-BOUND.md) derives and tests the next candidate: a
same-order endpoint dot already bounds the engine's computed floating-point
score, permitting removal of two outward-rounding operations per coordinate.
Its separate frozen screen retains both earlier controls and low-dimensional
pruning requirements; no performance benefit is assumed from the proof alone.

## Candidate: use captured owner membership during traversal

The computed-bound screen still misses its gate in all four trained 10% cells.
Those traversals repeat metadata predicates after capture has already selected
the exact matching current owners. The next candidate gives pure semantic
filtered tree queries a hash set of those owner addresses, using the existing
`FastSet`. Other queries retain the existing sorted-owner representation.

The captured scan pins every current owner, and the captured tree pins its old
owners; addresses cannot be reused while that query runs. Membership therefore
also excludes replaced/deleted rows. The new set lives only for the query and is
refused before iteration when its input exceeds the existing 524,288-owner cap.
Hash construction is included in timing, and owner counts/capacities are logged.

Plan: `benchmarks/research/filtered_dense/OWNER-PLAN.json`; isolated source:
`/Volumes/A/.hms-bench-cache/warm-owner-membership-20261103-v1`. Three controls are
the preserved product, the preceding combined candidate, and owner membership.
The same 36-cell gate is retained, including low-dimensional pruning. All 157
native checks pass, including cap-1/cap/cap+1 allocation and membership checks.
The frozen screen completed at 2026-11-03T16:31:35Z: 3,456 query rows and 6,912
paired full-result comparisons match exactly. The development speed gate fails
the DEEP trained 10% baseline/churn cells (1.097×/1.099× against the preserved
product, below the required 1.10×). Other cells pass; no production promotion.
Do not round these failures into passes or rerun merely to cross the threshold.

The largest observed owner set contains 16,384 entries with capacity 28,672;
allocation is included in timing, not a whole-process memory measurement.
Freeze SHA256: `64df4d0a793916b6fbb59680797e675941c952248b5cebf0ab6178ae37ead414`.
Raw-query SHA256: `13633c05e39cfeebd6174163bb8fb726067bd711bad725d065f16ec36ef8c8b0`.
All 464 frozen files and recorded artifacts/outputs reverified. Receipts:
`owner-membership-v1-{freeze,finished,summary}.json`.
