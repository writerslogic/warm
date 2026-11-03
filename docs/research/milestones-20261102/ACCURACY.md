# Shipping accuracy qualification

Milestone 2 passes at the frozen 100,000-row scale and declared workloads. All
72 cells completed with 1,000 queries each. Every cell satisfies mean recall@10
at least 99%, worst-query recall at least 90%, and zero unexplained exact
membership errors. No policy was selected or changed from these results.

## Frozen policy and inputs

The public `VectraMigrationIndex` default used `highRecall=false`, degree 64,
construction ef 256, shipping two-hop traversal, ef/rerank 128, full dimensions,
and k=10. All WARM/HMS environment knobs were absent. The source revision was
`ab978afd94a2d92a89c162c1e2af77372ffeeb16`; the macOS arm64 Node binding was freshly
built with `node-api,security` and copied before execution. Its SHA256 is
`bd38731775e91b6728661ac23b94fefe55a7509d029b73fe6b69d6c6b43fec7b`.

The [plan](../../../benchmarks/public/accuracy_qualification/PLAN.json) was sealed
before download. It selected the first 100,000 finite, nonzero float32 training
rows and first 1,000 eligible published test queries in source order. No selected
row or query required exclusion. Last.fm, COCO-I2I and DEEP1B represent independent
published corpora; GloVe and NYTimes were excluded because of prior development
exposure. A targeted repository search found no earlier references to these three
exact dataset names; it cannot establish absence of exposure outside this project.

Dataset identities and producer URLs follow the
[ANN-Benchmarks collection](https://github.com/erikbern/ann-benchmarks), including
the [COCO producer release](https://github.com/fabiocarrara/str-encoders/releases/tag/v0.1.3).
Last.fm's angular augmentation has 65 coordinates despite the `64-dot` filename.
This experiment evaluates cosine on the downloaded angular vectors, with its own
oracle; it does not claim qualification of an independent raw-dot-product API.

## Results

The unfiltered graph results below hold in baseline, post-churn maintenance and
separate-process reopen. Each displayed workload uses all 1,000 fixed queries.

| Corpus | Dimensions | Graph mean recall@10 | Graph worst recall@10 | Exact mean/worst | Cells passed |
|---|---:|---:|---:|---:|---:|
| Last.fm | 65 | 99.98% | 90% | 100% / 100% | 24/24 |
| COCO-I2I | 512 | 99.99% | 90% | 100% / 100% | 24/24 |
| DEEP1B | 96 | 99.99% | 90% | 100% / 100% | 24/24 |

Per corpus, baseline exact and graph each contribute four filter cells; churn
graph contributes four cells before and four after maintenance; churn exact and
separate-process reopen contribute four each. Filters admit all, 10%, 1%, or zero
rows. The 10% and 1% cells all have 100% mean/worst recall, and every empty-filter
query returns no results. Replacements change ordinals 0–99 to vectors 1–100;
deletions remove ordinals 100–199.

Unfiltered graph cells report `fullGraph`. Before maintenance the changed store
reports `fallbackExact` and preserves exact recall. Filtered cells report
`filteredExact`; exact mode reports `fullExact`. These are observed routes, not
assumed labels. Payload, metadata, raw-vector, duplicate, deleted-result and
eligibility checks all pass. The oracle and scalar score checks are independent
JavaScript implementations. No exact membership substitution required a numerical
tolerance explanation: the stricter analyzer found zero unresolved substitutions.

## Evidence and limits

Source, executable, wrapper, worker, controller and extracted-input hashes were
sealed before search. Final source/package/worker hash checks passed. The run
completed at `2026-11-03T06:37:28.765Z`. Three analyzer regression tests and two
input-selection tests passed before execution.

- Inputs, original HDF5 downloads, ordinals and receipts:
  `/Volumes/A/.hms-bench-cache/warm-accuracy-inputs-20261103-v1`.
- Frozen implementation, all 72,000 query records, oracles, jobs, process resource
  samples and terminal receipt:
  `/Volumes/A/.hms-bench-cache/warm-accuracy-20261103-v1`.
- Repository copies: `accuracy-freeze.json`, `accuracy-finished.json`; phase log
  `accuracy-run.log` and build/test logs in this directory.
- Freeze SHA256: `0d5909b5ab4b486bfd96dd1c69616c970e2b1e0faf9745dd04701aaa00f0f06d`.

Peak sampled worker-tree RSS was 1,389,772,800 bytes for Last.fm, 3,786,964,992 for
COCO and 1,956,954,112 for DEEP1B. Resource samples occur once per second and can
miss shorter peaks. All phases stayed below the declared per-process 8 GiB limit
and two-hour deadline. Timing output was deliberately disabled on the shared
host. These results do not establish competitive speed, memory superiority,
larger-scale accuracy, natural-language utility, or a research invention.
