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
