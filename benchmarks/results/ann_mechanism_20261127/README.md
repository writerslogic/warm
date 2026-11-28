# ANN mechanism and paired factor, 2026-11-27

Question: why is WARM faster than hnswlib on SIFT-1M at recall@10 >= 0.95, and does the factor hold on paired, same-host runs across hardware classes.

Source: origin/main `dbe04ad6` plus `benchmarks/competitive/ann_mechanism.py` and the additive `count` mode in `hnswlib_cpp/driver.cpp` (both in this PR). Hosts: GCP n2 Ice Lake 8c 16G (Xeon) and c3d Genoa 8c 16G (EPYC 9B14). c4a was not run. Single thread, pinned (`taskset -c 2`), fresh process per observation, rounds rotate system order, foreign-CPU load gate. The operating point is chosen on even queries; QPS and recall are reported on odd queries. Factor CIs are paired-by-round bootstraps (`common.round_ratio_ci`, 95%).

Files
- `heavy-ann-*.json`: raw interleaved runs (WARM, hnswlib python, hnswlib-cpp, faiss-hnsw, usearch). `.latency.npz` sidecars are gitignored; the JSON records their sha256.
- `factor-heavy-ann-*.json`: per-target factor with CI (`ann_mechanism.py headline`).
- `ablation-*.json`: WARM variants interleaved with hnswlib-cpp and hnswlib python, SIFT only.
- `mechanism-*.json`: mechanism table (`ann_mechanism.py report`), including hnswlib true distance-evaluation counts from the counting-space driver (`ann_mechanism.py evals`).
- These files live outside `results/competitive/`, so `warm-bench table` does not read them; `table --check` is unaffected.

## Factor WARM / hnswlib-cpp, SIFT-1M (8 rounds)

Grid is the first ef-grid point reaching the recall target; interpolated is log-QPS interpolation at the exact target (removes ef-grid overshoot).

| host | recall | grid | interpolated |
|---|---|---|---|
| n2 Ice Lake | 0.90 | 2.58 [2.40, 2.74] | 2.50 [2.34, 2.67] |
| n2 Ice Lake | 0.95 | 2.27 [2.14, 2.44] | 2.47 [2.33, 2.64] |
| n2 Ice Lake | 0.99 | 2.14 [2.05, 2.28] | 2.24 [2.13, 2.39] |
| c3d Genoa | 0.90 | 2.03 [1.98, 2.08] | 1.98 [1.93, 2.03] |
| c3d Genoa | 0.95 | 1.85 [1.77, 1.94] | 2.02 [1.94, 2.09] |
| c3d Genoa | 0.99 | 1.82 [1.79, 1.90] | 1.92 [1.84, 1.97] |

Against hnswlib python the factors are within about 0.1 of these (2.16 to 2.52 on Ice Lake, 1.87 to 2.15 on c3d); against faiss-hnsw they are larger (2.2 to 3.0). usearch is slower still. Full tables with every system are in the `factor-*` files.

## Factor, GloVe-100 (6 rounds, ef capped at 768)

| host | recall | hnswlib-cpp grid | interpolated |
|---|---|---|---|
| n2 Ice Lake | 0.90 | 2.19 [2.13, 2.32] | 2.37 [2.26, 2.55] |
| n2 Ice Lake | 0.95 | 2.23 [2.12, 2.35] | 2.27 [2.16, 2.38] |
| c3d Genoa | 0.90 | 2.52 [2.51, 2.53] | 2.69 [2.68, 2.70] |
| c3d Genoa | 0.95 | 2.56 [2.53, 2.59] | 2.59 [2.57, 2.63] |

Recall 0.99 is not reached by the comparators within ef <= 768; no 0.99 factor is claimed on GloVe.

## Mechanism table, SIFT-1M

Each row is a WARM variant against the base build (degree 64, build_ef 500, 8-bit vertex codes, residual code, reorder, aligned rows, 3-byte ids, `rerank 32,64`), same host, interleaved. Value is base QPS / variant QPS (above 1 means the base choice helps; the interval is a paired bootstrap, grid / interpolated). Ice Lake, recall 0.95: 

| factor | variant | base/variant |
|---|---|---|
| traversal code width | other bit width (`bits-other`) | 1.59 [1.58, 1.60] |
| residual code | off | 1.28 [1.26, 1.29] |
| float rerank | off (serve time) | 1.27 [1.26, 1.28] |
| graph degree | 32 | 0.94 [0.93, 0.95] (WARM faster at lower degree; interpolated 1.02) |
| build_ef | 200 | 1.01 [0.99, 1.02] |
| row reorder | off | 0.97 [0.95, 0.99] |
| id width | 4 bytes | 1.01 [1.00, 1.02] |
| row alignment | off | 0.97 [0.95, 0.98] |
| full build search | on | 1.01 [1.00, 1.02] |
| timing boundary | hnswlib-cpp / hnswlib python | 1.03 [1.02, 1.04] |

c3d at recall 0.95: bits-other 1.42 [1.39, 1.44], no-residual 1.23 [1.22, 1.25], degree-32 0.90 [0.88, 0.94], build-ef-200 1.01, no-reorder 1.06 [1.05, 1.08], id-4-bytes 1.00, layout-off 1.07 [1.03, 1.13], full-build-search 1.01, rerank off 1.24 [1.22, 1.26], cpp/python 1.06 [1.03, 1.08]. Recall 0.90 and 0.99 rows are in `mechanism-*.json` (the 8-bit alternatives and rerank-off do not reach 0.99).

### Evaluation decomposition (hnswlib distance calls via counting space vs WARM estimates per query)

QPS ratio = (evaluation ratio) x (cost per evaluation ratio).

| host | recall | hnswlib evals / WARM estimates | WARM QPS / hnswlib-cpp QPS in this run |
|---|---|---|---|
| Ice Lake | 0.90 | 1.08 | 2.92 |
| Ice Lake | 0.95 | 0.83 | 2.20 |
| Ice Lake | 0.99 | 0.84 | 2.30 |
| c3d | 0.90 | 1.08 | 2.36 |
| c3d | 0.95 | 0.82 | 1.85 |
| c3d | 0.99 | 0.85 | 2.05 |

WARM evaluates the same or more nodes than hnswlib at 0.95 and 0.99 (ratio below 1) and wins on cost per evaluation, roughly 2.6x cheaper at 0.95. The cause is the narrow vertex code: the code width is the dominant factor. Graph degree, build_ef, reorder, id width, alignment and the build-time search move the result by about 0 to 7%; the timing boundary (binding vs native call) is 2 to 6%. Rerank and the residual code are paid-for accuracy, each worth about 1.25x here.

## What weakens or qualifies the headline

- The factor is host dependent: 1.8 to 2.6x across the two classes. The previously published 1.97x came from an unpaired WARM row that was lower than this session's WARM runs. On c3d the paired factor at 0.95 is 1.85 (grid) or 2.02 (interpolated), consistent with 1.97; on Ice Lake it is higher, 2.27 to 2.47.
- Grid and interpolated factors differ by up to about 0.2 (c3d 0.95: 1.85 vs 2.02). The ef grid overshoots the recall target by different amounts per system.
- The mechanism is cheaper evaluations, not fewer evaluations. At 0.95 and 0.99 hnswlib does fewer distance calls than WARM does estimates. The advantage depends on the 8-bit traversal code, which is also why float vs 8-bit traversal could not be isolated as a separate row (no float-traversal build exists in the harness); the table isolates width by the alternative bit width only.
- The ablation grid is thinner on Ice Lake than on c3d. The ablation base/cpp factor differs from the headline run (Ice Lake 0.90: 2.92 vs 2.58) because it is a separate set of processes and operating points, a reminder of run-to-run spread.
- GloVe: 6 rounds, capped ef, no 0.99 comparison, and no mechanism ablation.
- WARM through the Python binding was not remeasured; only hnswlib was timed both ways (cpp/python in the table). WARM timing is in-process `search_into` with per-query `Instant`; hnswlib-cpp is `searchKnn` with per-query `steady_clock`.
- c4a was not run; the two classes are Intel Ice Lake and AMD Genoa.
- Run provenance: the tree was origin/main `dbe04ad6` plus the two files in this PR; `dirty_tree: false` was asserted through the environment override because the harness files were copied in.
