# ann-benchmarks harness, single core

These results come from running WARM and its competitors through the [ann-benchmarks](https://github.com/erikbern/ann-benchmarks) runner (commit `2e081ad`) on an Intel Core Ultra 9 185H under WSL2 with AVX2 and no AVX-512. Each run is pinned to one CPU, for the index build and for every query. WARM is commit `19f4371`, using the wrapper in [`benchmarks/ann-benchmarks/warm`](../../benchmarks/ann-benchmarks/warm). The results file [`ann_benchmarks_harness_wl02_v1.json`](../../benchmarks/results/ann_benchmarks_harness_wl02_v1.json) has every point, each competitor's version, and each deviation from upstream's docker runner. The main deviations are:

- `--local` with `taskset` replaces docker;
- `--runs 2`;
- up to 10 runs in parallel;
- subsets of the competitors' upstream run groups.

The table gives the best QPS at recall@10 of at least 0.90, 0.95 and 0.99, for WARM and for the fastest competitor that was run. The last column is the ratio of the two. Noise is large on this host: two WARM settings that did identical work in the same run differed by up to 1.37x. Treat ratios within 1.37x as ties. Variation between runs made at different times was not measured, so a gap a little beyond that is a probable loss rather than a firm one.

| Dataset | Recall | WARM QPS | Fastest competitor | QPS | WARM / competitor |
|---|---|---|---|---|---|
| glove-100-angular | 0.90 | 1,457 | scann | 2,985 | 0.49 |
| glove-100-angular | 0.95 | 1,019 | scann | 1,535 | 0.66 |
| glove-100-angular | 0.99 | 511 | scann | 592 | 0.86 |
| sift-128-euclidean | 0.90 | 4,649 | faiss-ivfpqfs | 5,276 | 0.88 |
| sift-128-euclidean | 0.95 | 2,819 | faiss-ivfpqfs | 3,787 | 0.74 |
| sift-128-euclidean | 0.99 | 1,267 | faiss-ivfpqfs | 1,895 | 0.67 |
| fashion-mnist-784-euclidean | 0.90 | 9,454 | glass | 14,180 | 0.67 |
| fashion-mnist-784-euclidean | 0.95 | 9,454 | glass | 14,180 | 0.67 |
| fashion-mnist-784-euclidean | 0.99 | 7,316 | glass | 7,809 | 0.94 |

WARM is not the fastest at any of these points. Four are ties within the noise: GloVe 0.99, SIFT 0.90, SIFT 0.95 and Fashion-MNIST 0.99. GloVe 0.95, SIFT 0.99 and Fashion-MNIST 0.90/0.95 trail by about 1.5x, which is probable rather than firm. GloVe 0.90 is a clear loss at 2.0x.

On GloVe, recall grows slowly with the pool size. Degree 64 needs ef 120 for 0.93 and ef 640 for 0.99, so traversal cost dominates there. Re-ranking the whole pool barely changes recall at a given ef.

Competitors that were not run include NGT/ONNG/QSG-NGT and DiskANN/Vamana. The host has no AVX-512, so glass, faiss and WARM all run their AVX2 paths; published AVX-512 rankings do not carry over. hnswlib's GloVe points are far below its published position relative to scann and are not validated.

![glove-100-angular](ann-benchmarks/glove-100-angular.png)
![sift-128-euclidean](ann-benchmarks/sift-128-euclidean.png)
![fashion-mnist-784-euclidean](ann-benchmarks/fashion-mnist-784-euclidean.png)

## Single-thread search after `perf/single-thread-search` (v2)

[`ann_benchmarks_single_thread_wl02_v2.json`](../../benchmarks/results/ann_benchmarks_single_thread_wl02_v2.json) pairs the code of the v1 standing (`791749b`, 4-bit traversal codes) against this change (`d1aa609`) on the same host and the same indexes: build_ef 500, 8-bit residual, one pinned CPU, 10,000 test queries per configuration, three interleaved rounds of [before, after 4-bit, after 8-bit] under the host timing lock, median QPS. The after 4-bit arm returns the same ids as before for every configuration (result hashes in the file); the after 8-bit arm uses 8-bit traversal codes, now the wrapper's `code_bits` default. Every number below is from a local driver over the Rust search, not from the Python harness, so it is not comparable with the v1 table above; the harness cross-check was not measured (below). Start, end and load of each set are in `datasets.*.window`; the host carried jobs outside the lock, so differences within the round spread (`qps_rounds`) are ties.

| Dataset | Degree | Recall | before (4-bit) | after (4-bit, same results) | after (8-bit codes) | after 8-bit / before |
|---|---|---|---|---|---|---|
| fashion-mnist-784-euclidean | 32 | 0.90 | 26,460 | 35,157 | 39,453 | 1.49 |
| fashion-mnist-784-euclidean | 32 | 0.95 | 26,460 | 35,157 | 39,453 | 1.49 |
| fashion-mnist-784-euclidean | 32 | 0.99 | 17,573 | 23,697 | 27,339 | 1.56 |
| sift-128-euclidean | 32 | 0.90 | 12,371 | 16,824 | 26,618 | 2.15 |
| sift-128-euclidean | 32 | 0.95 | 8,411 | 11,812 | 21,734 | 2.58 |
| sift-128-euclidean | 32 | 0.99 | 3,047 | 4,364 | 9,269 | 3.04 |
| glove-100-angular | 32 | 0.90 | 3,758 | 4,846 | 4,958 | 1.32 |
| glove-100-angular | 32 | 0.95 | not reached | not reached | 2,076 | - |
| glove-100-angular | 32 | 0.99 | not reached | not reached | not reached | - |
| glove-100-angular | 64 | 0.90 | 4,167 | 5,407 | 5,621 | 1.35 |
| glove-100-angular | 64 | 0.95 | 2,451 | 3,153 | 2,968 | 1.21 |
| glove-100-angular | 64 | 0.99 | 670 | 829 | 821 | 1.23 |

- fashion-mnist-784-euclidean (degree 32): same-configuration after/before on the 4-bit index 1.13 to 1.50; max recall@10 in the sweep 0.9992 with 4-bit codes, 0.9999 with 8-bit.
- sift-128-euclidean (degree 32): same-configuration after/before on the 4-bit index 1.27 to 1.45; max recall@10 in the sweep 0.9963 with 4-bit codes, 0.9982 with 8-bit.
- glove-100-angular (degree 32): same-configuration after/before on the 4-bit index 1.16 to 1.52; max recall@10 in the sweep 0.9485 with 4-bit codes, 0.9503 with 8-bit.
- glove-100-angular (degree 64): same-configuration after/before on the 4-bit index 1.18 to 1.43; max recall@10 in the sweep 0.9917 with 4-bit codes, 0.9931 with 8-bit.

Losses and limits: on GloVe degree 32 neither code width reaches 0.99 in the swept pool sizes and 4-bit codes do not reach 0.95; degree 64 reaches all three. GloVe was not rerun in the harness, so its standing against scann is the v1 table's. On GloVe the 8-bit codes buy almost no recall at a given ef (0.9174 against 0.9137 at ef 120, degree 64), so the wider codes cost more than they buy there: at degree 64 the after 8-bit arm runs at 1.04x, 0.94x, 0.99x of after 4-bit at 0.90, 0.95 and 0.99. Below 1.00 is a loss of the new default against 4-bit codes; the gain on GloVe is the prefetch and re-rank work.

The Fashion-MNIST set started while jobs outside the lock held the host at 1-minute load 8.5, which fell to 1.2 during it: its first round ran up to 2.6x slower than the other two, so its medians are the slower of the two clean rounds and its same-configuration ratios are less certain than SIFT's and GloVe's.

Harness cross-check: not measured. Command (one pinned CPU under the timing lock, wheel built at `d1aa609`, wrapper `code_bits` 8): `taskset -c 4 python run.py --local --dataset fashion-mnist-784-euclidean --definitions <F-warm> --runs 2 --algorithm warm --force`, the same for `sift-128-euclidean` with the degree-32 definitions, and the v1-winning competitor families (glass R 16 level 2, faiss-ivfpqfs n_list 512 and 1204) in the same session; then `data_export.py`.

What changed, from the profile in the results file (`profile_before`): each row estimate waited on the scale (and, for Euclidean, the norm term) of the row, two random 4-byte reads that were not prefetched with the code row; the re-rank kernels were scalar; the entry descent read upper-layer rows one at a time. Prefetching those, and AVX2/NEON re-rank kernels that keep the scalar arithmetic, leave every result unchanged. On SIFT the 4-bit max-abs code is what forces the large pool: with 8-bit codes the same graph reaches a given recall at a smaller ef.

## Historical partial campaign (v3, wl-02, 2026-11-08)

Status: this is a partial development record. The committed artifacts contain two SIFT control rounds and profiler reports. Other arms were planned but have no completed result in this artifact set; the old queue description is not a claim that jobs remain running. Harness controls, profiler attribution and the reported IVF smoke test have separate measurement boundaries.

### What the entry is

The ann-benchmarks entry is the quantised vertex graph (`VectorIndex`, `qgraph`), not an IVF index. The wrapper builds one graph (degree, `build_ef`, traversal `code_bits`) and sweeps `ef` and `rerank`. The README row "sift 2,819 QPS, glove 1,019 QPS" is the v1 standing: commit `19f4371`, 4-bit traversal codes, up to ten parallel runs on WSL2 logical CPUs, one job swapping. The wrapper default at the time was 8-bit codes (`d1aa6097`), and `ann_benchmarks_single_thread_wl02_v2.json` measured 2.1x to 3.0x on SIFT from that change with a local driver, never through the harness. These older README measurements describe their recorded revision, not the throughput of a later release.

### Control through the harness (measured)

Historical control wheel: `f21de886` (rustc 1.99.1), wrapper `code_bits` 8, degree 32, `build_ef` 500, `rerank` 64, ef grid 10 to 800. Harness `2e081ad`, `taskset -c 4`, one thread, `--runs 2`, every run under the lock, 10,000 official test queries, two rounds. Host load was 6 to 18 from other jobs.

| Dataset | Recall | Round 1 QPS | Round 2 QPS | Median | v1 README standing |
|---|---|---|---|---|---|
| sift-128-euclidean | 0.95 | 17,908 | 17,423 | 17,666 | 2,819 |
| sift-128-euclidean | 0.99 | 9,196 | 9,086 | 9,141 | 1,267 |

QPS is the best of the points with recall at least the target (the harness convention); linear-in-log-QPS interpolation gives 18,595 and 9,505. Files: `benchmarks/results/ann_benchmarks_v3_wl02/g0-sift-ctrl.json` and `g0-sift/`. No same-session faiss-ivfpqfs (SIFT) or ScaNN (GloVe) rerun is committed. The v1 faiss-ivfpqfs figures of 3,787 and 1,895 QPS were measured on another day under different contention and do not support a matched speedup ratio.

No full GloVe harness control is committed for this campaign. The v2 driver numbers (not comparable) were 2,968 QPS at 0.95 for degree 64 against ScaNN's 1,535 in v1.

### Profile of the control at its 0.95 point (measured)

`perf record -e cpu-clock -F 4000` of the harness query loop (`index.query` per test query), pinned to one CPU outside the lock (attribution only, not timing; host load about 17). Raw reports are in `benchmarks/results/ann_benchmarks_v3_wl02/profiles/`.

| Share of samples | sift (held-out, degree 32, ef 30, recall 0.953) | glove (degree 64, ef 240, recall 0.956) |
|---|---|---|
| `search_inner` (traversal, admission, memory stalls) | 51% | 56% |
| `dot_i8` (AVX-VNNI tier selected on this host) | 8.9% | 12.6% |
| `VGraph::est` | 8.7% | 9.3% |
| `Pool::insert` | 4.7% | 6.5% |
| `prefetch_neighbours` | 3.3% | 4.1% |
| `score_fresh` | 2.6% | 3.6% |
| `prefetch_rerank_rows` | 2.6% | 0.9% |
| exact/residual re-rank (`dot_f32_i8`) | 1.6% | 0.9% |
| Python and numpy | about 5% | under 1% |

The entry is traversal-bound. The re-rank occupies under 3% of these samples, limiting the sampled work it could eliminate, and the IVF scan's scalar compare loop (41% of that profile) does not exist in this index. A fused admission has no counterpart in the graph: the per-neighbour admission is `Pool::insert` (5% to 7%) plus the part of `search_inner` that waits on memory.

### Candidates and their status

| Candidate | Status |
|---|---|
| IVF tier (`PartitionIndex`: SOAR, 4-bit LUT16 scan with the fused admission, exact reorder) as a second entry | Historically reported as rejected on SIFT by a smoke run (held-out sift, 2048 lists, `pq_m` 64, reorder 200): recall 0.968 at 2,976 QPS and 0.992 at 1,292, against 18,182 QPS at recall 0.954 for the graph. No raw smoke result is committed in this PR; this report is not independently verified here. Not run on GloVe. |
| Compile-time variants already in tree: `experimental-batch-pool`, `-neighbor-admission`, `-query-x4` (wheels `xbp`, `xna`, `xq4`) | Reported built; no completed timing result is committed (`b1-sift`, `b2-glove` plans). |
| Searcher env knobs (`WARM_QGRAPH_PREFETCH`, `BATCH_EST`, `FRONTIER`, `RERANK_Z`, `KERNEL=avx2` against the auto-selected AVX-VNNI tier) | No completed timing result committed. |
| `rerank` depth, `patience` (query arguments) | No completed timing result committed. |
| Degree, `build_ef`, 4-bit codes, compact adjacency | Reported prebuilt; no completed timing result committed. Only `g1-sift` is committed; there is no `g1-glove` plan in this artifact set. |
| Evolutionary search over the survivors | No evolutionary result is committed. `campaign/ga.py` is experimental development tooling. |

### Held-out method

The proposed selection split draws 10,000 random train rows (NumPy seed 20261108), removes them from the index and writes the ann-benchmarks HDF5 layout (`<dataset>-heldout.hdf5`; 990,000 and 1,173,514 index rows). The historical `campaign/gen_heldout.py` ranks a full scan using float32 arithmetic, then recomputes distances for the selected 100 IDs. This is floating-point brute-force ground truth; exact membership and ordering were not independently certified, and no label error is established here. The SIFT control already inspected official test queries, and profiling inspected generated queries, so those observations are development evidence. Future confirmation needs fresh, uninspected queries and a declared generalisation band. A difference larger than timing spread alone does not establish overfitting, particularly with different indexed corpus sizes.

### Protocol and harness deviations

The historical runner took one lock hold per generation and rotated arm order per round. Deviations from upstream included an arm tag, a build cache based on sampled rows and a wheel tag, `--local` instead of Docker, and external registration of held-out dataset names in `DATASETS`. The recorded builds used four niced threads outside timing. Cached `fit` measurements are load work, not fresh build time. Competitor environments were described as faiss-cpu 1.15.1 and scann 1.4.2 from PyPI; no matched competitor result is committed here.

The corrected tooling requires successful runner/export exits and a valid CSV covering the registered configuration count before publishing a success receipt. Resume verifies that receipt against the plan, source/data/definition/wheel fingerprints and export hash; legacy CSVs alone do not qualify, and installed competitor environments without a pinned wheel are not resumed. Fitness requires all registered rounds and both recall floors in every round. Environment normalization uses each knob's own default. Cache identity now covers the full f32 fit matrix, build settings and loaded native artifacts; unique temporary files and atomic no-overwrite publication prevent partial or competing cache writes; a racing builder loads the published winner. These fixes do not remeasure or retroactively validate the historical results.

The committed plans reference host-specific wheels, definitions, Python environments and a separate harness checkout. Reconstruct and pin those prerequisites before a new, separately registered experiment. Each arm must declare a positive `expected_rows` count, or a dataset-to-count mapping, matching its complete query grid; the archived plans are not runnable unchanged. The evolutionary driver derives this count from its single-build ef grid. Importing the runner or evolutionary helper does not launch work. The offline regression file is `campaign/test_campaign_integrity.py` and uses synthetic process/file fixtures, not benchmark datasets or native index execution.

### Historical reproduction pins

At the time, `benchmarks/ann-benchmarks/warm` pinned `d1aa6097` with 8-bit codes. The historical control revision was `f21de8861b194326dcc641dc18483cc365ed3de6`; this is not a suggestion to label it current main. The proposed smaller ef grid was not measured and supplies no runtime or unchanged-best-point guarantee. No new configuration should be selected from the inspected official queries.
