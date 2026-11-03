# Query-reinforced edge repair (issue #165)

**Verdict: park.** The repair does not win end to end on held-out queries on either dataset.
The code stays on branch `issue/165-edge-repair` as an explicit, default-off build step
(`Graph::apply_query_repair`, `Graph::build_with_repair(.., None)` is the legacy build).

Ship rule, fixed before any run: the repair (`full` arm) ships as a default-off option only if,
on **both** glove-100-angular and sift-128-euclidean, on held-out evaluation queries, it beats
**both** `base` and `control_random` in recall@10 at matched or better QPS across the ef sweep,
with its build seconds and index bytes reported. Anything less parks it.

## Protocol

- **Data**: `~/.cache/hms-bench/{glove-100-angular,sift-128-euclidean}` (ann-benchmarks HDF5
  converted by `benchmarks/public/prepare.py`). **Full scale**: glove 1,183,514 x 100 base rows,
  sift 1,000,000 x 128; plus a glove 200,000-row prefix with denser training. Exact top-10 is
  recomputed by f32 brute force; at full scale it agrees with the shipped `neighbors.i32` on
  1.0 (glove) and 0.99932 (sift) of evaluation top-10 ids (`truth.shipped_top10_agreement_on_eval`).
- **Split**: the 10,000 `test.f32` rows are permuted (splitmix64 Fisher-Yates, `split_seed`
  165). The first 5,000 are construction (train) queries, the other 5,000 are the only queries
  evaluated. Both id lists and their FNV hashes are in each report (`split`). The 200k run adds
  27,768 base rows after the prefix (rows 200,000..227,768) as further train queries.
- **Base graph** (the current `native_harness.py` warm config): degree 32, build_ef 200,
  alpha 1.0, seed 0x5EED; encoded `VGraph::from_graph_with(8, 8, align, no reorder, 3-byte ids,
  no screen)`; search k 10, rerank 16, patience 0, `SearchKnobs::OFF`, one thread. Built once per
  run; every arm reloads the same base from its graph cache and is changed in memory.
- **Arms**: `base`; `failure` (edges from the nearest reached vertex toward each true top-10
  neighbour an exact ef-32 beam missed, for every failing train query); `failure_antihub`
  (+ in-edges from their own nearest out-neighbours for every vertex of in-degree < 16);
  `full` (+ the upper layers replaced by one complete layer over the medoids of 64 k-means
  clusters: 60 distinct on glove, 63 on sift);
  `control_random` (`full` with every planned edge's target replaced by a seeded random vertex:
  same sources, same per-vertex counts, same capacity, same bytes, same entry layer);
  `entry_only` (medoid entry layer, no edges). All edge arms use +8 list slots (capacity 40).
  The build fills all 32 slots: a 50k-row glove pilot with no extra slots dropped all 20,260
  planned failure edges because every candidate source list was full.
- **Sweep**: ef 10,16,20,24,32,48,64,96,128,192,256 (the tested range); 3 repetitions, one pass
  over the 5,000 evaluation queries per (arm, ef, repetition), arm order rotated. QPS on this
  loaded shared 10-core machine (load average 15-18) is indicative only; recall and mean
  evaluations per query are load independent. "Matched cost" interpolates base recall linearly
  in log(cost) between bracketing measured base points (points outside the base range skipped).

## Results (full scale)

Two full runs per set (`*-run1.json`, `*-run2.json`, commands in `run1.sh`/`run2.sh`) built
identical graphs (same base and per-arm structure hashes) and identical recall at every point;
only QPS differs. Run 2 adds the in-sample check. Tables use run 1 except where noted.

Recall@10 at selected ef (all 11 points are in the reports and `summary-*.txt`):

| set | ef | base | failure | failure_antihub | full | control_random | entry_only |
|---|---:|---:|---:|---:|---:|---:|---:|
| glove | 10 | 0.5200 | 0.5202 | 0.5238 | 0.5203 | 0.5174 | 0.5164 |
| glove | 32 | 0.7144 | 0.7150 | 0.7195 | 0.7202 | 0.7158 | 0.7153 |
| glove | 96 | 0.8370 | 0.8370 | 0.8414 | 0.8408 | 0.8376 | 0.8377 |
| glove | 256 | 0.9073 | 0.9074 | 0.9113 | 0.9112 | 0.9072 | 0.9071 |
| sift | 10 | 0.7646 | 0.7647 | 0.7658 | 0.7664 | 0.7655 | 0.7655 |
| sift | 32 | 0.9352 | 0.9351 | 0.9352 | 0.9346 | 0.9340 | 0.9340 |
| sift | 96 | 0.9891 | 0.9890 | 0.9894 | 0.9891 | 0.9888 | 0.9888 |
| sift | 256 | 0.9987 | 0.9987 | 0.9988 | 0.9988 | 0.9987 | 0.9987 |

Recall minus base recall at matched mean evaluations per query (range over the sweep), and
median paired QPS ratio versus base over all (ef, repetition) pairs:

| arm | glove at matched evals | glove QPS ratio | sift at matched evals | sift QPS ratio |
|---|---|---:|---|---:|
| failure | -0.0006 .. +0.0001 | 0.994 | -0.0000 .. +0.0001 | 0.979 |
| failure_antihub | -0.0068 .. +0.0007 | 0.970 | -0.0019 .. +0.0000 | 0.987 |
| full | -0.0372 .. -0.0004 | 0.986 | -0.0540 .. -0.0002 | 0.940 |
| control_random | -0.0404 .. -0.0046 | 0.966 | -0.0554 .. -0.0003 | 0.952 |
| entry_only | -0.0298 .. -0.0012 | 1.010 | -0.0517 .. -0.0002 | 0.968 |

Cost: base build 135.8 s (glove) and 81.8 s (sift) under load; exact truth of the 5,000 train
queries 39.6 s / 43.8 s; the repair itself 0.2-0.4 s. Index bytes: glove 412,301,856 base,
440,706,192 with +8 slots (+6.9%), 435,548,000 with the medoid layer replacing the hierarchy
(+5.6%); sift 367,870,448 / 391,870,448 (+6.5%) / 388,016,388 (+5.5%). Edges added: glove
14,233 failure edges (3,174 of 5,000 train queries failed at exact ef 32, train recall 0.7153)
and 569,118 antihub edges into 133,100 vertices; sift 3,321 failure edges (1,773 failing,
train recall 0.9336) and 322,635 antihub edges into 81,941 vertices.

**In-sample versus held-out** (run 2, `train_recall_at_repair_ef_served`: the served index
searched with the 5,000 construction queries at ef 32):

| set | base | failure | failure_antihub | full | control_random | entry_only |
|---|---:|---:|---:|---:|---:|---:|
| glove train queries | 0.7158 | 0.9041 | 0.9015 | 0.8602 | 0.7133 | 0.7137 |
| sift train queries | 0.9347 | 0.9898 | 0.9897 | 0.9853 | 0.9349 | 0.9349 |

The same arms change held-out recall at ef 32 by +0.0006 (glove) and -0.0001 (sift).

**Denser training, glove 200k prefix** (`glove-200k-dense.json`): 32,768 construction queries
(the 5,000 split rows plus base rows 200,000..227,767), 77,355 failure edges and 48,521 antihub
edges on 200,000 vertices; base build 21.9 s, train truth 36.5 s; bytes 69,673,048 base,
74,473,048 with +8 slots (+6.9%). In-sample recall at ef 32: base 0.7601, failure 0.9268.
Held-out, fixed ef: failure +0.0024..+0.0075, failure_antihub +0.0040..+0.0083, full
+0.0005..+0.0060, control_random -0.0102..+0.0005. At matched evaluations: failure
-0.0049..+0.0002, failure_antihub -0.0070..+0.0009, full -0.0490..-0.0003, control_random
-0.0620..-0.0060, entry_only -0.0445..-0.0012. Median paired QPS ratio versus base: 0.966,
0.961, 0.956, 0.944, 0.993.

## Reading

- The failure edges fix the queries they were built from (glove in-sample 0.716 -> 0.904 at
  ef 32) but do not generalize: held-out recall moves by at most +0.0006 at full scale, and
  with 6.5x the queries on a 5.9x smaller base (200k prefix, 39x the queries per vertex) by +0.0024..+0.0075 at fixed ef, which is
  paid for in evaluations (at most +0.0002 at matched evaluations).
- The antihub in-edges raise fixed-ef recall (glove +0.0033..+0.0060, sift up to +0.0012), and
  their targets beat random ones (`full` over `control_random`: glove +0.003..+0.005 at equal
  evaluations), but every gain is paid for in evaluations: at matched evaluations they are at
  best even with the base graph. They also cost +6.5-6.9% index bytes.
- k-means medoid entry points cost about 2 x 64 scored vertices per query and lose at small ef
  on both sets; the HNSW-style upper layers are the better entry.
- Matched-QPS deltas (in the summaries) cross zero for every arm and swing by up to +-0.1
  between the two identical full runs: on this machine they are load noise, so the
  load-independent evaluation count decides.

Verdict under the preregistered rule: **park** (no arm beats base at matched cost on either
set at either scale).

Full-scale rerun on dedicated cores (not launched here):
`uv run --with modal modal run --detach benchmarks/public/ann_modal_repair.py --rev <commit> --cap-usd 8`
after the two `modal volume put` lines in that script's docstring. Not tested: degree-preserving edge replacement, more than 5,000 train queries at full
scale (the 200k run below has 32,768), other degrees/build_ef, the varint id layout.
