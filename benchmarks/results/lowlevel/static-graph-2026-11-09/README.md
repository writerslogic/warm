# Static graph evidence, 2026-11-09–10

See [the research report](../../../../docs/research/LOWLEVEL-SYSTEMS.md#static-graph-follow-up-2026-11-09)
for measured outcomes, negative results and comparison limits. This directory
contains full official VectorDBBench result JSON, separate query-replay data,
source patches and validation receipts. Binary modules, complete build logs and
input/image archives remain in the durable benchmark archive; no dataset or
compiled library is checked into Git.

- `phase1/`, `phase1-harness-summary.json`: component ablation against main,
  with published 0.6.4 outer controls; all 1,000 Cohere queries.
- `phase2/`, `phase2-harness-summary.json`: integrated/released comparison,
  including existing within-query grouping on/off and a fresh index build.
- `shared/`, `shared-harness-summary.json`: eight full runs under 8 logical
  CPUs, 16 GiB and no swap. Both the initial per-worker-hash implementation
  (`local-shared-*`) and final publisher-binding implementation (`local-bound-*`)
  are retained, including slower shared results. Each run includes all QPS
  levels, latency quantiles, cgroup memory peak/events, timestamps and exit code.
- `query-final-*.json`, `final-mac-summary.json`: M4 query replay, not official
  harness throughput. Ordered-ID hashes compare all queries at six efforts.
- `rerank-bound-probe.*`, `filter-*`, `narrow-*`: rejected pruning and narrower
  filter-kernel experiments. Their results are not presented as adopted gains.
- `source-commit-map.json`: old measured commits and signed/DCO replacements
  have identical tree hashes. Native final source is `4299b539`; the snapshot
  publisher protocol is `699c480e`; baseline main is `431722cf`.
- `validation/`, `validation-summary.json`: focused and full final gates.
- `cloud-manifest.json`, `cloud-live.sh`, `cloud-run-one.sh`, `cloud.Dockerfile`:
  protocol frozen before Azure measurement. The harness is VectorDBBench
  `4c6bdb566423d39afc2287bd27c4a3787eb80113`; shared-source arms additionally
  apply `shared-binding-adapter.patch`. Released/main/private arms do not take
  that optional adapter branch. `shared-adapter.patch` is the retained earlier
  per-worker-hash alternative.

Recompute the shared summary from the repository root:

```sh
python3 scripts/lowlevel/summarize_vdbbench.py \
  benchmarks/results/lowlevel/static-graph-2026-11-09/shared
```

The full CLI in `cloud-run-one.sh` uses Performance768D1M, k=100, nq=1,
ef=rerank=150, M=16, build_ef=300, concurrency 1/5/10 and 30 seconds per level.
The first released run builds the index; subsequent runs validate and reuse it.
The source modules are built from the same compiler/base image and Cargo
features, without CPU-specific RUSTFLAGS. `WARM_QGRAPH_BATCH_EST` is explicitly
0 or 1. Shared snapshot creation occurs before the official timer; its setup
and total wall times are separately recorded in `snapshot.json`, and its memory
is charged to the same cgroup as the workers. Worker initialization stays inside
the official throughput timer.

Local archive SHA256:

| Archive | SHA256 |
|---|---|
| Phase 1 | `2d386170224e73635b209ef11241eb8d5546e64e4fef470e7f4d2440cf32ca60` |
| Phase 2 | `d77cd136de301fcf82656d7db3ee6c279ff16fc23567e7a8e970de05588facda` |
| Shared study | `5107b46861bb4adbed3eb0489bf5563cb137ea6085e75a10d5e9f143b627e255` |

These are submitter measurements. Previously exposed public queries are not an
untouched holdout. Query-quality settings were not changed during this study.
The Azure run and its final archive receipt will be added after completion;
local results do not establish official standing or a matched-service win over
Zilliz Cloud.
