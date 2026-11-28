# Quantised-traversal comparators against WARM, SIFT-1M, 2026-11-28

Objection: WARM traverses the graph on quantised codes, and the paired factor in `ann_mechanism_20261127` was measured against indexes that traverse float vectors. Do the factors hold against comparators that also traverse quantised vectors?

Comparators added (all free, in `benchmarks/competitive/warmbench/ann_worker.py`):

- `faiss-hnswsq`: FAISS `IndexHNSWSQ` (M 32, efConstruction 500) wrapped in `IndexRefineFlat`. The graph search reads 8-bit (`sq8`) or 4-bit (`sq4`) scalar-quantised vectors; the top `k * k_factor` candidates are re-scored against a float copy. Sweep: ef x `k_factor` in {1, 2, 4}.
- `usearch` with `dtype` int8 (`m32e500_i8`) and float16 (`m32e500_f16`) next to its float32 build. usearch has no refinement stage.
- `faiss-hnsw` (float32 traversal, as in #548) ran in the same session as the float control.

hnswlib has no quantised mode, so it is not repeated; its factors on these hosts are in `ann_mechanism_20261127`.

Protocol is that of #548: `warm-bench run --suite heavy`, single thread pinned to one core, fresh process per observation, 8 rounds with the system order rotated, operating point chosen on the even queries and QPS reported on the odd ones, recall targets 0.90, 0.95, 0.99, paired-by-round bootstrap 95% intervals (8 rounds is below the 10 the tool wants for a published confidence interval, so read them as ranges). WARM is the `dbe04ad6` binary used in #548, run in the same session. Hosts: GCP n2 Ice Lake 8c 16G and c3d Genoa 8c 16G, both fresh clones of the #548 hosts. Source: `ann_mechanism.py headline` output (`factor-*.json`, includes the comparators' own best builds) and `quantised_tables.py` (`tables-sift.md`, every build). The raw `.latency.npz` sidecars stay out of git.

## Result: SIFT-1M, WARM QPS / comparator QPS (grid; interval in `tables-sift.md`)

| host | recall | FAISS HNSW float32 | FAISS HNSW-SQ8 + refine | FAISS HNSW-SQ4 + refine | usearch float16 | usearch int8 |
|---|---|---|---|---|---|---|
| Genoa | 0.90 | 2.69 | 2.13 | 2.48 | 4.63 | 5.65 |
| Genoa | 0.95 | 2.09 | 2.24 | 2.17 | 3.54 | not reached |
| Genoa | 0.99 | 2.37 | 1.85 | 2.14 | 3.63 | not reached |
| Ice Lake | 0.90 | 3.25 | 2.55 | 3.12 | 4.59 | 5.96 |
| Ice Lake | 0.95 | 2.45 | 2.63 | 2.67 | 3.42 | not reached |
| Ice Lake | 0.99 | 2.67 | 2.05 | 2.54 | 3.51 | not reached |

WARM QPS at 0.90 / 0.95 / 0.99: Genoa 25,423 / 15,890 / 7,274; Ice Lake 26,976 / 16,414 / 7,163.

- WARM is faster in all 26 cells where the comparator reaches the recall (30 cells minus 4 usearch int8 cells that never reach 0.95 or 0.99), and every interval excludes 1. Against the strongest quantised comparator (FAISS HNSW-SQ8 + refine) the factor is 1.85 to 2.63, against 2.09 to 3.25 for float32 FAISS HNSW. Quantised traversal narrows the gap at 0.90 and 0.99 (float 2.69 to 2.13 on Genoa at 0.90) but does not close it; at 0.95 SQ8 + refine is not faster than float FAISS HNSW (WARM / SQ8 2.24 against WARM / float 2.09 on Genoa).
- usearch is the slowest comparator in every cell in this session, float16 and int8 included. usearch int8 does not reach recall 0.95 (best 0.915): SIFT components go to 218 and the int8 conversion saturates, so that is a limit of the plain int8 cast on this dataset, not a statement about int8 usearch in general.
- Interpolated factors (QPS interpolated at exactly the target, no ef-grid overshoot) are in `tables-sift.md` and agree with the grid factors within about 20 percent.
- The 4-bit variant is slower than 8-bit at 0.90 and 0.99 and about level at 0.95; neither SQ width comes close to WARM.

## Hardware counters

Neither host exposes the guest PMU: `/sys/bus/event_source/devices` lists only software events, `perf_event_paranoid` is 4, and `perf stat -e cycles,instructions,cache-misses` fails on both hosts; no sudo is available to change that. Instructions, cycles and cache misses per query are therefore not reported for WARM or any comparator on either host. Software counts exist instead: distance evaluations per query for hnswlib and WARM code estimates per query in `ann_mechanism_20261127`. FAISS distance-evaluation counts were not collected in this run.

## Not in this directory

- GloVe-100 (1,183,514 x 100, angular): not measured. The chain started GloVe on both hosts after SIFT and had completed only 2 of 8 serve rounds on each when it was stopped at the 14:00 UTC deadline (about 8 hours of GloVe wall clock; the angular sweep goes to ef 768 and 3 refine factors across 5 builds); a heavy run writes its result file only at the end, so nothing partial exists. No GloVe claim is made, and the SIFT result does not extend to it. The #548 float-traversal factors on GloVe-100 stand as they were.
- Build time and index size per comparator are in each `heavy-ann-*.json` under `systems.<name>.builds.<build>`.
