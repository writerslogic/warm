# Milestone qualification

The acceptance criteria are the eight milestones supplied on 2026-11-02 in
`/Users/davidcondrey/.codex/attachments/62f6cffb-9206-4aa7-86cf-5b154f6a7770/pasted-text-1.txt`.
None is replaced by an easier development gate. Milestone 1 is complete for both
original GloVe sizes and both specified beam widths. Milestone 2 passes its frozen
three-corpus, 100K-row qualification; milestones 3–8 remain open.
The starting revision is `ab978afd` on `main`. The user authorized committing,
merging and synchronizing all session work, including temporary merge-restriction
bypass. PRs #377, #379, #380 and #384 are merged; original branch protection is
restored exactly (`remote-pr-merge-receipt.json`). #384 fixes issue #381; the
README source-tag correction resolves the unavailable-crate claim in #382.
Large evidence files remain outside git with verified copies and hashes in
`external-evidence.json`, following CONTRIBUTING.md.
Development uses local/free resources only; paid services are limited
to explicitly authorized live benchmark runs.

The user's latest direction is original scientific progress, with competitive
reproduction serving as a control. [RESEARCH.md](RESEARCH.md) records three
mechanistic hypotheses, established prior art, and falsification requirements.
The first frozen information-navigation probe completed on exposed data only:
all three degree-based navigation proxies failed promotion; see
[INFORMATION.md](INFORMATION.md). A fast Walsh-codebook idea was screened out as
established linear-code structure before implementation. No novelty claim is made.
A guarded tensor-association probe also failed its frozen 2× gate after correcting
an unfair control layout; every tested winner remained correct. It retains raw
input and supplies no memory saving. See [ASSOCIATION.md](ASSOCIATION.md).
[MUTATION.md](MUTATION.md) gives an exact-arithmetic counterexample to discarding
common score offsets: identical readouts for every cue can require different
answers after the same deletion. Future compression candidates must account for
the information their mutation contract needs; this is analysis, not a novelty claim.
[QUOTIENT.md](QUOTIENT.md) applies exact observability closure: whole-row changes
permit a smaller score-contrast state, while individual-cell changes force full
rank in all three declared shapes. All 729-state, 17,496 mutation, 30,618 readout
and 2,000 sequence checks passed. This is a restricted-contract result using known
control theory; it does not establish byte savings, provenance storage or an invention.
The subsequent [winner-only identifiability screen](IDENTIFIABILITY.md) recovered
all 2,115 declared positive tables up to global scale using 48,306 queries.
It extends the specific masking-contract constraint beyond linear encodings;
it does not establish an impossibility result for general memory or an invention.
[SOURCE-LIFT.md](SOURCE-LIFT.md) now gives a constructive bounded-withdrawal
certificate using representative-family algebra. All 6,000 exhaustive cases /
22,713 states, 30 tight constructions and 12 larger cases preserve exact evidence
semantics. Its predefined size screen failed against the stronger exhaustive
control; the receipts retain that tradeoff. Natural-language utility and novelty
remain unestablished.
[SOURCE-REFINEMENT.md](SOURCE-REFINEMENT.md) completed 12 further cases /
209,868 withdrawal states with exact evidence semantics. A stronger proof-guided
construction reproduced the exhaustive certificates using 6–116 basis calculations
per case, invalidating the more elaborate lift's early speed advantage in all
12 cells. This supplies a constructive research lead; shared-host development
timings and supplied equations do not qualify system performance or agent utility.
[TEMPORAL-TRACE.md](TEMPORAL-TRACE.md) extends the tested contract to nonlinear
temporal decisions: 3,932 cases / 86,846 snapshot-withdrawal checks preserve exact
answers and read provenance. Positive-only certificates fail when old facts return.
The overall size screen failed because shared source dependencies erased most of
the independent-source saving. An established symbolic-compilation control then
produced identical packets and was faster in all 12 cohort cells; the proposed
compiler supplies no demonstrated algorithm advantage.
[SOURCE-PREDICTION.md](SOURCE-PREDICTION.md) now tests a different representation:
source-aware predictive state models. Exact Boolean identities preserve normalized
predictions and unknown states under unlimited source withdrawals. The profile
screen passes 81,120 comparisons; full state refinement passes 16,908 transition
checks and 1,074 sequential withdrawals over 108 models. Both symbolic controls
produce identical quotients. The algebraic profile check is faster on constructed
cells, while independent-history source support prevents compaction entirely.
The subsequent [real-trace screen](PREDICTIVE-MINIWOB.md) retains 384 human
demonstrations, including empty/unmatched events. After correcting generated
text-reference identity, full refinement preserves 105,534 checked readouts and
48 retractions but saves only 6–9% on the date task and grows packets 2–4% on the
two checkbox tasks. State dictionaries and maps are charged.
Separating reusable control from source bindings then preserves the same checks
but saves only 2–4% against compressed exact controls on target-path actions,
failing its 20% screen; see [PREDICTIVE-FIBERS.md](PREDICTIVE-FIBERS.md).
The [deletion-bound construction](WITNESS-TIGHTNESS.md) then proves the `2g`
distribution-witness bound sharp even with strictly positive answers, under the
contract permitting background evidence. All 11 families / 8,188 comparisons
pass; its portable bundle reproduces thirteen evidence files byte-for-byte.
The subsequent [matrix construction](MATRIX-WITNESS.md) proves the same sharp
bound without background evidence: every record has exactly `g` withdrawable
dependencies. Five families / 37,448 comparisons pass, with matching unknown
states and positive known outcomes. Its fresh extraction reproduces seven output
files byte-for-byte; instructions and archive hashes are in [REPRODUCTION.md](REPRODUCTION.md).
The [source-count refinement](AMBIENT-WITNESS.md) then makes the distribution
witness bound `min(2g,n−g)` and constructs matching positive-evidence examples
for every mathematical source-count regime. Its 32 finite families / 37,410
comparisons pass, with 408 distinguishing masks and matching unknown states.
Known-versus-unknown discrepancies still require a separate support check.
No original system advantage or milestone completion is inferred from these results.

| Milestone | Required evidence | Current state / next action |
|---|---|---|
| 1. Recall failures | Reproduce ef=128/256 and classify every missing neighbor by reachability, expansion, admission and eviction; distinguish topology from bounded traversal. | Complete: 4,000 ordered rankings reproduced across 10K/100K and ef=128/256. All 1,308 misses are reachable, all absent before evaluation; shortest-path witnesses have 1,268 pre-expansion evictions and 40 rejections. Every incoming neighbor and witness-path vertex is traced. See [RECALL.md](RECALL.md) and raw receipts. |
| 2. Shipping accuracy | Policy frozen before >=3 independent corpora with >=1,000 held-out queries each; >=99% mean and >=90% worst recall@10 in every declared workload; no unexplained exact membership errors. | Complete for the frozen 100K-row workloads: Last.fm, COCO-I2I and DEEP1B pass all 72 cells / 72,000 query checks. Unfiltered graph means 99.98%, 99.99%, 99.99%, each worst 90%; exact/filtered means and worst 100%; empty results correct. Zero unexplained exact membership or payload errors. Policy stayed degree64/build256/two-hop/ef128. See [ACCURACY.md](ACCURACY.md). |
| 3. Competitive performance | Strong pinned competitors matched for recall, payload, durability and resources; >=10% lower median and p95 in each of 3 repetitions; p99 <=5% worse; filters, churn, cold start, memory included. | All six exposed corpus/engine calibrations completed with eligible selections. Synthetic qualification and serving-job audits pass; calibration audit v3 also passes without opening held-out vectors. The controlled diagnostic confirms a 5–17x filtered-tree slowdown; the bounded-traversal prototype passes its frozen 20-cell timing gate and 3,840 result comparisons. Its integrated patch passes 154 document tests and clippy plus 100 Linux server tests. WARM-only recalibration v5 and its 32,000-response audit pass with that new server; exposed filtered timings improve 1.7–4.9x but remain slower than the strongest competitor. The sealed 18-job Mac comparison and all raw audits completed: 576 cells / 576,000 requests, no request errors, 84 quality failures. COCO fails the fixed worst-query floor for all engines; the DEEP target throughput ratios are 1.895×, 1.919× and 2.144× against eligible OpenSearch cells. Original resource sampling missed detached PostgreSQL children; whole-process resource qualification and host isolation also fail. No competitive claim qualifies. See [PERFORMANCE.md](PERFORMANCE.md). |
| 4. Memory utility | Current/historical facts, contradictions, provenance, forgetting; any answer-quality claim needs >=5 percentage points at equal model/context budgets and paired 95% CI excluding zero. | 59 native memory/conversation/context contract tests passed. Installed HTTP SDK passes 12 live cases and two recovery cases on each of Python 3.11.14/3.14.7. Existing 100-question development evidence does not establish an answer-quality advantage over its stronger control; held-out utility remains unverified. See [MEMORY-UTILITY.md](MEMORY-UTILITY.md). |
| 5. Sustained operation | 24-hour mixed-workload soak plus supported crash/recovery; zero observed lost acknowledged writes, partial batches, duplicate retry effects, deleted or cross-tenant results; declared resource/latency ceilings. | Development 001 failed admission; explicit 8 slots/32 queue fixed it. Development 002 passed integrity/recovery but failed p95 (0.538 s >0.250 s). Receipt erasure now skips already-bodyless protected receipts. Development 003 passed all gates: 1,672 requests, p95 0.0818 s, p99 0.2871 s, both recoveries <0.2 s. Frozen 24-hour qualification started 2026-11-03T05:40:28Z on WSL; results pending. |
| 6. Noteworthy advantage | Preselect >=2x throughput, >=50% lower whole-process memory, or >=30% fewer context tokens against strongest matched baseline on 2 held-out workloads and 2 hardware configurations, preserving quality. | Preselected >=2x completed-query throughput on maintained 100K COCO/DEEP workloads, eight clients, test ordinals 1000–1999, all three repetitions on Mac M4 and Intel Ultra9. See [PERFORMANCE.md](PERFORMANCE.md); the frozen Mac comparison fails quality on COCO and the 2× threshold in two of three DEEP repetitions. Original whole-process accounting is incomplete; its repair passes 15 focused tests each on macOS and Linux. Windows and WSL are the same physical computer; the new Mac guest also adds no physical configuration. |
| 7. Supported product | Every advertised API/platform: fresh install, ingest/search/update/forget/reopen, migration, recovery; no release-blocking defects or unexplained skips. | Node package/migration/recovery checks pass on macOS and Linux arm64. C/Go/Java/.NET fresh packages and eight-death/32-reader recovery pass on each. macOS native Python passes three runtimes, including independent verification of warning-affected oracle values; Linux native Python passes three runtimes on the produced glibc 2.34 wheel. HTTP SDK live/recovery, six framework adapters and both DSPy adapters pass scoped checks; all 893 current native library tests also pass, with twelve explained diagnostic exclusions. The fifteen-profile integration rerun passes 389 tests with two explained optional Haystack exclusions after fixing the mem0 expiry sentinel; no failures remain in that scoped run. The installed WASM package passes 54 runtime lifecycle tests, Chrome/Firefox persistence, webpack, and 18-writer/54-reader recovery on macOS arm64. macOS arm64 also passes 18 UniFFI tests and seven tests each in fresh Kotlin/Swift consumers; a fresh iOS 27 arm64 simulator consumer passes all seven Swift tests. Android API 35 arm64 passes ten native checks, nine lifecycle cases and both completed-snapshot process-death recoveries after fixing unsupported standard-library file locking. All four Android ABIs build; remaining targets and workflows stay open. See [PLATFORMS.md](PLATFORMS.md). |
| 8. Independent reproduction | Another evaluator reproduces quality gates and advantage from published source, inputs and raw results, including failures/tradeoffs. | Portable temporal and deletion-bound bundles pass clean local packaging checks; neither establishes a system advantage. The user will independently reproduce the results; the complete system bundle and their returned results remain required. See [REPRODUCTION.md](REPRODUCTION.md). |

## Host access

Local Tailscale was stopped; restored with its existing `--accept-routes` setting.
WSL SSH is reachable after the user's restart. It reports Intel Core Ultra 9 185H,
22 logical CPUs, 27 GiB visible RAM, Ubuntu-24.04, and enabled/active ssh and
tailscaled services. Windows was reachable through WSL interop while its direct
Tailscale SSH address was offline. Existing Windows configuration reports automatic
sshd/Tailscale, zero AC/DC idle sleep/hibernate timers, `.wslconfig` vmIdleTimeout=-1,
and running task `WSL-Tailscale-Keepalive` with a live `/bin/sleep infinity` process.
Enabled the task's `StartWhenAvailable`, preserved its unlimited runtime and
999 one-minute restarts, set AC sleep/hibernate/lid-close to never sleep, restarted
the stalled Windows Tailscale service, and enabled/verified unattended mode.
Windows configuration backups are at
`C:\Users\David Condrey\AppData\Local\WritersLogic\host-availability`.
The local SSH alias had an incorrect Windows username (`David Condrey`); corrected
it to the verified account `davidcondrey`, preserving the original configuration at
`/Users/davidcondrey/.ssh/config.before-warm-host-availability-20261102`.
Both hosts are now reachable with key authentication. Battery lid-close sleep is
unchanged. No SSH authentication or firewall policy was weakened.

## Current implementation

`VGraph::diagnose_navigation` is experimental-only. It records coded beam
admission/rejection, eviction, expansion, recovery seeds, scores, and cutoff scores;
it fails if replay differs from the serving visited set, ordered beam, expansion
count, or results. The serving search loop is unchanged. `trace ... navigation`
exports decoded serving adjacency and all 2,000 query event streams. These
diagnostics use the exposed GloVe inputs solely to explain existing failures.

## Validation and running work

Graph tests: 144 passed initially; the sole failure was an absent golden fixture in
the sparse build checkout. Restoring the existing fixture and rerunning that test
passed, giving 145 passing tests. Ten ignored cases are explicitly timing/memory
microbenchmarks, not missing correctness checks. New navigation and serving-parity
tests passed. The analyzer's three regression tests and the driver test pass.
Experimental-library clippy passed with warnings denied; rustfmt checks passed
for the changed diagnostic/driver/durable-receipt files. The information probe's
four correctness/input-streaming tests pass. All 59 existing memory/conversation/
context API contract tests passed locally with conversation-memory and security:
30 memory API, 15 conversation memory, 14 context; zero failed/ignored. Output is
in `memory-contract-tests.log`; this is not an end-to-end QA gain.
Logs are retained here. The initial full checkout/build was slow because Cargo
enumerated the large working directory and git checkout invoked LFS; a sparse
isolated checkout at `/Users/davidcondrey/.cache/warm-milestone-navigation`
resolved this. Its research target is
`/Users/davidcondrey/.cache/warm-milestone-navigation-target`.
An interrupted unused checkout remains at `/Volumes/A/warm-milestone-navigation`.

Remote server source/build/evidence: `/home/davidcondrey/warm-milestones-20261102`
on `writerslogic-02`; the source is a restricted archive of `ab978afd` plus the
receipt-erasure patch. Baseline and fixed release builds passed. All 10 durable
receipt tests and 13 server operations tests passed. Server API tests initially
passed 23/24; the remaining test lacked `docs/server/openapi.json` in the restricted
archive and passed after restoring that existing fixture. Logs are under
`evidence/`, with receipt/operations/recheck logs copied here. The first offline build lacked cached
candle 0.11.1 metadata; an ordinary free crates.io fetch/build succeeded.

The running qualification uses pinned copies under remote
`evidence/soak-qualified-source/`, including the executable, harness, patch and
SHA256SUMS. Controller launcher PID 14118; evidence directory
`evidence/soak-qualification-001/`; outer log `evidence/soak-qualification-001.log`.
Inspect its `finished.json` before assuming it remains active. Binary SHA256:
`f25ed36d7f65cab826873e8211ef9e4b6a6766c30784cda10dcdc13172bbb800`.
The original UTC projection was 2026-11-04T05:40:28Z plus overhead. A fresh
2026-11-03T12:07Z check shows about 5h53m of monotonic workload time, fresh resource
samples and the original live server. Frozen UTC start and current wall/monotonic
alignment differ by about 34 minutes; the cause is unestablished. The remaining
monotonic duration now projects completion around 2026-11-04T06:14Z plus recovery.
The harness still requires all 86,400 workload seconds; elapsed wall dates alone
do not qualify it. Receipt: `soak-progress-20261103T1207.json`.
A fresh 13:21Z read-only check finds the same server/controller alive, 25,170.68
monotonic workload seconds, 174,166,016 combined RSS bytes and 446,046,208 allocated
store bytes, with resource samples updated one second earlier and no terminal
result. About 17 workload hours remain; no premature pass is inferred from UTC
start projections. Receipt: `soak-progress-20261103T1321.json`.
At 14:41Z, both SSH aliases are reachable and the original soak processes remain
alive: 29,504.40 monotonic workload seconds, 249,548,800 combined RSS bytes,
521,904,128 allocated store bytes and no terminal result. About 15.8 workload
hours remain. Receipt: `soak-progress-20261103T1441.json`.
At 15:12Z the same processes remain alive with 31,201.66 monotonic workload
seconds, 255,877,120 combined RSS bytes and 551,649,280 allocated store bytes;
no terminal result. Receipt: `soak-progress-20261103T1512.json`.
At 15:59Z both SSH aliases respond; Linux SSH/Tailscale are active and Windows
SSHD/Tailscale are running with automatic startup. The same soak processes have
33,727.17 monotonic workload seconds, 293,933,056 combined RSS bytes and
595,836,928 allocated store bytes, with fresh samples and no terminal result.
About 14.6 workload hours remain. Receipt: `soak-progress-20261103T1600.json`.
At 16:39Z both SSH endpoints still respond with those services active/automatic.
The original soak processes have 35,898.16 monotonic workload seconds,
258,932,736 combined RSS bytes and 633,888,768 allocated store bytes; fresh
samples, no terminal result, and about 14.0 workload hours remain. Receipts:
`soak-progress-20261103T1640.json`, `windows-services-20261103T1640.json`.
At 17:42Z the same processes have 39,299.43 monotonic workload seconds,
261,677,056 combined RSS bytes and 693,223,424 allocated store bytes, with fresh
samples and no terminal result. Both SSH endpoints respond; Linux services are
active and Windows services remain running/automatic. About 13.1 workload hours
remain. Receipts: `soak-progress-20261103T1743.json`,
`windows-services-20261103T1743.json`.
Do not rebuild or replace this frozen executable during the run.

Concurrent unrelated modifications appeared in `scripts/ci/check-versions.py`
and `scripts/ci/test_versions.py`; this work has not edited those files. Preserve
them and do not include them in the milestone patch or assume their ownership.

Accuracy qualification protocol and drivers are in
`benchmarks/public/accuracy_qualification/`. The pre-input plan and extracted
input receipts are sealed under
`/Volumes/A/.hms-bench-cache/warm-accuracy-inputs-20261103-v1`; all three preparations
completed without excluding any selected row or query. NumPy 2.5.4 and h5py 3.16.0
are pinned by the dedicated uv.lock. Two extraction tests and three evidence-gate
tests pass. A fresh macOS/arm64 node-api+security release build passed; its pinned
copy, source, wrappers, original oracle/worker, all jobs and raw outputs are under
`/Volumes/A/.hms-bench-cache/warm-accuracy-20261103-v1`. Completed successfully at
2026-11-03T06:37:28.765Z; all 72 workload cells pass, and source/artifact hashes
match the freeze. Terminal/freeze copies and `accuracy-run.log` are here.
No query timing or competitive claim is made
from this shared-host accuracy run. Exact boundary substitutions remain unresolved
until individually explained; the worker's generic tolerance alone cannot qualify
them. Preserve the frozen query cohort and all failed quality cells.

The soak harness is `benchmarks/public/server_qualification.py`; its three
oracle regression tests passed with `uv run --no-project python -m unittest
discover -s benchmarks/public -p test_server_qualification.py`. It freezes source,
binary and script hashes, rejects non-development runs shorter than 86,400 seconds,
and validates concurrent atomic batches, replacement, churn deletion, filters,
tenant authorization, old-request retries, compaction, and ledger equality after
both abrupt and orderly restarts. Qualification limits remain 2 GiB combined
server/controller RSS, 8 GiB allocated store, p95 <=250 ms, p99 <=1 s,
single request <=5 s, recovery <=60 s. Resource samples occur every five seconds.

A dedicated Linux arm64 Lima guest is ready on the Mac: `LIMA_HOME=/Volumes/A/.hms-lima`,
instance `warm-arm64`, four CPUs/8 GiB, Ubuntu 24.04.5. Native Node/server builds
and package lifecycle checks completed successfully, after restoring
registered Cargo example/bench source paths omitted from the first source archive.
Guest root is `/home/davidcondrey.guest/warm-milestones-20261103`; terminal receipt
and logs are copied to `linux-arm64/` here. All 433 source-file hashes and installed
package bytes match the frozen artifacts. The guest remains available; the matched server comparison completed and failed
its claim gates. Installed SDK and framework lifecycle evidence is recorded above.
Host log `linux-arm64-build.log`; source hashes and reproduction details are in
[PLATFORMS.md](PLATFORMS.md). The other existing Lima guest is unchanged.


## Universal Codex autopilot example — 2026-11-02

The global `$autopilot` skill and local `project-autopilot` MCP server now preserve per-project plans, isolated workspaces, exact-source verification receipts and resumable checkpoints. The first WARM cycle reproduced nine npm lock-metadata inconsistencies that the release gate accepted, fixed `scripts/ci/check-versions.py`, and added regression coverage in `scripts/ci/test_versions.py`. Both files were integrated only after their original hashes matched the captured baseline. The three-test version suite and current-version gate pass under both Python 3.14 and CI's Python 3.12; the existing stable/prerelease bump assertions remain unchanged.

Local evidence: `/Users/davidcondrey/.codex/project-autopilot/examples/warm-integration.json`, `warm-integrated-checks.log`, and controller project `e7afe3de18e1fe80` (checkpoint `release-lock-consistency`). The controller's 23 tests and actual Codex tool/skill discovery passed. This qualifies the local workflow and release-lock fix only; it does not qualify M0–M8 or the complete application. No paid development calls, commit, push or release were performed by this cycle. Concurrent navigation, durability and soak work was preserved.
