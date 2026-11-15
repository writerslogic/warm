# Build profiles

`BuildParams::profile` selects the insertion passes of a vector index build. The default,
`standard`, runs two passes and bounds each construction search (see below). `max-recall` is the
unbounded schedule of releases up to 0.6.8 and builds a byte-identical graph (SIFT-1M graph hash
`e0e8972a8c1d1c4d`, index file sha256 `2320c858...3874`, unchanged in all five repeats; a unit test
pins it). `public-bench ann-qgraph` takes `--build-profile`.

| Profile | Passes (`build_ef` 500) | Intent |
|---|---|---|
| `standard` | ef 100, then ef 500, both bounded | default: about 0.3x the `max-recall` build time |
| `max-recall` | ef 200, then ef 500, unbounded | highest recall, slowest build (earlier default) |
| `high` | ef 100, then ef 300 | about 0.58x the `max-recall` build time |
| `balanced` | ef 100, then ef 200 | about 0.45x |
| `fast` | one pass at ef 100 | about 0.20x |

Passes use `min(build_ef, ef)`. The profile is not recorded in a graph cache header: build with a
fresh cache path when it changes. A saved index record always names the profile; a record without
one is an earlier release's `max-recall` build and does not match a `standard` request.

## The bounded construction search

A construction search keeps a pool of `ef` candidates and expands every one of them. The bounded
search stops expanding once the best unexpanded candidate lies farther than 1.02 times the
squared distance of the 64th best candidate in the pool (`BOUND_RANK`, `BOUND_SLACK` in
`src/core/qgraph/mod.rs`). The pool is still `ef` long, so the lists and the prune see the same
kind of candidate set; the tail beyond rank 64 is no longer searched for. On glove-100 the
distances concentrate, so the unbounded search expanded most of a 500-entry pool.

Selection used a 5,000-row train holdout (never the benchmark queries), `ann-qgraph` at 8 threads,
estimates per query at recall 0.99 as the machine-independent quality measure.

## Default profile measurements

Heavy ANN suite, pinned core, foreign-CPU gate, GCP n2 Ice Lake 8 vCPU. WARM `standard` rerun
(SIFT: 5 build repeats, 10 rounds; GloVe: 3 build repeats, 5 rounds) against the comparator rows of
the earlier run on the same VM type; WARM `max-recall` in the same files was the previous default.
Graph hashes are identical at 1, 2, 4 and 8 build threads (SIFT `a83e53a9a22731bd`, GloVe
`51ef29295b67cf7d`, measured with the exploratory binary at the same parameters).

| dataset | system | build s | QPS@0.90 | QPS@0.95 | QPS@0.99 |
|---|---|---|---|---|---|
| sift-128 | WARM standard | 59.2 | 21,032 | 12,732 | 5,653 |
| sift-128 | WARM max-recall (previous default) | 184.7 | 22,424 | 16,856 | 8,262 |
| sift-128 | hnswlib | 257.0 | 10,780 | 7,223 | 3,389 |
| sift-128 | FAISS IVF-PQ fast-scan | 152.4 | 7,467 | 4,701 | 1,865 |
| sift-128 | USearch | 130.5 | 4,549 | 2,751 | 1,511 |
| glove-100 | WARM standard | 116.7 | 3,154 | 1,730 | 466 |
| glove-100 | WARM max-recall (previous default) | 360.2 | 4,328 | 2,372 | 667 |
| glove-100 | FAISS IVF-PQ fast-scan | 126.8 | 2,016 | 1,402 | not reached |
| glove-100 | DiskANN | 607.6 | 1,543 | 856 | 318 |
| glove-100 | hnswlib | 170.3 | 1,610 | 859 | 238 |

The default costs 17-31% of QPS at recall 0.95 and 0.99 against the previous default and stays
first at every recall level on both datasets with non-overlapping intervals. ScaNN builds in 31-35 s
(IVF with quantisation) and is faster to build than any graph index here; it does not reach recall
0.99 on GloVe. GloVe index size is unchanged at 913 MiB (the exact f32 rows are 461 MiB), above
FAISS IVF-PQ fast-scan at 490 MiB.

### Attempts to recover QPS at the same build time

Held-out GloVe (5,000 train rows as queries, 8 build threads, Ice Lake VM; estimates per query,
lower is better; `NR` = recall not reached within the ef range of that run). Experimental
environment switches only; none is in the library.

| construction | build s | est/q @0.95 | est/q @0.99 |
|---|---|---|---|
| `standard` (pass 1 ef 100 bounded, pass 2 ef 500 bounded at rank 64, slack 1.02) | 112.9 | 11,905 | 42,315 |
| `max-recall` | 337.5 | NR | 28,293 |
| standard with pass 2 unbounded | 308.2 | 9,842 | 30,479 |
| standard with pass 2 bound at rank 128, slack 1.05 | 216.6 | 9,785 | 31,712 |
| standard with pass 1 bound at rank 48, slack 1.01 | 106.2 | 12,942 | 47,662 |
| standard with pass 1 bound at rank 32, slack 1.0 | 89.7 | 14,381 | NR |
| standard with a first pass at ef 200 | 120.1 | 12,006 | NR |
| standard plus an unbounded ef 500 repair of the vertices with in-degree at most 0 / 2 / 5 (1 / 372 / 4,669 vertices) | 114.9 / 115.3 / 116.3 | 11,905 / 11,897 / 11,913 | NR |

Recall at 0.99 is paid for by the bound of the second pass, and the price is linear in build
time: every point that is cheaper than `standard` is worse at 0.95, and every point that is
better at 0.99 costs 100 s or more. The repair of poorly connected vertices changes nothing, so
the loss is not unreachable vertices. QPS within 3% of `max-recall` and a build 10% faster than
FAISS IVF-PQ (114 s) do not hold together on GloVe. The reverse-edge re-prune with the
`max-recall` rule was not tried: it only reorders candidates that the bound already dropped.

### Index size

GloVe 913 MiB is the vertex file (452 MiB) plus the exact f32 rows (461 MiB). A clean index
serves from the vertex file alone (the rows are read only to rescore after updates or deletes), so
changing the rows changes size but not QPS or the reachable recall of a clean index.

| change | GloVe MiB | measured |
|---|---|---|
| delta-coded neighbour ids (`id_bytes` 0) | vertex 450 -> 393 | same graph hash; single-thread QPS at 0.99 on SIFT 7,513 -> 6,053 (-19%), GloVe at 0.95 about -30% |
| f16 rows | rows 461 -> 230 | top-10 of exact search over 200,000 rows changes in 0.05% of slots (overlap 0.9995) |
| byte-plane transposition plus zlib level 1 on the f32 rows | rows 461 -> 397 | ratio 0.861 (lzma 0.854) |

The best combination (delta ids plus f16 rows) is still well above FAISS IVF-PQ's 490 MiB, and the
delta ids cost 19-30% of QPS. None of these is shipped. Below 490 MiB the exact rows would have to
be dropped or rebuilt from the vertex residual codes, which makes returned scores and rebuilds
approximate; that is a format and semantics decision, not made here.

Results that were produced with `max-recall` as the default and are not regenerated:
`benchmarks/results/qgraph_build_profiles_gcp_v1.json`, the `qgraph_*` results, the
`heavy-ann-sift-128-euclidean-...-intel-ultra-9-185h-22c-27g.json` file, and every tabulated number
below.

All numbers below are from a GCP n2-standard-8 (Ice Lake, 8 vCPU = 4 physical cores), 8 build
threads, nothing else running, builds interleaved, serving on one pinned core with its hyper-thread
sibling idle, 10 timed rounds. SIFT-1M has 5 build repeats; glove-100-angular has 1 (time cap), so its
build-time intervals are single points. The project's timing host re-measures. Base: the
`perf/build-time-and-memory` branch on main `120ab9ff`; `bench/fair-harness` and
`integrate/publication-fixes` are not merged, so recall points are read on the 10,000 test queries
without a selection/report split. hnswlib 0.8.0 is served through the native C++ driver. Raw data:
`benchmarks/results/qgraph_build_profiles_gcp_v1.json`; the tables are generated by
`benchmarks/competitive/build_pareto.py`.

Rules fixed before the numbers: a profile is "no slower to build" when its median build seconds is at
most the hnswlib median; QPS at a recall is read off the upper envelope of the sweep, log-linear
between bracketing points, with the interval taken over the 10 rounds; a cell dominates when the WARM
lower bound exceeds the hnswlib upper bound. The "fastest" column is the literal reading (it is the
`fast` profile in every row); "best_serving" is the highest-QPS profile that is no slower to build.

## Results

### sift

Build seconds (median, min-max over repeats), peak build RSS, index bytes, serving RSS, QPS at recall (median, min-max over 10 rounds, one pinned core).

| system | build s | build RSS MiB | index MiB | serve RSS MiB | QPS@0.90 | QPS@0.95 | QPS@0.99 |
|---|---|---|---|---|---|---|---|
| WARM fast | 35.6 (35.4-37.5) | 1,294 | 446 | 482 | 24,407 (24,115-24,999) | 16,051 (15,758-16,231) | 7,131 (6,937-7,253) |
| WARM balanced | 78.1 (77.6-79.4) | 1,295 | 446 | 482 | 25,662 (25,055-25,785) | 17,821 (17,341-17,961) | 8,337 (8,107-8,460) |
| WARM high | 101.0 (99.5-102.1) | 1,298 | 446 | 482 | 26,120 (25,623-26,458) | 18,385 (18,026-18,578) | 8,697 (8,531-8,773) |
| WARM max-recall | 175.0 (174.2-177.2) | 1,292 | 446 | 482 | 26,751 (25,597-26,974) | 18,894 (18,430-19,262) | 9,124 (8,773-9,338) |
| hnswlib M=16 efC=200 | 83.4 (83.0-85.5) | 1,270 | 630 | 732 | 9,139 (8,897-9,551) | 6,017 (5,912-6,285) | 2,770 (2,702-2,829) |
| hnswlib M=32 efC=200 | 106.5 (104.5-106.6) | 1,392 | 752 | 853 | 9,063 (8,691-9,200) | 6,292 (6,044-6,420) | 2,952 (2,884-2,997) |
| hnswlib M=32 efC=500 | 257.9 (253.8-272.3) | 1,392 | 752 | 853 | 8,879 (8,624-9,012) | 6,389 (6,215-6,447) | 3,028 (2,946-3,090) |
| hnswlib M=48 efC=500 | 283.5 (278.8-304.7) | 1,513 | 874 | 975 | 8,302 (8,001-8,462) | 5,950 (5,885-6,031) | 3,013 (2,945-3,060) |

For each hnswlib build: the WARM profiles whose median build is no slower, the fastest of them, and the best-serving of them, as WARM/hnswlib QPS ratio (lower-upper bound over rounds).

| hnswlib build | no-slower WARM profiles | column | profile | QPS@0.90 | QPS@0.95 | QPS@0.99 |
|---|---|---|---|---|---|---|
| M=16 efC=200 | fast, balanced | fastest | fast | 2.67x (2.52-2.81) dominates | 2.67x (2.51-2.75) dominates | 2.57x (2.45-2.68) dominates |
| M=16 efC=200 | fast, balanced | best_serving | balanced | 2.81x (2.62-2.90) dominates | 2.96x (2.76-3.04) dominates | 3.01x (2.87-3.13) dominates |
| M=32 efC=200 | fast, balanced, high | fastest | fast | 2.69x (2.62-2.88) dominates | 2.55x (2.45-2.69) dominates | 2.42x (2.31-2.52) dominates |
| M=32 efC=200 | fast, balanced, high | best_serving | high | 2.88x (2.79-3.04) dominates | 2.92x (2.81-3.07) dominates | 2.95x (2.85-3.04) dominates |
| M=32 efC=500 | fast, balanced, high, max-recall | fastest | fast | 2.75x (2.68-2.90) dominates | 2.51x (2.44-2.61) dominates | 2.35x (2.25-2.46) dominates |
| M=32 efC=500 | fast, balanced, high, max-recall | best_serving | max-recall | 3.01x (2.84-3.13) dominates | 2.96x (2.86-3.10) dominates | 3.01x (2.84-3.17) dominates |
| M=48 efC=500 | fast, balanced, high, max-recall | fastest | fast | 2.94x (2.85-3.12) dominates | 2.70x (2.61-2.76) dominates | 2.37x (2.27-2.46) dominates |
| M=48 efC=500 | fast, balanced, high, max-recall | best_serving | max-recall | 3.22x (3.02-3.37) dominates | 3.18x (3.06-3.27) dominates | 3.03x (2.87-3.17) dominates |

### glove

Build seconds (median, min-max over repeats), peak build RSS, index bytes, serving RSS, QPS at recall (median, min-max over 10 rounds, one pinned core).

| system | build s | build RSS MiB | index MiB | serve RSS MiB | QPS@0.90 | QPS@0.95 | QPS@0.99 |
|---|---|---|---|---|---|---|---|
| WARM fast | 67.4 (67.4-67.4) | 1,315 | 452 | 487 | 3,829 (3,714-3,904) | 1,596 (1,537-1,621) | not reached |
| WARM balanced | 166.2 (166.2-166.2) | 1,370 | 452 | 488 | 5,108 (4,900-5,149) | 2,384 (2,277-2,406) | not reached |
| WARM high | 221.1 (221.1-221.1) | 1,374 | 452 | 488 | 5,392 (5,289-5,440) | 2,554 (2,533-2,587) | 717 (700-723) |
| WARM max-recall | 396.7 (396.7-396.7) | 1,368 | 452 | 488 | 5,491 (5,293-5,544) | 2,689 (2,610-2,721) | 787 (774-791) |
| hnswlib M=16 efC=200 | 122.3 (122.3-122.3) | 1,243 | 619 | 734 | 1,297 (1,281-1,318) | 551 (510-561) | not reached |
| hnswlib M=32 efC=200 | 197.1 (197.1-197.1) | 1,385 | 763 | 878 | 1,690 (1,643-1,738) | 880 (868-893) | not reached |
| hnswlib M=32 efC=500 | 488.4 (488.4-488.4) | 1,385 | 763 | 878 | 1,690 (1,647-1,758) | 803 (784-814) | 244 (240-246) |
| hnswlib M=48 efC=500 | 664.5 (664.5-664.5) | 1,530 | 908 | 1,022 | 1,712 (1,676-1,734) | 855 (843-881) | 281 (277-283) |

For each hnswlib build: the WARM profiles whose median build is no slower, the fastest of them, and the best-serving of them, as WARM/hnswlib QPS ratio (lower-upper bound over rounds).

| hnswlib build | no-slower WARM profiles | column | profile | QPS@0.90 | QPS@0.95 | QPS@0.99 |
|---|---|---|---|---|---|---|
| M=16 efC=200 | fast | fastest | fast | 2.95x (2.82-3.05) dominates | 2.90x (2.74-3.18) dominates | tie: neither reaches the recall |
| M=16 efC=200 | fast | best_serving | fast | 2.95x (2.82-3.05) dominates | 2.90x (2.74-3.18) dominates | tie: neither reaches the recall |
| M=32 efC=200 | fast, balanced | fastest | fast | 2.27x (2.14-2.38) dominates | 1.81x (1.72-1.87) dominates | tie: neither reaches the recall |
| M=32 efC=200 | fast, balanced | best_serving | balanced | 3.02x (2.82-3.13) dominates | 2.71x (2.55-2.77) dominates | tie: neither reaches the recall |
| M=32 efC=500 | fast, balanced, high, max-recall | fastest | fast | 2.27x (2.11-2.37) dominates | 1.99x (1.89-2.07) dominates | WARM does not reach the recall |
| [M=32 efC=500](../../benchmarks/results/qgraph_build_profiles_gcp_v1.json) | fast, balanced, high, max-recall | best_serving | max-recall | 3.25x (3.01-3.37) dominates | 3.35x (3.21-3.47) dominates | 3.23x (3.15-3.29) dominates |
| M=48 efC=500 | fast, balanced, high, max-recall | fastest | fast | 2.24x (2.14-2.33) dominates | 1.87x (1.74-1.92) dominates | WARM does not reach the recall |
| [M=48 efC=500](../../benchmarks/results/qgraph_build_profiles_gcp_v1.json) | fast, balanced, high, max-recall | best_serving | max-recall | 3.21x (3.05-3.31) dominates | 3.15x (2.96-3.23) dominates | 2.80x (2.74-2.85) dominates |

## Where WARM does not dominate

- The default profile still builds 2.1x slower than hnswlib M=16 (175 s against 83 s on SIFT-1M). The
  objection is answered by choosing a profile, not by the default.
- glove-100: `balanced` (166 s) is slower than hnswlib M=16 (122 s), so only `fast` competes with that
  build, and `fast` and `balanced` do not reach recall 0.99 within the swept `ef` (up to 1024). The two
  hnswlib builds they are compared with (M=16 and M=32 efC=200) do not reach it either, so those cells
  are ties, not wins. `high` reaches 0.99 at 717 QPS ([data](../../benchmarks/results/qgraph_build_profiles_gcp_v1.json)) but builds in 221 s, slower than hnswlib M=32
  efC=200 (197 s).
- glove-100 has one build repeat per configuration.
- Serving is single-thread QPS on 10,000 queries; one dataset per metric family.

## Thread scaling (SIFT 250k subset, default and profile schedules)

Graphs are identical at 1, 2, 4 and 8 threads (hash `c299a33dd6161106` for the default). Build seconds:

| Profile | 1 thread | 2 | 4 | 8 |
|---|---|---|---|---|
| max-recall | 188.0 | 97.0 | 46.5 | 34.3 |
| balanced | 88.9 | 43.8 | 21.8 | 16.3 |
| fast | 38.4 | 19.5 | 9.7 | 7.6 |

Scaling is linear to 4 threads (each 4-thread time is a quarter of the 1-thread time to within 2%). From 4 to 8 threads it is 1.27-1.36x in every phase (both
passes, fill and upper layers), because the 8 vCPUs are 4 cores with two hyper-threads each. No phase
is serial or contended: the reverse-edge merge is about 1 s of a 35 s build. Thread scaling is not the
remaining lever on this VM; the passes are memory-latency bound.

## Schedules tried and rejected (SIFT-1M minus a 10,000-vector train holdout, 8 threads, unpinned)

Build seconds; QPS at recall 0.95 and 0.99 from a 3-repeat unpinned sweep, so within about 10 percent.

| Schedule | Build s | QPS 0.95 | QPS 0.99 | Verdict |
|---|---|---|---|---|
| default (200, 500) | 170 | 25,311 | 11,615 | reference |
| 100, 500 | 150 | 23,947 | 11,113 | between `high` and the default; not named |
| 100, 300 | 99 | 22,291 | 10,434 | kept as `high` |
| 100, 200 | 78 | 22,475 | 9,938 | kept as `balanced` |
| 200, 200 | 102 | 24,114 | 10,676 | no better than 100, 300 at the same time |
| one pass ef 200 | 59 | 21,554 | 9,732 | on the front, not named |
| one pass ef 100 | 35 | 20,371 | 8,231 | kept as `fast` |
| one pass ef 64 | 25 | 18,366 | 7,262 | worse per second than ef 100 |
| 50, 100 | 43 | 19,990 | 8,292 | dominated by one pass ef 100 plus time |
| one pass ef 100, alpha 1.2 | 78 | 22,253 | 8,646 | 2.2x slower, no recall gain |
| one pass ef 200, alpha 1.2 | 108 | 20,364 | 8,519 | rejected |
| one pass ef 300, alpha 1.2 | 126 | 22,710 | 9,286 | rejected |
| (200, 500) second pass on 40% of points | 102 | 24,112 | 10,488 | same as 200, 200 |
| (100, 200) second pass on 50% of points | 54 | 22,111 | 9,663 | same as one pass ef 200 |

Earlier on this branch: huge pages on the build arrays (190.4 s against 189.8 s) were rejected, and
the locality batch order gave 3 percent.
