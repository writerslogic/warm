# Low-level system assumptions in the L22 streaming path

Status: measured 2026-11-08. Branch `perf/lowlevel-systems`. Facts are tagged [M] measured or [I] inferred.

## Question

WARM, like most published ANN and memory systems, runs on defaults nobody re-tests: the system allocator, 4 KiB pages, one thread per hardware thread, the stock compiler profile, a Python harness that reads rows inside the timed step. This note tests each default against the L22 streaming run (msturing-30M-clustered `final_runbook`, `np600 / ro1000`) on the GCP Ice Lake VM (n2-custom-8-16384, Xeon Ice Lake-SP 4 cores x 2 threads, AVX-512, THP `madvise`), and keeps what measurably helps.

## Protocol

- Proxy P: the first 160 runbook steps (80 searches, up to 5.0 M rows inserted, live up to 4.6 M) replayed through `PartitionIndex` by `scripts/lowlevel/replay.py`. It uses the adapter's data reads and id types, per-operation timers, an FNV digest of every returned id, and the harness tie-aware recall@10 against the GT100 files. Page cache dropped before each run, machine idle (load < 0.4), 8 threads.
- Arms were registered in `STATE.md` before they ran. Rounds are interleaved (the arm order rotates). Control spread over two rounds is 0.6 s of 302 s wall (0.2 %) [M], so a difference above 1 % is outside the noise.
- Kill rule: an arm is dropped when its wall gain over the control is below 1 %, or its result digest or recall differs (a differing digest is reported exactly, never hidden).
- Confirmation: the full 1,280-step runbook through the harness (`gcp-il-s2e/chain_e.sh` pattern, `chain_y.sh`), 8 GiB scope, no swap.

## Where the time goes (full run, control c5cd1c61) [M]

| term | seconds |
|---|---|
| scan (slowest thread, includes tables 223) | 1,906 |
| insert (binding) | 271 |
| reorder | 151 |
| centres (query-to-centre GEMM) | 71 |
| route | 43 |
| delete | 37 |
| harness reads and conversions inside the steps | 45 |
| total step sum | 2,601 |

The harness overhead is 45 s on Ice Lake, not the 200 s seen on the Windows host in test B.

## Candidate verdicts (proxy P, Ice Lake)

| arm | what | n | wall s | vs control | insert s | search s | read s | peak RSS GiB | results |
|---|---|---|---|---|---|---|---|---|---|
| c0 | control (current wheel, THP off) | 2 | 302.1 | +0.0 % | 36.5 | 252.3 | 6.9 | 3.41 | identical |
| thp | WARM_THP=1, madvise(MADV_HUGEPAGE) on allocations >= 2 MiB | 1 | 300.0 | -0.7 % | 37.4 | 249.3 | 6.9 | 4.30 | identical |
| t4 | 4 threads | 1 | 345.1 | +14.2 % | 39.9 | 291.9 | 6.9 | 3.23 | identical |
| t6 | 6 threads | 1 | 326.6 | +8.1 % | 38.1 | 275.1 | 7.0 | 3.37 | identical |
| v4 | -C target-cpu=x86-64-v4 | 2 | 305.9 | +1.3 % | 37.3 | 255.3 | 6.9 | 3.38 | identical |
| pgo | PGO from 200 instrumented steps | 2 | 310.1 | +2.6 % | 38.1 | 258.3 | 7.0 | 3.41 | identical |
| pf | read-ahead of the next 4 insert ranges (adapter) | 2 | 295.0 | -2.4 % | 36.3 | 251.6 | 0.7 | 3.42 | identical |

Second control (the wheel the later arms use):

| arm | what | n | wall s | vs control | insert s | search s | read s | peak RSS GiB | results |
|---|---|---|---|---|---|---|---|---|---|
| c0b | control (wheel with allocator cfg, THP off) | 2 | 300.2 | +0.0 % | 36.3 | 250.4 | 6.9 | 3.40 | identical |
| place | slots renumbered in list order after training | 1 | 300.5 | +0.1 % | 36.6 | 250.5 | 6.9 | 3.39 | identical |
| mi | mimalloc as global allocator (dynamic TLS) | 1 | 303.3 | +1.0 % | 38.5 | 251.5 | 6.9 | 4.56 | identical |

Verdicts: huge pages KILLED (-0.7 %, n=1, +0.9 GiB RSS); 4 and 6 threads KILLED (+14 %, +8 %: all 8 hardware threads win even though the quad kernel saturates p0/p5); x86-64-v4 KILLED (+1.3 %); PGO KILLED (+2.6 %); mimalloc KILLED (+1.0 %, +1.2 GiB RSS; with the default initial-exec TLS the extension does not even load: `cannot allocate memory in static TLS block`); list-order placement KILLED (0.0 %, digest identical); read-ahead KEPT (-2.4 %, read time -90 %, digest identical, n=2). jemalloc was not built: mimalloc is the better-case allocator for the large-array pattern and showed no gain, and insert is GEMM-bound (profile below).

## Insert profile [M]

`perf` (cpu-clock, 499 Hz) on the first 60 steps: the matrixmultiply AVX-512 sgemm kernel and its packing are 42 % of samples, the rayon epilogue that scans the 1024 x 5000 score block for the nearest and spill centres (`nearest_all`, `assign`) 29 %, `search_partition` 8 %, quad scan 6 %. Insert is therefore GEMM plus a scalar epilogue, not allocation or memory bound, which is why the allocator and huge-page arms are flat.

`matrixmultiply` already selects its AVX-512 kernel at run time on this CPU. A microbenchmark of the 1024 x 100 by 100 x 5000 product (`gemm` 0.18 against matrixmultiply 0.3.11, 3 rounds): 1 thread 74 against 61 GFLOP/s, 8 threads aggregate 252 against 292 GFLOP/s (+16 %), max abs difference 0. The aggregate gain bounds a swap at a few percent of the run; it was not adopted (new dependency, below the effect of the read-ahead below).

## Miss audit (np600 / ro1000, harness live sets) [M]

| step | live | queries | set | misses | recall (tie-aware) | probe miss | scan loss | near tie | duplicate | harness-credited ties | queries with a miss | top 1% queries | top 10% queries | top 1% lists | top 10% lists |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 200 | 2999384 | 10000 | board10k | 850 | 0.9915 | 798 | 32 | 20 | 0 | 1 | 778 | 20.2% | 100.0% | 18.1% | 79.4% |
| 200 |  | 500 | board500 | 36 | 0.9928 | 35 | 1 | 0 | 0 | 0 | 34 | 19.4% | 100.0% | 100.0% | 100.0% |
| 200 |  | 2000 | heldout2000 | 183 | 0.9909 | 167 | 10 | 6 | 0 | 0 | 167 | 19.7% | 100.0% | 37.2% | 100.0% |
| 200 |  | 2000 | next2000 | 173 | 0.9913 | 156 | 10 | 7 | 0 | 0 | 157 | 20.8% | 100.0% | 33.5% | 100.0% |
| 360 | 7725755 | 10000 | board10k | 421 | 0.9958 | 389 | 23 | 9 | 0 | 1 | 393 | 30.4% | 100.0% | 25.2% | 100.0% |
| 360 |  | 500 | board500 | 14 | 0.9972 | 13 | 0 | 1 | 0 | 0 | 14 | 35.7% | 100.0% | 100.0% | 100.0% |
| 360 |  | 2000 | heldout2000 | 93 | 0.9953 | 84 | 7 | 2 | 0 | 0 | 90 | 24.7% | 100.0% | 59.1% | 100.0% |
| 360 |  | 2000 | next2000 | 75 | 0.9962 | 64 | 9 | 2 | 0 | 0 | 62 | 44.0% | 100.0% | 73.3% | 100.0% |
| 640 | 10292043 | 10000 | board10k | 445 | 0.9956 | 400 | 31 | 14 | 0 | 1 | 405 | 31.5% | 100.0% | 23.6% | 100.0% |
| 640 |  | 500 | board500 | 20 | 0.9960 | 18 | 2 | 0 | 0 | 0 | 18 | 35.0% | 100.0% | 100.0% | 100.0% |
| 640 |  | 2000 | heldout2000 | 101 | 0.9950 | 83 | 11 | 7 | 0 | 0 | 90 | 30.7% | 100.0% | 55.4% | 100.0% |
| 640 |  | 2000 | next2000 | 76 | 0.9962 | 72 | 4 | 0 | 0 | 0 | 72 | 31.6% | 100.0% | 65.8% | 100.0% |


Reading:
- Of the remaining misses, 93 to 95 % are IVF probe misses: neither of the missed neighbour's two lists ranks inside the 600 probed lists for that query. 4 to 7 % are lost in the scan or the reorder although a list was probed. 2 to 3 % are near ties with the 10th neighbour (relative distance gap below 1e-3 with a returned substitute at the same distance). There are no exact or near duplicates, and the harness credits one tied neighbour in 100,000 slots [M]. The reachable ceiling set by ties is therefore about 0.0002 above the measured recall; the rest is reachable only with more probes or better lists (run17: np1500 gives 0.9998 to 1.0).
- Held-out queries (2,000 from MSTuring query100K, none in the board set) and the two board sets miss at the same rates (0.9909 to 0.9962 against 0.9928 to 0.9972), so the audit does not depend on the board queries.
- Concentration: every miss sits in at most 8 % of the queries. The 100 worst queries (top 1 %) carry 20 % (3.0 M live), 30 % (7.7 M) and 32 % (10.3 M) of the board misses; no query misses more than 4 neighbours. Over lists, the 50 busiest primary lists (top 1 %) carry 18 to 25 % of the missed neighbours; 675 of 5,000 lists are involved at 3.0 M live. The top-10 % rows read 100 % because fewer than 10 % of queries miss.

## Elite and ablation

The elite is one component, the adapter read-ahead (`scripts/lowlevel/fork-streaming-io-readahead.patch`, applied to the harness at 56fc6e6b); every other candidate was killed on the proxy. With one component the ablation is control against elite, and a factorial has a single factor.

Full 1,280-step runs, np600 / ro1000, Ice Lake VM, 8 GiB scope, nice 19, taskset 0-7, page cache dropped before each run [M]:

| run | recall@10 | min post-train | step sum s | elapsed | insert s | delete s | search s | peak RSS GiB |
|---|---|---|---|---|---|---|---|---|
| control IL-S2E (a), c5cd1c61, same VM, earlier today | 0.993062 | 0.9836 | 2601 | 43:22.14 | 270.712 | 36.776 | 2247.615 | 6.53 |
| elite1 (read-ahead) | 0.993062 | 0.9836 | 2587 | 43:09.02 | 272.441 | 37.501 | 2269.840 | 6.60 |

Results are bit-identical to the control (same recall, same post-train minimum). The full-run gain (-14 s, -0.5 %) is far smaller than the proxy's -2.4 %: the control run's input file was probably already in the VM page cache from earlier runs, so its reads were cheap, while the elite run started from a dropped cache [I, not verified]. The honest reading is that read-ahead removes the cold-read cost (read time -90 % in the proxy) but that cost is at most about 1 % of the run. Step-sum change against the control: -14 s (-0.5 %) (elite runs finished: 1; the second run was started and stopped at the tool-call cap, so the two-run requirement is NOT met and the figure rests on a single run). The control is the IL-S2E run of earlier today, not an interleaved partner, so the comparison carries the between-run variance of the VM, which was not measured separately; the proxy shows 0.2 % between identical runs [M]. Recall and the term split are in `benchmarks/results/lowlevel/ice_lake_full_*_report.txt`. Joint effects among the killed singles (huge pages, placement, read-ahead together) were not run; the single effects are 0.0 to -0.7 %, so an additive prediction is -3 % dominated by read-ahead [I].

## Graph benchmark (SIFT-1M, official single-CPU protocol)

Run through the official `run.py --local`, one CPU (`taskset -c 4`), 2 runs per arm, under the timing lock, arms ctrl (main wheel), ybase (this branch, THP off), ythp (THP on), ymi (mimalloc). QPS at recall >= 0.95 and 0.99 (best point at or above the threshold, and log-linear interpolation):

```
dataset                        arm                     n | 0.95 hc med   interp                 rounds(hc) | 0.99 hc med   interp                 rounds(hc) | maxrec
sift-128-euclidean             ctrl                    1 |      17,858   18,524                     17,858 |       9,378    9,779                      9,378 | 0.9999
sift-128-euclidean             ybase                   2 |      18,279   19,876              14,788/21,771 |       7,924    8,443               5,102/10,745 | 0.9999
sift-128-euclidean             ythp                    1 |      16,990   17,751                     16,990 |       8,947    9,217                      8,947 | 0.9999
12:16:07 done ymi sift-128-euclidean r 0 rc 1 1643s
12:16:07 start ybase sift-128-euclidean r 1 load (6.13525390625, 5.40234375, 6.0458984375)
12:56:22 done ybase sift-128-euclidean r 1 rc 0 2415s
12:56:22 start ythp sift-128-euclidean r 1 load (1.0302734375, 1.51904296875, 2.57421875)

```

The host was shared with other timing jobs at load 6 to 18 during these runs, and ctrl against ybase (same algorithm, same code path, allocator wrapper compiled out) differs by up to 45 % at 0.99, so the graph benchmark cannot resolve a systems effect on this host [M]. mimalloc (ymi) failed to load in the wheel built before the dynamic-TLS fix (rc 1). No graph-benchmark gain is claimed.

## Docker and THP

THP is a host setting (`/sys/kernel/mm/transparent_hugepage/enabled`), shared by containers [I]; the VM is `madvise` and Ubuntu's default is `madvise`. The Docker daemon on the VM was stopped, so a container probe was not run. Every flag tested here is expressible in the board Dockerfile through `RUSTFLAGS` or a cargo feature; none is adopted.

## Reproduce

`scripts/lowlevel/replay.py` (proxy, audit, read-ahead arm), `scripts/lowlevel/summ.py` (tables), `scripts/lowlevel/audit.py` (miss classes), results under `benchmarks/results/lowlevel/`.

The killed arms' code is not on main. Build commit `46740d6a` to rerun them. It contains:
- the `WARM_THP` and mimalloc allocator wrapper (`cfg(warm_wheel_alloc)`, `src/bindings/python/warm_alloc.rs`);
- the `place` / `WARM_PLACE` list-order placement;
- the replay driver's `--place` flag.

The read-only `PartitionIndex.centres()` and `lists_of()` diagnostics stay, because `audit.py` needs them. The read-ahead arm is a patch to the big-ann fork's `streaming_io.py` (`scripts/lowlevel/fork-streaming-io-readahead.patch`). It is not applied, because its full-run gain (-0.5%, n=1) cannot move the submitted nprobe.

## Static graph follow-up: 2026-11-09

This investigation accompanies [VectorDBBench PR 898](https://github.com/zilliztech/VectorDBBench/pull/898).
It examines memory traffic, integer width, floating-point scaling, persisted
identity and Python allocation boundaries. Control is `431722cf`; final native
source is `4299b539`, with the snapshot publisher protocol at `699c480e`.
The released adapter is `dcondrey/VectorDBBench:4c6bdb5`, pinned to
`warm-python==0.6.4`. New core features are unreleased. Streaming and filter Azure
runs retain their independent frozen pins.

### Protocol and scope

Development and compilation use owned Apple M4 and Intel Core Ultra 9 185H hosts.
Paid cloud resources execute frozen full live benchmarks only. Full harness runs
use Cohere 1M × 768D cosine, all 1,000 queries, k=100, nq=1, M=16 (degree 32),
build_ef=300, ef=rerank=150 and concurrency 1/5/10 for 30 seconds each.
Source patches, input hashes, raw results and reproduction scripts are in
[`benchmarks/results/lowlevel/static-graph-2026-11-09/`](../../benchmarks/results/lowlevel/static-graph-2026-11-09/).
The source-commit map records a DCO/signature rewrite: each measured old commit
and its replacement have exactly the same Git tree.

Historical Cohere public queries were already exposed. Seed 898 divides them
into 500 development and 500 confirmation queries, but does **not** make either
partition an untouched holdout. No quality settings from this investigation
are adopted: ef=150 remains the existing setting. Speed transformations retain
score arithmetic and are checked against complete ordered-ID digests, including
ties. Synthetic and extreme-input tests use independent exact or float64
oracles. These checks establish specified behavior, not universal generalization.

### Changes and counterexamples

- Paired 4-bit primary/8-bit residual reranking shares float-query loads while
  retaining sixteen accumulation lanes, separate multiply/add and reduction
  order. NEON and AVX2 differential tests compare exact float bits.
- Rerank prefetch formerly requested every candidate row before scoring any.
  The candidate keeps a lead of 1–8 rows bounded by an 8 KiB row footprint.
  Scores and search effort are unchanged.
- Scalar/list Python queries reserved k elements even for a tiny index. Padded
  results multiplied nq×k unchecked. Huge k reproducibly panicked or aborted;
  an empty batch also segfaulted because zero elements hid an overflowing row
  stride. Unpadded buffers now cap at the vertex count. Padded results validate
  byte length and stride, then reserve fallibly. This covers VectorIndex,
  UpdatableIndex, FilteredIndex and experimental PartitionIndex. It cannot
  promise survival of system OOM.
- Finite extreme cosine queries broke scale invariance. Against 16 identity
  rows, an ascending query scaled by f32::MAX returned 2,3,...,15,0,1 instead
  of 15..0 despite examining every row. Extreme angular queries are rescaled;
  ordinary arithmetic is unchanged. Tests cover 4/8-bit codes, overflow and
  underflow magnitudes, and SIMD-width boundaries.
- Eight-bit query dot products accumulated in i32 across accepted dimensions.
  A four-row angular case reverses order at 133145 dimensions; at 262144 it
  returns the negative vector first. Long rows now accumulate bounded chunks
  into i64. Scalar/AVX2/AVX-VNNI tests use an i64 oracle up to the supported
  one-million-dimensional limit. Ordinary bounded rows keep their fast path.
- Persisted original IDs were range-checked but need not form a permutation.
  A modified 16-row index loaded and returned ID 0 sixteen times. Loading now
  rejects duplicates with a fallibly allocated n/8-byte bitmap, tested across
  mapped, prefaulted and heap residency. The file format is unchanged.
- The adapter formerly sorted an assembled matrix with advanced indexing,
  copying all 3.072 GB for 1M × 768D. It now validates chunk shapes and assembles
  rows directly in sorted-ID order. Old/new index SHA256 is identical on the
  local full build. Removing that copy is structural evidence; measured overall
  build time and peak RSS did not establish a corresponding gain.

### Full-harness local throughput

Intel Core Ultra 9 185H, 22 logical CPUs, WSL Ubuntu 24.04, Python 3.11.18;
no other benchmark or compiler ran on this host during timing. Every row below
has recall@100=.9406 and nDCG=.9496. These tables show concurrency-10 QPS;
complete concurrency-1/5/10 results and latency quantiles are retained in JSON.
Historical shared-M4 figures in PR 898 are superseded as throughput evidence.

The first ablation order was control/paired/prefetch/combined and its reverse,
with published-SDK controls around it:

| Source/arm | Run 1 QPS | Run 2 QPS |
|---|---:|---:|
| Main `431722cf` | 20,018.48 | 20,539.26 |
| Paired rerank alone | 20,437.23 | 19,142.87 |
| Bounded prefetch alone | 20,924.31 | 21,171.46 |
| Paired + prefetch + extreme-query guard | 21,089.17 | 21,470.67 |

The combined mean is 4.94% above the main-control mean. Paired-only run 2 is
slower and remains in the record. Two runs do not establish a precise universal
speedup. Published 0.6.4 outer runs were 19,292.20 (fresh), 20,325.02 and 20,937.43.

A second frozen comparison uses the integrated correctness fixes and new
adapter. Existing `WARM_QGRAPH_BATCH_EST=1` groups four candidate vectors within
**one query**; nq remains 1. It is not enabled by default by these changes.

| Source | Candidate grouping | Run 1 QPS | Run 2 QPS |
|---|---|---:|---:|
| Published 0.6.4 | Off | 20,524.32 | 19,749.77 |
| Integrated `90a55d81` | Off | 21,027.44 | 20,542.81 |
| Integrated `90a55d81` | On | 22,283.90 | 22,127.41 |
| Published 0.6.4 | On | 22,789.51 | 22,830.50 |

Integrated default improves 3.22% over the contemporaneous released-default
mean. Grouping improves the integrated mean 6.83%; however, released grouped
is 2.72% faster than integrated grouped. A fresh released grouped run gave
21,970.45 QPS, insert 35.62 seconds, optimize 211.37 seconds. This is modest
throughput progress, not a demonstrated 2× gain.

### Sharing immutable index memory

Private index copies cost memory for every worker. `VectorIndex.load(path,
shared=True)` now accepts a Linux memfd with irreversible WRITE, SHRINK and GROW
seals, verified on the exact opened file before mapping. Ordinary mutable files
and FUTURE_WRITE alone are rejected; the existing parser still validates all
index structure. Private loading remains the default. A mapping stays valid
after the publisher closes its descriptor. All workers must open the same inode
for physical pages to be shared.

The benchmark publisher copies and hashes the index once, seals it, and records
setup and total wall time. A bounded binding identifies both the source file
and the sealed snapshot. Workers validate that binding without rereading every
vector. This is a private parent/worker protocol, not authentication for an
untrusted publisher. Publisher and workers are inside the same measured cgroup.

Full official runs below use the same Intel host restricted to 8 logical CPUs,
16 GiB, no swap, grouping enabled, and heap/shared/shared/heap order. Recall is
.9406 throughout. QPS is the harness maximum across the three reported levels;
all levels and memory receipts are preserved.
The local sequence did not clear the host page cache between containers;
the registered Azure sequence clears it before every run.

| Load mode | Run 1 QPS | Run 2 QPS | Peak GiB, run 1 / 2 |
|---|---:|---:|---:|
| Private heap | 18,689.16 | 18,519.60 | 14.6165 / 13.8040 |
| Shared sealed snapshot | 18,048.95 | 18,156.31 | 3.2196 / 3.2205 |

Mean peak cgroup memory falls 77.34%, while mean maximum QPS falls 2.70%.
This is a memory/throughput tradeoff, not a throughput improvement.

The initial shared implementation hashed both complete files again per worker.
Its retained full ABBA results were heap 17,713.88/18,902.60 maximum QPS versus
shared 16,298.74/16,651.12, with respective peak memory 14.9764/13.8047 and
3.2167/3.2157 GiB. Inspection confirmed the official throughput timer includes
worker initialization. The binding protocol removes redundant validation reads;
the official timer and query loop remain unchanged. Snapshot setup is reported
separately because it happens before that timer.

### Query replay, separate from harness QPS

A quiet M4 ABBA comparison initially measured 1.85–3.61% combined throughput
gains across ef=120/150/200/400/600/800. A later five-arm mirrored comparison
(two runs per arm, five randomized full-query orders per run) measured these
median microseconds/query. Every arm's ordered-ID SHA256 matched at all six
efforts. Timing includes Python calls and output assignment.

| ef | Main | Paired only | Prefetch only | Integrated | Integrated grouped |
|---:|---:|---:|---:|---:|---:|
| 120 | 119.06 | 116.43 | 117.34 | 112.54 | 108.88 |
| 150 | 148.38 | 144.01 | 143.88 | 137.76 | 137.01 |
| 200 | 188.26 | 185.14 | 187.91 | 179.36 | 175.57 |
| 400 | 353.85 | 357.40 | 358.65 | 352.00 | 338.99 |
| 600 | 521.20 | 521.81 | 510.10 | 518.88 | 494.65 |
| 800 | 696.67 | 721.33 | 675.06 | 690.22 | 642.98 |

Paired-only regresses at high effort. Between-session variation exceeds some
component effects, so these are workload/host measurements, not isolated causal
estimates for every CPU. On Intel, grouping's ABBA query-loop gain was 9–12%
across these six efforts with identical IDs; full-harness limits appear above.

### Rejected ideas

An untimed safe-pruning probe on all 500 development queries at ef=150/400/800
could skip **zero** rerank candidates, even with an optimistic oracle final
cutoff. It used actual primary/residual norms and query-quantization error;
a tighter bound after the complete primary dot also skipped zero residual
reads. All 1,500 records are retained. This is a negative feasibility result on
an exposed dataset, not a general impossibility theorem.

A narrower filter-only dot-product prototype exploited that filter dimensions
are already bounded below the i32 overflow threshold. Intel results ranged from
−2.2% to +3.7%, and Mac results were around ±1%, with identical IDs. There was no
consistent benefit; the prototype is not integrated. Its patch and raw results
are retained. Partial sorting was deferred because diagnostic consumers require
the complete sorted rescored pool. The existing visited set already uses a
bitset and touched-word reset.

### Validation and comparison limits

At final native source `4299b539`, default Rust suites passed 1,018 tests
(19 ignored), all-features passed 1,667 (22 ignored); default/python/all-features
clippy, rustfmt and MSRV 1.89 passed. Linux Python tests passed 164 with 3
feature-dependent skips; Mac passed 131 with 36 Linux-only skips. The publisher
binding tests passed 24 on Linux; the adapter and UI passed 63. Sealed-file tests
cover required seals, existing writable mappings, publisher lifetime, corrupt
files and failed-load preservation. Validation receipts accompany the artifacts.

Mac modules use identical `-C strip=none` and dynamic-Python linking flags because
this macOS 27 host encounters Rust issue 157750's stripped-Mach-O LINKEDIT problem.
This is a build workaround, not a performance change.

Azure VDBBench full runs are registered after the independent ANN live chain:
8 vCPUs, 16 GiB/no-swap containers, unchanged query settings and 14 mirrored
released/main/integrated/shared arms. At the user's stop request, input checks and
image loading had completed, but no VDBBench workload had started. The service
was stopped; no Azure VDBBench result is available. Frozen inputs and the stop
receipt are retained. Local figures cannot establish a win against Zilliz Cloud: an embedded index and a managed service
have different deployment costs and boundaries. The published comparison point
is [13,316.23 QPS at recall@100=.9383 on the $1000/month tier](https://zilliz.com/vdbbench-leaderboard).
No matched-service or official leaderboard standing is claimed.
