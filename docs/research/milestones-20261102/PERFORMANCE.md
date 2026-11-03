# Matched performance: preselection and implementation constraints

The first held-out qualification queue started on the Mac at 18:32Z on 2026-11-03;
no comparison result is yet qualified. All six exposed-query calibrations completed
in the local arm64 guest; their timings are not qualified comparisons. The noteworthy
advantage selected before those measurements is **at least 2× completed-query
throughput**, preserving the quality gate, on both workloads and both physical
hardware configurations below. Do not switch to memory or context-token savings
after inspecting throughput. Original research hypotheses have their own gates;
this comparison supplies a control for a future advance.

Exposed measurements now identify a filtered-query slowdown when the generic
dense tree is trained. [FILTERED-DENSE.md](FILTERED-DENSE.md) records the code-path
evidence, the completed same-store counterfactual and a passing bounded-tree
development screen. The fix is integrated and a fresh arm64 server build is in
progress in the original preparation; WARM's recalibration v5 is now complete. This is an engineering
diagnosis, not a qualified comparison; no running qualification binary was changed.

The later [certified arithmetic screen](CERTIFIED-SCAN.md) preserves all 6,912
paired result comparisons but fails six of its fixed performance cells; it is
not promoted. The serving comparison therefore retains the calibrated bounded-
traversal binary `b61f4e70bcddb3673ef551ea921903124d364b6fb34dc48bb01af8642bd58709`.

The 33-file execution/audit bundle is now frozen at
`/Volumes/A/.hms-bench-cache/warm-serving-qualification-20261103-v1`;
archive SHA256 `edb4ce972277910a242ee0d3a5903c79b3e561567676ebbf3df7cf0dc3b8b867`,
freeze SHA256 `25db4682c68d88c949649c123c9b8c363375277174cb1456995a639be154e004`.
It includes the already-tested engine-specific sealer and calibration-auditor
corrections. `linux-serving-seal.py` requires synthetic process-death recovery
on that exact WARM binary and verifies all six selections and installed
OpenSearch components before issuing the configuration seal. It opens no
held-out vectors. `linux-serving-qualification.py` retains the frozen 18-job
order and continues after completed quality failures without changing policy;
incomplete execution or changed sources stop the sequence.

Physical-host observations will retain cumulative per-process CPU, RSS, paging,
memory pressure, swap use, thermal and power status every five seconds using
`mac-serving-isolation.py`. Raw logs go to the internal disk to avoid monitoring
writes on the benchmark's external USB disk. Observation alone never sets an
isolation pass; gaps, other workloads, paging and throttling require review.
All agent-owned local benchmarks/builds must stop before held-out measurements.

Storage admission found only 14 GB free in the 48 GiB guest, below the existing
20 GiB per-store ceiling plus 1 GiB reserve. The idle guest was stopped and a
full backup copied and hash-verified before growing its disk to 192 GiB;
the four-CPU/eight-GiB limits and disabled guest swap stay fixed. The underlying
volume is an external USB hard disk with HFS+, which must be disclosed. An early
restart was mistakenly issued during the first backup; that copy was interrupted,
marked invalid and retained, the guest was stopped, and a new sequential backup/
verify/resize/start operation completed. No disk resize or held-out search
occurred in that failed preparation. Evidence lives under
`/Volumes/A/.hms-bench-cache/warm-serving-host-20261103-v1`.
The successful backup completed at 18:01Z, with both full 48 GiB hashes equal to
`513460f981f325de1b896f521222e56cf84ad5adabded79689e6020461a66ed8`.
Lima restarted successfully at 18:14Z. Filesystem admission passed with
164,015,960,064 free bytes, four CPUs, 8,308,285,440 memory bytes and no guest swap.
`mac-serving-preflight-launch.py` transferred both immutable bundles, then stopped
on the DSPy v1 test-fixture error described in PLATFORMS.md; its failed receipt is
retained. DSPy v2 passed all 20 cases before the separately launched sealer passed
all 16 synthetic quality/recovery cells and sealed all six configurations at
18:21:36Z. Configuration seal SHA256:
`45ce0a0d974a861ab0e45a47c9bb4940635867e8ed0dd1d77ca77ed4e5382864`.
Receipts: `serving-v1-sealed.json`, `serving-v1-configuration-seal.json` and
`serving-v1-warm-recovery-finished.json`. No held-out search occurred during these
checks. The supplemental guest observer is frozen
in `serving-observer-v1-freeze.json`; it streams process/load/paging/pressure data
to the host's internal disk and supplies no automatic isolation pass.

The sealed held-out files were transferred and hash/size checked at 18:22Z;
`serving-v1-inputs-prepared.json` binds that transfer to the configuration seal.
The physical-host observer began at 18:23Z. Its prelude found sustained
background media analysis above one CPU core, so measurement was delayed until
that burst ended. The native functional gate completed at 18:25Z. Final prelude
inspection found an idle guest, 75% free host memory, no swap-outs/page-outs and
no recorded thermal warning. The foreground controller and its Terminal ancestor
used about 22% of one host CPU core together; their overhead remains recorded.
Four swap-in pages were observed without memory pressure. These observations
permit starting measurements; they do not establish whole-run isolation.

The frozen 18-job queue finished at 2026-11-03T19:06:19Z, and both observers
terminated. All 576 cells / 576,000 requests completed without request errors;
84 cells failed quality. Every COCO engine/repetition fails the fixed worst-query
floor. DEEP WARM and pgvector pass all cells; OpenSearch fails some baseline
cells but passes the maintained, unfiltered, eight-client target. Its eligible
target throughput is the strongest baseline: WARM ratios are 1.895356×,
1.919053× and 2.144044×. The preselected 2× gate fails in two repetitions.
Milestones 3 and 6 are not met. No serving policy was retuned after opening queries.

All 18 frozen raw audits pass membership, payload, score, oracle and recorded
resource arithmetic checks. This does not validate process coverage. The original
sampler counts the PostgreSQL postmaster's process group, while its backends call
`setsid()` and escape that group. For example, job 01 baseline/all/eight clients
records only `server:2297:121359`, around 29 MB, while the independent guest
observer sees PostgreSQL workers using hundreds of MB. Thus whole-process memory
and the declared resource cap are unqualified. PostgreSQL 18.6's
[InitPostmasterChild source](https://github.com/postgres/postgres/blob/REL_18_6/src/backend/utils/init/miscinit.c)
confirms this process-session behavior. Working source now follows observed
parent/child ancestry across sessions; a detached 32 MB child regression and
measurement tests pass all 15 cases on both macOS and Linux. Sampling remains
a lower bound, including when descendants reparent before observation. The
original frozen evidence and its failures remain unchanged; any future
qualification needs a new freeze and resource calibration.

Whole-run observations cover 407 host and 408 guest samples, with no failed
commands and maximum spacing 5.012 seconds. Host memory stayed 66–76% free, but
52 page-outs and 217 swap-ins occurred; background media/Spotlight processes
also produced CPU bursts. There were no swap-outs or recorded thermal warnings.
`isolationPassed` remains false. Raw audit success does not certify isolation.

The verified export contains 7,537 files (490,816,800 uncompressed bytes):
`/Volumes/A/.hms-bench-cache/warm-serving-qualification-20261103-v1/audit-export-v1/serving-evidence-v1.tar.gz`,
SHA256 `564007b5db46c9ca549b00f5bf7ebf84175935bc5464f6636308a4fc720efc30`.
Receipts are `serving-v1-finished.json`, `serving-v1-raw-audits-finished.json`,
`serving-v1-audit-export-finished.json`, and
`serving-v1-observations-summary-v1.json`. The export preserves measurements,
responses, settings and audits; it is not a self-contained source/input reproducer.

The sections below retain the development and protocol history. Earlier queued
or not-yet-opened states are superseded by the terminal results above.

## Cohorts and declared outcomes

Use COCO-I2I and DEEP1B, the same first 100,000 source training rows as the accuracy
qualification. Calibration may use the first 1,000 queries already exposed in
milestone 2. Performance qualification must use source test ordinals 1,000–1,999,
which were sealed before any calibration and have not been searched. Seal their hashes and eligibility
before calibrating a serving configuration. Any nonfinite/zero input is a declared
eligibility exception with its source ordinal; do not replace a difficult query.
Changing the cohort invalidates this preselection.

The two throughput targets are the unfiltered, maintained, 100K-row workload for
each corpus at eight concurrent clients. Require 2× against the fastest eligible
matched comparator in each of three retained repetitions on both machines.
Measure all engines with the same driver, query order, requests completed and
end-to-end response boundary; count every error and timeout. Publish build,
maintenance and mutation costs even though throughput is the selected metric.

Milestone 3 additionally requires each declared latency cell to beat the strongest
eligible comparator by at least 10% at both median and p95 in all three repetitions,
with p99 no more than 5% worse. Include concurrency one and eight; all/10%/1%/empty
filters; baseline, changed-before-maintenance and changed-after-maintenance;
separate-process cold open; and whole-process memory. A good unfiltered throughput
row cannot cancel a failed filter or churn row. Do not pool repetitions or hide
failed quality cells. Reuse the 100 replacements/100 deletions from the accuracy
protocol, and include returned text and metadata in validation and timed decoding.

The quality gate remains mean recall@10 >=99%, worst >=90%, with zero unexplained
exact membership, payload, duplicate, deleted or eligibility errors, and exactly
empty output for the empty filter. This is a new HTTP qualification; the Node
accuracy result cannot substitute for it.

## Comparators and durability

Reuse the existing server lifecycle, binary pinning and process-tree measurement
code in `benchmarks/competitive/warmbench/`. Its current ID-only request/response
and update timing protocol does not itself meet this matched-payload gate.

The required candidates include PostgreSQL/pgvector HNSW and IVFFlat and
OpenSearch/Faiss HNSW, with per-request durable acknowledgement and read visibility.
Calibrate public query/build knobs on the calibration cohort only, charging
payload handling and resource consumption, then freeze the fastest quality-eligible
configuration for each engine before any held-out query. The grid, payload schema,
deadlines, resource ceilings and repetition order are now specified in
[PLAN.json](../../../benchmarks/public/matched_server/PLAN.json). The executable
and imported lifecycle sources are now sealed in driver v6 after adapter validation.
No held-out search has started. Calibration must pass every declared state before
a configuration can be selected.

The existing pins are pgvector 0.8.7 and OpenSearch 3.9.0. Check each executable's
actual version, extension version and digest on both hosts, and use the same
versions. Qdrant 1.19.2 may supply an additional comparison row; its existing
`wait=true` configuration uses periodic WAL flushing and is **not** eligible for
the per-request-flush claim. Do not silently label it matched or force a costly
snapshot per request as a substitute for its normal write contract.

On native macOS the existing PostgreSQL adapter sets `fsync_writethrough` to match
WARM's drive-cache flush. The planned Linux environment avoids mixing those
platform-specific primitives. OpenSearch's translog acknowledgement must remain
`request`; refresh visibility is a separate obligation. Recovery checks must
verify the declared write boundary on each actual artifact before timing.

## Hardware and current implementation differences

The physical configurations are Apple M4 and Intel Core Ultra 9 185H. Linux arm64
in a dedicated Lima guest on the Mac and Linux x86_64 in WSL on the Intel host
allow both durable server comparators to run on both machines. The guest and host
are one hardware configuration; Windows and WSL are also one. Match allocated
CPU/memory limits across engines, disclose virtualization and storage, measure
only one engine at a time, and reject runs contaminated by unrelated load.
The Intel qualification cannot begin while the frozen 24-hour soak is running.

Do not reuse the Node policy label for the HTTP server. Inspection found that
`src/bin/warm-server/vindex.rs` constructs its persisted graph with
`BuildParams::default()` (degree32/build ef128) and ordinary single-hop search,
whereas the document engine used by the Node qualification explicitly builds
degree64/ef256 and uses two-hop search. Server query dispatch can
prefer the persisted index while it covers the store, use the engine path when
it does not, and uses generic exact filtering. Capture the actual graph and route
in each phase before asserting parity. Increasing a public query ef is calibration,
not an original contribution or a change justified by held-out results.

An arm64 OpenSearch archive is provided by its
[official installation guide](https://docs.opensearch.org/latest/install-and-configure/install-opensearch/tar/).
The published SHA512 for `opensearch-3.9.0-linux-arm64.tar.gz` is
`629b6a97104d6f925becd2372d902ea253a6209827435841c2cea9866c83567af776942b488110807370f9f504451bb4512f3bb1afb3c956386914c9474423fc`.
The verified archive was installed in the local arm64 guest, alongside PostgreSQL
18.6 built from its verified source and pgvector 0.8.7 at tag commit
`f37c13f68b57d2c3472b2214fbcff699d6d34876`. All three reported their expected
versions. Installation logs and executable digests are in `linux-comparators/`.

## Input and adapter evidence

The held-out input seal is
`/Volumes/A/.hms-bench-cache/warm-performance-inputs-20261103-v1` with freeze SHA256
`2f787fb0cb48ecf7b4d21925d9709e186a460c5a3a8de25622195e942f02c3b7`.
Both selected 1,000-query cohorts passed finite/nonzero eligibility without
replacement. Training and calibration inputs match the accuracy-run digests.
`performance-input-{freeze,finished}.json` retain the receipts here; two extraction
tests and three protocol/oracle tests pass.

Synthetic adapter validation uses 1,024 generated rows, 12 queries, identical
payloads and an injected process-group death after 100 acknowledged replacements
and 100 deletions. It is development, not a benchmark or power-loss claim.
Attempt v1 passed the eight WARM baseline cells but its recovery inspector used
an unsupported GET route (HTTP 405). Version 2 uses the documented ID-constrained
query and verifies every acknowledged replacement and deletion. The failed
attempt remains under the guest's `evidence/adapter-warm-v1/`; no search-policy
parameter or selected input changed in this correction.

WARM v2, PostgreSQL/HNSW v3, OpenSearch/Faiss v5, OpenSearch/Lucene v5 and
PostgreSQL/IVFFlat v5 each passed all 16 synthetic cells and recovered all 100
acknowledged replacements and 100 deletions after process-group SIGKILL. Full
receipts, logs and failed attempts are in `adapter-evidence.tar.gz`. PostgreSQL
v2 exposed an SQL alias-expression error in the adapter; the outer projection in
v3 corrected it. OpenSearch v3 lacked the native-library path set by its bundled
tar launcher; v4 supplied that same path without changing the executable.

OpenSearch/Faiss v4 then passed recovery and exact queries but failed unmaintained
churn ANN quality at candidate `k=10`: mean 91.67%, worst 80%, on the 12 synthetic
queries. Its returned scores agreed with independent cosine scoring; a read-only
diagnosis found `k=32` and `k=64` each restored all memberships, still returning
10 results. This implicates candidate truncation after churn, not a score-scale
error; the precise internal cause has not been proven. The raw explanations and
membership checks are in `opensearch-scoring-diagnosis{,-v2}.json`.

Before any corpus calibration, plan version 2 added candidate counts 10/32 and
Lucene build controls, and required calibration eligibility across baseline,
unmaintained churn, maintained churn and reopen. These changes strengthen the
controls. The original plan and failure remain in driver-v1 through v4 archives;
the cohort, quality gates, selected advantage and held-out split did not change.
Faiss v5 at candidate count 32 passed the full synthetic protocol. Do not select
candidate count 10 solely from a clean baseline and conceal its failed churn case.

The adapter follows the official
[OpenSearch filtered k-NN request](https://docs.opensearch.org/latest/query-dsl/specialized/k-nn/index/)
and [pgvector iterative-scan contract](https://github.com/pgvector/pgvector/tree/v0.8.7#iterative-index-scans).
PostgreSQL uses `fsync=on`, `synchronous_commit=on` and `full_page_writes=on`, whose
[documented boundary](https://www.postgresql.org/docs/18/runtime-config-wal.html)
waits for the local WAL flush. OpenSearch's actual translog setting and all returned
payloads must be checked on the running artifact. A passing small adapter check
does not establish the performance or durability gates at 100K rows.

## Running calibration

All six corpus/engine calibrations have completed with eligible selections.
The complete synthetic qualification driver and serving-job audit pass on all
five implementation variants. The selected calibration response audit passes
288 configuration/state/filter cells (288,000 query responses) across every
eligible build, with independently recomputed oracle scores identical to the
stored oracle and maximum returned-score error below 5.3e-7. No held-out vector
was opened. Receipts: `qualification-check-v1-finished.json`,
`serving-job-audit-v1-finished.json`, and `calibration-audit-v3-summary.json`.

Calibration audit v1 incorrectly required raw responses for rejected warm-up
cells, although the frozen producer opens the response log only after warm-up.
The corrected auditor records eight COCO and 16 DEEP PostgreSQL rejections
without raw responses explicitly; it still requires every passing selected cell
to have complete verified responses. Its four regression tests pass. Launcher
v2 then falsely detected its own `sh -c` argument as an active calibration;
launcher v3 checks actual argument paths and runs from a script file. All failed
receipts remain. Auditor source is frozen in `calibration-audit-driver-v2`;
successful output directories use suffix `v3`.

The [same-store tree/scan diagnostic](FILTERED-DENSE.md) now establishes a
maintenance-triggered filtered-route slowdown: 1%-filtered tree queries are
12–17x slower than the existing exact scan on these exposed inputs, with identical
results. This must be addressed and recalibrated before held-out qualification;
the current calibrations do not establish a competitive performance pass.

The bounded exact-traversal correction now passes its frozen development gate,
154 document tests on both hosts and 100 Linux server tests. A new pinned Linux
server was built. WARM-only recalibration v5 and its response audit pass on the same exposed
COCO/DEEP cohorts (32 selected cells / 32,000 verified query responses); the unchanged competitor selections remain controls. Attempts
v3/v4 failed in their launchers before queries and are retained. See
[FILTERED-DENSE.md](FILTERED-DENSE.md) for hashes and launch receipts. Held-out
qualification remains unopened. The change improves exposed filtered medians
1.7–4.9x across the four states, but filtered cells remain slower than the strongest
calibrated competitor. These are development selections, not a qualified win.

A sealing preflight identified an overly broad artifact check: every calibration
receipt includes all installed binaries, even those its engine never executes.
The validator now binds WARM calibration to WARM, PostgreSQL calibration to both
PostgreSQL and pgvector, while the existing complete pinned OpenSearch archive
verification remains required by the sealer. Rebuilding WARM therefore does not
invalidate unchanged competitors' measurements. Four regression tests reject
changed/missing serving artifacts and retain unrelated-engine calibration; all
six actual selections pass the new preflight. No configuration seal or held-out
run has been produced. Evidence: `seal-engine-identity-{tests.log,preflight.json}`;
previous frozen driver/auditor versions remain unchanged.

The guest root is `/home/davidcondrey.guest/warm-milestones-20261103`.
The initial durable launcher was `linux-calibration-v1.sh`, PID 25041.
Driver archive SHA256:
`a27c70fe1d29f7a40eeadc26456a0f21bb03cf5da7d95f9049022ad6b34c489c`.
The executable copy is `calibration-qualified-source/warm-server`. The transferred
input directory intentionally contains training and calibration vectors only;
held-out vector files are absent. Five protocol/oracle/selection tests passed.

Each `evidence/calibration-{engine}-{dataset}-v1/` contains the pre-run seal,
oracle, settings, raw query rows, resource samples, rejected cells and selection.
`evidence/calibration-v1-exit-codes.txt` records every controller exit;
`calibration-v1-finished.txt` would mark launcher completion, not passing gates.
That launcher was stopped after WARM's idle management connection expired between
baseline queries and the first churn write (`BrokenPipeError`). The failed run
and any uncertain write remain preserved; it cannot supply a completed selection.
The independent PostgreSQL/COCO controller already running (PID 25101, driver v6)
was left active. Its SQL connection does not use the failed HTTP transport.

Driver v7 opens a fresh management connection before each mutation/maintenance
phase. It does not retry a failed write and does not change query timing, inputs,
grid or quality gates. Archive SHA256:
`fba17b05dc9c207c7acf896c2e7f35cda844754f7006fb008175a35a5639ac50`.
`linux-calibration-v2.sh`, PID 25201, waits for the identified PostgreSQL controller
before running WARM/COCO, OpenSearch/COCO and all three DEEP controllers sequentially.
It deliberately retains PostgreSQL/COCO v1 instead of repeating valid independent
work. Inspect both versions' selection receipts; a stopped controller or missing
receipt is not success. Version 2 logs/exit codes/terminal marker use `calibration-v2`.
Do not edit frozen `matched-driver-v6/`, `matched-driver-v7/` or the executable copy.
The Intel run remains deferred while its frozen 24-hour soak is active.

## Qualification controller and raw evidence

`measure.py` creates one or eight separate client processes, establishes their
connections before a common start, uses the frozen query permutation and contiguous
client slices, and retains every request/error without retries. Query latency ends
after response decoding; completed-query throughput includes client validation and
scheduling. Canonical payload hashes, returned IDs/scores, timestamps and raw-file
hashes are retained. A float64 audit checks every returned cosine, including replaced
vectors and OpenSearch's `(1+cosine)/2` score scale. Its absolute tolerance is 1e-4
and cannot waive a membership discrepancy. Linux samples include parent, workers
and server PSS/CPU; sampled peaks and short-lived-process CPU are lower bounds.
Development is the default and cannot silently qualify a small run.

`seal_qualification.py` requires all six complete exposed-query calibration
receipts, recomputes selection, pins source/binaries and verifies the installed
OpenSearch distribution against its official archive. It never reads held-out
vectors. `qualify.py` admits one job in the frozen 18-job order only after sealing,
then enforces four CPUs, at most eight GiB and no swap. It includes all four states,
filters and concurrencies, 100 acknowledged replacements and deletions, maintenance,
and a fresh-process cold open whose request is prepared before restart. Filesystem
page cache is retained and disclosed. Mutation deadlines are enforced. Storage
usage is checked at phase boundaries; it is not a continuous peak measurement.

`verify_evidence.py` independently recomputes raw cell membership, payload hashes,
cosines, recall, latencies, throughput and sampled resource totals. It checks source
query order, worker completion and every raw-file digest. Recomputed digests alone
cannot hide tampered payload, oracle or timing fields. This is independent code
within this agent's work, not milestone 8's independent evaluator. Calibration's
older payload-hash format and the enclosing source/job/physical-host-isolation
receipts still need separate verification before any performance claim.

`claim_gate.py` requires the complete two-host, two-corpus, three-repetition matrix;
one failed latency cell rejects that gate, and all 12 selected throughput cells
must clear 2x against the fastest eligible comparator. Raw-evidence, resource and
physical-host-isolation verification must all pass. No controller sets these
obligations true by assertion. No held-out qualification has started.

Twenty-five protocol, concurrent-client, claim-gate, archive-integrity, seal and
controller tests pass locally; see `qualification-driver-tests.log`. Cases include
503 responses without retries, bad payload types/scores, tampered evidence with
recomputed hashes, missing/duplicate cells, one failed repetition, memory-cap
boundaries, archive traversal, changed components and actual mutation timeouts.
The initial fixture's five-connection listen backlog was corrected to 32; HTTP
clients connect before readiness. No error allowances were added.

The previous measurement-only development queue (PID 25856) was stopped before
integration began; its frozen source is preserved and the replacement reason is
in `measurement-v1-queue-replaced.json`. The consolidated full-lifecycle source is
`/Volumes/A/.hms-bench-cache/warm-matched-qualification-driver-20261103-v1`, with
29 source/configuration files and archive SHA256
`17b2bebc232ca1a153e909425826e52c0b63930a02f8865262db4c4a306de29c`.
See `qualification-driver-source.json` and `linux-qualification-check-v1.sh`.
Guest launcher PID 26553 waits for calibration v2 to finish, verifies the installed
OpenSearch components, then checks WARM, PostgreSQL HNSW/IVFFlat and OpenSearch
Faiss/Lucene sequentially. Each uses 1,024 synthetic rows and 12 queries through
all 32 lifecycle/filter/concurrency cells, followed by raw-evidence verification.
It stops at the first failure. Logs/receipts use `evidence/qualification-check-*-v1`
and `qualification-check-v1-finished.json`. These checks are queued, not passed;
they contain no held-out vectors and cannot support a benchmark claim. The frozen
calibration drivers and Intel soak remain unchanged.

## Queued calibration evidence audit

[audit_calibration.py](../../../benchmarks/public/audit_calibration.py) checks every
declared cell summary and recomputes each build's selected configurations and the
winning build. For each eligible build, it audits the selected raw configuration
in every filter/state against freshly computed calibration truth, returned IDs,
payload hashes, cosine scores and recorded latency percentiles. Rejected raw
configurations are retained but not independently replayed by this audit.

The original calibration hash included metadata object insertion order. The
auditor identifies the one matching permutation of the five expected metadata
keys on the first nonempty response, then verifies every subsequent digest with
that order. It does not accept arbitrary payload changes, boolean/integer type
substitutions or membership substitutions. Oracle scores may differ by at most
1e-12 due to numerical execution; reference membership remains exact. Three new
tests pass, including all filters, churn, the three score scales, typed payload
tampering and the tolerance boundary. Log: `calibration-audit-tests.log`.

Frozen source is at
`/Volumes/A/.hms-bench-cache/warm-calibration-audit-driver-20261103-v1`, archive SHA256
`41bd076ef847c9849b64cdf3f9b981b08898da311288187197c5adf3b9f90096`.
Guest launcher `linux-calibration-audit-v1.sh`, PID 27018, waits for the synthetic
qualification checks to finish, then audits all six completed calibrations. It
stops at the first failed audit and records a terminal failure. Files use
`evidence/calibration-audit-*`; `calibration-audit-v1-finished.json` is the final
launcher receipt. This is queued work, not a verified calibration result. It
opens only training and exposed calibration vectors; the sealed held-out queries
remain untouched. Physical-host isolation and full resource/source-job validation
remain separate admission requirements before any competitive claim.

## Complete serving-job evidence audit

On 2026-11-03, COCO calibration completed for PostgreSQL v1 and WARM v2 with
eligible selections and no failed builds. PostgreSQL selected HNSW m32/build-ef500:
planner ef64 unfiltered, planner ef256 at 10%, planner ef128 at 1%, and iterative
ef128 for the empty filter. WARM selected the frozen degree32/build-ef128 graph
with query ef512 unfiltered and exact search for all three filters. Copies are
`calibration-pgvector-coco-selection.json` and `calibration-warm-coco-selection.json`.
These are completed selection receipts awaiting the queued independent raw-data
audit, not qualified comparative timings. OpenSearch COCO calibration continues;
DEEP and the all-six selection seal remain pending. No held-out performance query
has been opened.

[audit_serving_job.py](../../../benchmarks/public/audit_serving_job.py) connects the
existing raw-cell verifier to the complete lifecycle. It requires the ordered
32-cell grid, exact agreement between standalone and embedded cell receipts,
all 100 replacement and 100 deletion receipts, declared store/PSS bounds and
resource timelines, and an independently checked first response after restart.
The cold response is checked for membership, payload types, transformed cosine
scores, timing boundaries and its quality verdict. Full exact references are
recomputed. Qualification mode additionally binds the pre-query seal, frozen
implementation, job order, input hashes, oracle preparation, effective resource
caps and recorded post-run source check. It does not open qualification vectors
in synthetic mode.

Four focused tests pass, including missing/duplicated/reordered cells, identity
and mode substitutions, forged mutation receipts, bounded resource values,
cold-response tampering across all three score conventions, and oversized/linked
evidence files. See `serving-job-audit-tests.log`. No serving implementation or
acceptance threshold changed.

Frozen source:
`/Volumes/A/.hms-bench-cache/warm-serving-job-audit-driver-20261103-v1`;
33-file archive SHA256
`748cb56e5a983d0fc90ddee9d0641013dd1daf2789131f35de1752894f7214a6`.
Guest launcher PID 27564 was verified waiting for the existing synthetic
qualification checks. It then audits WARM, both PostgreSQL variants and both
OpenSearch variants; outputs use `evidence/serving-job-audit-*`, with terminal
`serving-job-audit-v1-finished.json`. Integration is queued, not passed.

The auditor deliberately leaves physical-host isolation, actual engine-settings/
durability review, the complete comparison matrix and independent reproduction
as outstanding evidence. It cannot promote its own result to a competitive
claim. Future isolated performance runs must also wait for the native-Python and
live-SDK qualification work recorded in [PLATFORMS.md](PLATFORMS.md) to stop.
