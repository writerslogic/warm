# big-ann-benchmarks streaming track (NeurIPS'23)

The streaming track replays a runbook of inserts, deletes and searches from an empty index and
scores the mean recall@10 over its search checkpoints, under 8 GiB of DRAM and one hour.

WARM's current entry runs the final runbook (msturing-30M-clustered, 1,280 steps) on
`PartitionIndex`. Its result is in [the partition tier section](#final-runbook-msturing-30m-clustered-partition-tier):
mean recall@10 0.9942 in 2,722 s at nprobe 650 and reorder 1000, with the submitted configuration
pinned at `f21de886`, measured on a GCP Ice Lake VM. It is not a board result; wall time on the
Azure evaluation VM is unmeasured.

The sections before it describe the earlier graph entry. It ran the delete runbook
(`neurips23/runbooks/delete_runbook.yaml` on msturing-10M-clustered: 100-d float32, L2, at most 6M
live rows, 33 search checkpoints of 10,000 public queries) with `UpdatableIndex`
(`src/bindings/python/vector_index.rs` over `core::qgraph::UpdatableVGraph`): a memory-mapped base
graph, an in-memory delta graph of the rows inserted since the last consolidation, and a rebuild of
the base over the live rows when the delta or the deleted share passes the adapter's thresholds
(delta 1.0x the base, 30% deleted, 100k rows minimum).

Every number in those graph-entry sections traces to
`benchmarks/results/bigann_streaming_delete_runbook_v1.json` or, from "Operating point" on,
`benchmarks/results/bigann_streaming_delete_runbook_v2.json`; both record the WARM commit, the
harness commit (`89a3aba`), the machine and the parameters. The final-runbook numbers trace to the
evidence directories under `benchmarks/research/l22_partition/results/`.

## Where the recall went

The recall deficit at the ef the leaderboard entries use (64-128) was diagnosed on the first
1M rows of the dataset (float64 brute-force ground truth over the live rows, 1,000 public
queries) and on the first 12 steps of the official runbook against the harness ground truth,
with per-search-step recall recorded. The hypotheses that deletes or the delta were the loss
are falsified by that record: recall is low at the first checkpoint, before any delete, and
stays flat across deletes and consolidations (at ef 1024 the 33 official checkpoints sit in
0.91-0.98 with no trend). Two settings of the build explain the gap to the static
`VectorIndex` on the same rows and the gap to the track baseline:

| 1M prefix, R32 L64, recall@10 | ef 64 | ef 128 | ef 256 |
|---|---|---|---|
| static `VectorIndex` (alpha 1.0) | 0.716 | 0.810 | 0.888 |
| `UpdatableIndex` base, alpha 1.2 (the old default) | 0.631 | 0.743 | 0.826 |
| `UpdatableIndex` base, alpha 1.0 | 0.716 | 0.811 | 0.888 |
| `UpdatableIndex` delta, alpha 1.2 | 0.619 | 0.714 | 0.798 |
| `UpdatableIndex` delta, alpha 1.0 | 0.667 | 0.763 | |
| static `VectorIndex` R64 L128 | 0.836 | 0.912 | 0.957 |
| `UpdatableIndex` base R64 L128, alpha 1.2 | 0.796 | 0.887 | 0.941 |

* `alpha`, the pruning parameter of the second construction pass, defaulted to 1.2 in
  `UpdatableIndex.create` (and in the track adapter) while `VectorIndex` and
  `BuildParams::default()` use 1.0. At degree 32 the alpha-1.2 prune keeps nearer, shorter
  edges and loses 7-9 points of recall@10 at ef 64-128 on both the base and the delta. With
  alpha 1.0 the consolidated base reproduces the static index exactly (the regression test
  `consolidated_base_matches_static_build_at_equal_params` pins that equality). The default
  is now 1.0.
* The remaining distance to the DiskANN baseline (R64, Ls 100: 0.924) is the degree: R64 L128
  gains another 12 points at ef 64 and 10 at ef 128 on the prefix, at 2x the build time per
  consolidation and about 128 more bytes per live row; R32 peaked at 7.47 GiB
  (sampled summed RSS, Modal, ef 1024) and at 4.85 GiB (GNU time, writerslogic-02, ef 64 and 128), and
  the official R64 L64 run at ef 256 peaked at 5.57 GiB on Modal (see the official runs table).
* Candidate retention (`reservoir`) changes nothing at any ef (0.7161 at 0, 256 and 512), so
  neither quantized estimates nor pool eviction is a loss mechanism here; the queries are
  simply far from the clustered rows (in-distribution queries reach 0.973 at ef 64 on the
  same graph).

Per search step of the official runbook (harness ground truth, same wheel, R32 L64 T8):

| step (live rows) | ef 64 before | ef 64 after | ef 128 before | ef 128 after |
|---|---|---|---|---|
| 2 (256k) | 0.706 | 0.777 | 0.825 | 0.875 |
| 4 (492k) | 0.712 | 0.771 | 0.814 | 0.862 |
| 6 (825k) | 0.586 | 0.692 | 0.710 | 0.796 |
| 8 (725k) | 0.605 | 0.705 | 0.725 | 0.805 |
| 10 (981k) | 0.654 | 0.739 | 0.761 | 0.830 |
| 12 (1.47M) | 0.644 | 0.721 | 0.738 | 0.807 |
| mean of the six | 0.651 | 0.734 | 0.762 | 0.829 |

## Official runs (delete runbook, `run.py --nodocker`, 8 CPUs, 8 GiB)

| WARM | build | ef | recall@10 | wall | peak RSS | host |
|---|---|---|---|---|---|---|
| ff70f3e | R32 L64 alpha 1.2 | 64 | 0.6814 | about 18 min per ef | 7.78 GB (both) | Modal 8 vCPU |
| ff70f3e | R32 L64 alpha 1.2 | 128 | 0.7755 | | | Modal 8 vCPU |
| cc5dede | R32 L64 alpha 1.2 | 1024 | 0.9301 | 36.0 min | 7.47 GiB | Modal 8 vCPU |
| 791749b, alpha 1.0 passed explicitly | R32 L64 alpha 1.0 | 64 | 0.7623 | 59:40 for both replays, nice 19 | 4.85 GiB (GNU time, both) | writerslogic-02, cores 0-7 |
| 791749b, alpha 1.0 passed explicitly | R32 L64 alpha 1.0 | 128 | 0.8401 | | | writerslogic-02, cores 0-7 |
| c80aa52 (main) | R32 L64 alpha 1.0 | 64 | 0.7623 | 14:24 (runbook steps 864 s) | 4.98 GiB | Modal cpu 8.0 |
| 082766e | R32 L64 alpha 1.0 | 64 | 0.7623 | 13:32 (runbook steps 811 s) | 4.85 GiB | Modal cpu 8.0 |
| 082766e | R32 L64 alpha 1.0 | 128 | 0.8401 | 18:53 (1,132 s) | 4.84 GiB | Modal cpu 8.0 |
| 082766e | R32 L64 alpha 1.0 | 384 | 0.9226 | 20:51 (1,250 s) | 4.87 GiB | Modal cpu 8.0 |
| 082766e | R32 L64 alpha 1.0 | **512** | **0.9371** | **15:41 (941 s)** | 4.84 GiB | Modal cpu 8.0 |
| 082766e | R64 L64 alpha 1.0 | 256 | 0.9353 | 15:09 (908 s) | 5.57 GiB | Modal cpu 8.0 |

The baseline the track README quotes is DiskANN at R64 L50 Ls100: 0.924 in about 23 minutes
on the competition VM (Azure D8lds v5). The writerslogic-02 runs are recall runs at `nice 19`
on a shared host (Core Ultra 9 185H, 8 of 22 threads, 8 GiB cgroup cap); their wall time is
an upper bound, not a timing claim.

The Modal runs at 082766e each replay the runbook once (one round per row), the R32 ones in
four containers started together, under `/usr/bin/time -v` around `run.py`, with an 8 GiB hard
memory limit and non-preemptible scheduling. `cpu=8.0` gives eight schedulable CPUs; Modal
bills `cpu` in physical cores and hides the topology inside the container, so whether these are
eight cores or eight hyperthreads is not known, and the compute is not matched to the
competition VM's 8 vCPU (D8lds v5). Wall time spread between the containers is host noise as
much as ef: the ef 512 replay finished before the ef 384 one, and the runbook-step sums differ
by 33% in the direction opposite to the search work. The runbook-step sum is the harness's own
per-step time; the wall includes start-up and the dataset range reads.

## Operating point

At R32 L64 alpha 1.0, **ef 512 scores 0.9371 against the track baseline's 0.924, at a 4.84 GiB
peak, in a 15:41 replay (941 s of runbook steps)**; ef 384 falls just short (0.9226). Every one
of the five Modal replays finished in 13:32-20:51 against the baseline's 23 minutes, one round
each and on different hardware from the baseline's, so that is an observation, not a matched
timing comparison. The ef 64 and ef 128 rows reproduce the Core Ultra 9 numbers of the 791749b runs (alpha 1.0 passed explicitly) to
the fourth digit, and main without the 3-byte ids scores the same 0.7623 on Modal, so the
change leaves the results unchanged.

The choice came from full-runbook replays on a shared Intel Core Ultra 9 185H (8 threads per
replay, nice 19, load average 11-25; `replay.py`: the official runbook through `UpdatableIndex`
with the adapter's policy, recall@10 against the harness ground truth at every checkpoint,
several ef per pass). Mean recall@10 over the checkpoints each replay reached (rows marked
partial were stopped early; their means cover the early checkpoints, which score higher than
the late ones, so they are not comparable with full means):

| build | checkpoints | ef 64 | ef 128 | ef 192 | ef 256 | ef 384 | ef 512 | ef 768 |
|---|---|---|---|---|---|---|---|---|
| R32 L64 | 33 of 33 | | | | 0.8971 | 0.9226 | 0.9371 | 0.9537 |
| R64 L64 | 33 of 33 | 0.8281 | 0.8915 | 0.9195 | 0.9353 | | | |
| R64 L48 | 11 of 33 (partial) | | 0.8563 | 0.8940 | 0.9160 | 0.9411 | | |
| R64 L128 | 6 of 33 (partial) | 0.8533 | 0.9238 | 0.9524 | 0.9669 | | | |

Each complete replay reproduced the official score to the fourth digit (R32 ef 384 0.9226, ef
512 0.9371; R64 ef 256 0.9353), so a full replay is the official score without the harness.

Degree 64 reaches the same recall at half the ef: R64 L64 at ef 256 scores 0.9353 (0.0018 below
R32 L64 at ef 512) at 0.73 GiB more memory, and its 908 s of runbook steps are
indistinguishable from R32's 941 s given the spread between containers. R32 ef 512 is the
configured point. In both, the inserts and deletes, which run the four rebuilds of 1.9M to 3.8M
live rows, are 80-87% of the step time. The R64 L128 and R64 L48 replays were stopped early.

## Memory

Peak RSS at R32 is 4.84-4.87 GiB for the benchmark process (4.97 GiB sampled with the Modal
parent), reached near the end of the runbook, when the live count (5.2M) and the mapped base
pages searches touch are largest; at R64 it is 5.57 GiB. The anonymous peak comes earlier, in
the step-49 rebuild of 3.8M rows (old delta, new adjacency, Python): 2.1 GiB at R32 and 2.7 GiB
at R64 in the replays. The 8 GiB limit is not binding at either point. The earlier 7.47-7.78 GB
peaks were measured at ff70f3e and cc5dede, before #205 moved consolidation onto a mapped
scratch matrix and a bounded encoder instead of an owned row copy plus a full owned graph and
encoding; the drop is consistent with that change, which this one does not touch.

This change narrows the consolidated base's neighbour ids from 4 to 3 bytes whenever the
generation has fewer than 2^24 - 1 rows (4-byte ids above that; variable-width encodings are
untouched), as the static `VectorIndex` already does: the base vertex file shrinks from 380 to
348 bytes per row at R32 (508 to 444 at R64), about 166 MB at 5.2M rows. Measured on Modal at
ef 64, one round each, run one after the other: main (c80aa52) peaks at 5,350,993,920 bytes
(4.98 GiB) for the benchmark process and 5,572,100,096 sampled with the parent; this change at
5,212,020,736 (4.85 GiB) and 5,432,266,752, 139 MB lower (2.6%), with the same score (0.7623).

On the Core Ultra 9 host (no reclaim pressure, 12 GiB scope), the replays' RSS peaks split into
anonymous memory (delta graph, build adjacency, Python) and mapped files (base, scratch rows,
and the replay's own `numpy.memmap` of the source, which the official adapter does not hold):
R32 L64 peaks at 4.55 GiB VmRSS (2.0 GiB anonymous, 2.5 GiB mapped) at step 73 with 5.2M live
rows; R64 L64 at 6.04 GiB (2.0 anonymous, 4.1 mapped) at step 68 with 4.8M live rows.

## Final runbook (msturing-30M-clustered): pinned run (af8d4017, Modal 8 vCPU, 16 GiB, 2026-11-06)

The submission image (`neurips23/streaming/warm`, PR harsha-simhadri/big-ann-benchmarks#357, harness
5d6225a, `ARG WARM_COMMIT=af8d4017`) run as `run.py --nodocker --neurips23track streaming
--algorithm warm --dataset msturing-30M-clustered --runbook_path neurips23/runbooks/final_runbook.yaml`
on a Modal function with `cpu=(8,8) memory=(16384,16384)` (AMD EPYC Zen 4, Genoa, AVX-512; gVisor). `warm(R32_L64, {'ef': 256, 'rerank': 256})`: mean recall@10 over the search
checkpoints **0.8768**; `run.py` 6175 s (1.72 h) for the 1,280 steps, exit 0,
no timeout; peak RSS 7.51 GiB summed over processes, cgroup peak
5.28 GiB. Results: `benchmarks/results/bigann_streaming_final_runbook_v1.json`
The same image under an 8 GiB cap (`memory=(8192,8192)`, AMD EPYC Zen 2 without AVX-512) gave the identical
recall 0.8768 in 10959 s (3.04 h), exit 0, no OOM, peak RSS
7.57 GiB summed and cgroup peak 5.28 GiB. It completed under the recorded Modal
8 GiB memory cap; this does not establish official Docker or host eligibility.
The run-time difference mixes the slower CPU with the cap.

The upstream `ongoing_leaderboard` CSV (fetched 2026-11-06) lists scann 0.9924, zilliz 0.9219, pinecone 0.9123,
pyanns 0.8702, hwtl 0.7690, DiskANN 0.7218, cufe 0.6484; it differs from the older CSV below, so which ground truth and
metric the live board uses must be confirmed with the organisers before placing WARM. Older board CSV by best recall (fork copy at that harness commit): puck 0.9855 (16 GB docker mem_limit), hwtl_sdu_anns_stream 0.9675 (not open-sourced), pyanns 0.9597, diskann 0.8833, cufe 0.8189. The differing board snapshots and hosts do not establish a placement. The board requires the runbook to
finish within 1 hour on Azure D8lds_v5; this run took 1.72 h on different hardware, so that
condition is not shown to hold and the maintainers' run decides.

## Final runbook (msturing-30M-clustered): earlier loss

One run at the same setting (R32 L64 alpha 1.0, ef 512, 082766e, Modal cpu 8.0, 8 GiB) did not
finish: the runner stopped `run.py` at its 3,600 s limit after 541 of 1,280 steps (4.3M live
rows at that point), so no recall was recorded. Measured over those steps: 270 searches at 3.88 s
each (1,049 s), 143 insert steps 1,089 s, 128 delete steps 1,461 s (deletes include the
tombstone-triggered rebuilds); peak RSS 6.02 GiB for the benchmark process, 6.22 GiB summed with
the parent, so memory was not the limit. Projection, not measured: the 640 searches alone need
about 2,480 s at ef 512, so this runbook needs a lower ef and fewer or cheaper rebuilds to fit
the hour.

## Final runbook (msturing-30M-clustered): partition tier

The graph entry above could not finish this runbook within the hour. The entry now serves it from `PartitionIndex` (`src/bindings/python/partition_index.rs` over `core::qgraph::partition`):
- Brute force runs until 1M rows are live.
- After that, the index uses an IVF with 5,000 lists, SOAR spill, PQ-50 residual codes scanned by a 4-bit LUT, and an exact f32 reorder.
- The scan fuses the threshold test into the LUT kernel. On AVX-512 VBMI+VNNI CPUs, eight queries share each code block, and the exact reorder runs in batches.

Design, kill tests and the measurements behind each parameter are in [`docs/research/STREAMING-L22.md`](../research/STREAMING-L22.md).

All runs below use `run.py --nodocker` with the final runbook, all 1,280 steps, exit 0, in an 8 GiB scope without swap. The Ice Lake VM is a GCP n2-custom-8-16384 (Xeon @ 2.60 GHz, 4 cores / 8 threads, AVX-512 VBMI/VNNI). It is the evaluation VM's class (D8lds_v5), not that VM. Each row is one run [M]. Evidence is under `benchmarks/research/l22_partition/results/`.

| host | WARM | nprobe / reorder | mean recall@10 | min after training | step-time total | peak RSS | evidence |
|---|---|---|---|---|---|---|---|
| Core Ultra 9 185H | `e29b62fc` | 300 / 317 | 0.9726 | 0.9543 | 2,550 s | 6.45 GiB | `s2b_board_config` |
| Ice Lake VM | `e29b62fc` | 300 / 317 | 0.9726 | 0.9543 | 5,618 s, **over the hour** | 6.41 GiB | `il_s2b_icelake` |
| Ice Lake VM | `5bba104f` (fused scan) | 300 / 317 | 0.9726, identical | 0.9543 | 2,187 s | 6.47 GiB | `il_s2c_scan` |
| Ice Lake VM | `5bba104f` | 350 / 1000 | 0.9808 | 0.9627 | 2,466 s | 6.58 GiB | `il_s2d_np350_ro1000` |
| Ice Lake VM | `5bba104f` | 400 / 1000 | 0.9848 | 0.9686 | 2,666 s | 6.56 GiB | `il_s2d_np400_ro1000` |
| Ice Lake VM | `c5cd1c61` (8-query scan, batched reorder) | 600 / 1000 | 0.9931 | 0.9836 | 2,601 s | 6.53 GiB | `il_s2e_np600_ro1000` |
| Ice Lake VM | `c5cd1c61` | **650 / 1000** | **0.9942** | **0.9859** | **2,722 s** | 6.53 GiB | `il_s2e_np650_ro1000` |

- The submitted configuration is the last row, pinned at `f21de886`: `c5cd1c61` merged, with an identical source tree. It is the highest recall with a step-time total at or below 2,800 s, which leaves 878 s under the hour.
- The scan change cut Ice Lake search from 5,293 s to 1,790 s at identical recall. Blocks and admitted counts are identical. The extra time went into higher nprobe and a larger reorder. The second scan round (`c5cd1c61`) bought another 250 lists of nprobe at about the same time.
- The `neurips23/README.md` baselines table quotes DiskANN at 0.883 on this runbook.
- This is not a board result: wall time on the Azure evaluation VM is unmeasured. The submission is draft PR 357 on harsha-simhadri/big-ann-benchmarks.

## Merged consolidation (development measurements)

`UpdatableIndex.consolidate(merge=True)` keeps the previous base graph instead of rebuilding
it: it splices deleted rows out of the lists that pointed at them, links the delta in with the
build's first insertion pass, and runs the build's second pass over the delta, the base rows it
links to and the rows that lost a deleted neighbour (`docs/file-format/VECTOR-UPDATES.md`). The
fork adapter turns it on with `"merge": true`. Each Modal row below is one round (`run.py
--nodocker`, harness 89a3aba, `cpu=8.0`, 8 GiB, non-preemptible, R32 L64 alpha 1.0 T8, ef 512,
WARM 9b26418, which is c1ec45d before its rebase onto main); the control is the same image
without merging, run in the same hour, since step sums vary by about a third between containers.

| delete_runbook | delta / deleted ratio | recall@10 | wall | runbook steps: insert / search / delete s | peak RSS |
|---|---|---|---|---|---|
| rebuild (control) | 1.0 / 0.3 | 0.9371 | 17:42 | 1,061: 505 / 149 / 408 | 4.70 GiB |
| merge | 1.0 / 0.3 | 0.9376 | 12:07 | 726: 382 / 103 / 241 | 4.69 GiB |
| merge | 0.25 / 0.15 | 0.9444 | 17:32 | 1,052: 696 / 50 / 306 | 4.93 GiB |

A merge searches again for about four to six rows per changed row, so it beats a rebuild only
while the churn per consolidation is a fraction of the live set: merging at a quarter of the
base spends more on inserts than rebuilding at doubling. The configured point merges at the
rebuild thresholds (1.0 / 0.3): by construction a delta-triggered consolidation there usually falls back to a rebuild (the delta is then at least as large as the surviving base), so the merges replace the tombstone-triggered rebuilds.

Final runbook (msturing-30M-clustered), ef 256: no recall was recorded. With the image of 9b26418 at ef 256, merging at the rebuild thresholds (1.0 / 0.3) reached step 672 of 1,280 in 1,926 s of runbook steps (searches averaged about 1.0 s; the #238 run at ef 512 needed 3,599 s for its first 541 steps), then the launcher's 8 GiB guard on the summed benchmark and parent RSS stopped it at 8.60 GB during the step-673 consolidation (a merge after deletes, 8.42M live rows; the same step at which earlier rebuild-based candidates ran out of time or memory). Merging at 0.25 / 0.15 reached step 638 in 2,838 s before the same guard (8.61 GB). The ef 512 run at 0.25 / 0.15 was stopped at step 214 (1,461 s of steps against the #238 run's 976 s for the same steps) once it could not finish. The merge read the previous base's adjacency back into memory and kept those pages through the insertion passes; 50017d4 drops them as soon as the last list is read.

## Consolidation heap peak (development measurement)

The heap peak of a consolidation is the resident index and prepared rows plus the new adjacency, (R*4+4) bytes per live row (132 at R32, 260 at R64), plus the largest transient. The upper-layer build (a 29-32 MiB transient at 855k live rows) now runs before the adjacency is allocated instead of on top of it, and the merge drops its slot maps after the repair pass. Counting-allocator measurements on glove-100-angular (900k base rows, 30% deleted, 225k inserted, 855,414 live, build-ef 64, alpha 1.0) lowered the heap peak from 213.3 to 186.9 MiB (12.4%) for an R32 merge, 201.5 to 184.5 (8.4%) for an R32 rebuild, 350.3 to 326.7 (6.7%) for an R64 merge and 338.2 to 323.7 (4.3%) for an R64 rebuild. Base file and search fingerprints are identical between the two builds in all four cells. Maximum RSS did not change beyond run-to-run noise (1.19-1.46 GiB, 1216-1493 MiB, dominated by mapped pages), and wall time is not claimed. The adjacency alone is 65% of the R64 merge peak at this size. Per-run values and the method are in [`consolidation_heap_peak_v1.json`](../../benchmarks/results/consolidation_heap_peak_v1.json).

Extrapolation, not measured: scaling the heap peaks linearly to 30M live rows gives about 6.4 GiB (R32) and 11.2 GiB (R64, of which 7.26 GiB is adjacency) for a merge, 0.8-0.9 GiB below the previous build. Read against an 8 GiB limit, R32 leaves about 1.6 GiB for mapped pages and the rest of the process (the ef 256 run above reached 8.60 GB RSS at 8.42M live rows); R64 exceeds 8 GiB on heap alone and fits 16 GiB only before mapped pages are counted.

## Replacement entry memory (development measurement)

When consolidation deletes the previous entry vertex, it now selects the surviving row
nearest the mean directly from the prepared rows. Previously it copied all surviving
vectors and norms first. Accumulation order and ties remain identical; stored formats
and search parameters are unchanged.

A locked, interleaved three-pair kernel experiment on Intel Core Ultra 9 185H, eight
threads pinned to CPUs 0–7, load average 1.18–1.20, used 1M synthetic 100-dimensional
rows with 800k survivors. Median process peak RSS fell from 718,436 to 403,012 KiB;
median selection time fell from 182.64 to 38.79 ms. All six runs selected row 830166.
Raw values and source hashes are in `benchmarks/results/streaming_entry_memory_v1.json`.
These are development measurements, not streaming standings or full-consolidation costs.

To reproduce the parent arm, check out the recorded parent commit and apply
`benchmarks/results/streaming_entry_memory_v1/baseline.patch` from this revision. Build
both arms separately with `cargo test --release --lib replacement_entry_large_live_set
--no-run`. Under one `flock ~/warm-timing.lock`, run their test executables in A/B, B/A,
A/B order with `RAYON_NUM_THREADS=8 /usr/bin/time -v taskset -c 0-7 <test-executable>
replacement_entry_large_live_set --ignored --nocapture --test-threads=1`.

Whether step 673 deletes the previous entry, total consolidation RSS, and final-runbook
recall/completion remain unmeasured. After the standings freeze is lifted, pin the adapter
to the tested revision with R32/L64/alpha 1.0/T8, merge enabled, delta/deleted ratios
1.0/0.3 and ef256, then run in the harness checkout with its dataset and ground truth:

```sh
flock ~/warm-timing.lock systemd-run --user --scope -q \
  -p MemoryMax=8G -p MemorySwapMax=0 taskset -c 0-7 \
  /usr/bin/time -v timeout 3600 uv run python run.py --nodocker \
  --neurips23track streaming --algorithm warm --dataset msturing-30M-clustered \
  --runbook_path neurips23/runbooks/final_runbook.yaml
```

Record all 1,280 steps, mean recall across all 640 searches and peak RSS. A kernel
improvement does not establish that the one-hour/8-GiB limits or a leaderboard target
have been met.

## Earlier graph-page release (development measurement)

Consolidation now marks rows that lose deleted neighbours while repairing their adjacency,
then releases the old graph pages before inserting delta rows. This removes a second scan
of the old adjacency. The encoded generation remains byte-identical.

Three locked interleaved pairs on Intel Core Ultra 9 185H, eight threads pinned to CPUs 0–7,
used a 500k-row 100D base, degree32/build-ef64, every seventh row deleted except the entry,
and 100k inserted rows. At the end of the insertion phase, RSS was 441,480–444,072 KiB on
the parent versus 381,084–381,300 KiB with earlier release. All six encoded generations
had CRC32 434904111. Whole-process peak RSS stayed approximately 516–518 MiB
(528,836–530,216 KiB); that high-water mark includes fixture construction. Consolidation
times varied 19.86–27.29 s versus 19.79–22.32 s; no reliable speedup is established.

`benchmarks/results/streaming_page_release_v1.json` records each pair and load average.
Apply its sibling directory's `baseline.patch` to the recorded parent to add the same
fixture and generation test. Build both test executables with
`cargo test --release --lib consolidation_live_set_peak --no-run`. Hold the timing lock,
set `RAYON_NUM_THREADS=8 WARM_CONSOLIDATION_ROWS=500000`, and run A/B, B/A, A/B with
`/usr/bin/time -v taskset -c 0-7 <test-executable> consolidation_live_set_peak --ignored
--nocapture --test-threads=1`. Phase instrumentation is enabled identically in both arms.

An isolated consolidation peak, an 8-GiB constrained scaled replay, and final-runbook
completion/recall remain unmeasured. After the standings freeze is lifted, pin the adapter
to this revision with R32/L64/alpha1.0/T8, merge enabled, delta/deleted ratios 1.0/0.3,
ef256, and run in the prepared harness checkout:

```sh
flock ~/warm-timing.lock systemd-run --user --scope -q \
  -p MemoryMax=8G -p MemorySwapMax=0 taskset -c 0-7 \
  /usr/bin/time -v timeout 3600 uv run python run.py --nodocker \
  --neurips23track streaming --algorithm warm --dataset msturing-30M-clustered \
  --runbook_path neurips23/runbooks/final_runbook.yaml
```

Require all 1,280 steps, recall averaged over all 640 searches, and memory below 8 GiB.
The phase-RSS reduction alone does not establish that outcome.

## Delta-row spill (development evidence; incremental comparison pending)

Consolidation moves committed delta rows into a private unlinked read-only mapping before
building the next generation. Copying rows releases mapped pages in batches. The writer
serializes mutations; scratch I/O occurs outside the state lock. Failed consolidation can
still accept subsequent inserts by detaching the mapped backing. Persisted bytes are unchanged.

The completed development comparisons use older parent `e3bb3648`, so they combine
entry selection, earlier graph-page release and delta spilling. They do **not** isolate this
PR's benefit over its parent `0880ccda`. On Intel Core Ultra 9 185H, eight threads on CPUs
0–7 under the timing lock, one 3M-base/600k-insert pair within an 8-GiB cgroup reduced
isolated consolidation peak RSS from 2,581,784 to 2,348,628 KiB (9.0%). Consolidation took
185.401 versus 185.642 seconds; both generations had CRC32 2824448824. Linux
`clear_refs=5` reset the high-water mark before consolidation. Load averages and smaller
three-pair results are in `benchmarks/results/streaming_delta_spill_v1.json`.

The cached-slice implementation's single-thread query check gave paired median ratios
0.9939, 1.0263 and 0.9942, with identical result fingerprints; no consistent speed change
was established. A discarded per-neighbour backing lookup was 0.6–1.0% slower.

Local formatting, clippy, tests, byte-output and interrupted-consolidation recovery checks
passed. The master session owns the incremental comparison against the current PR parent.
Build both arms with `cargo test --release --lib --no-run`, adding the candidate's ignored
measurement fixtures to the parent without its production changes. Under the timing lock,
run A/B, B/A, A/B with `RAYON_NUM_THREADS=8 WARM_CONSOLIDATION_ROWS=500000`, an 8-GiB
cgroup and `taskset -c 0-7 <test-executable> consolidation_live_set_peak --ignored
--nocapture --test-threads=1`; also run `delta_search_live_set` in both arms.

Scaled official-data replay, eight-thread search throughput and final-runbook completion
and recall remain unmeasured. After the freeze is lifted, pin the tested revision and use
the R32/L64/alpha1.0/T8 merge configuration, ratios1.0/0.3, ef256:

```sh
flock ~/warm-timing.lock systemd-run --user --scope -q \
  -p MemoryMax=8G -p MemorySwapMax=0 taskset -c 0-7 \
  /usr/bin/time -v timeout 3600 uv run python run.py --nodocker \
  --neurips23track streaming --algorithm warm --dataset msturing-30M-clustered \
  --runbook_path neurips23/runbooks/final_runbook.yaml
```

Require all 1,280 steps within one hour and 8 GiB, and report recall over all 640 searches.
These development measurements establish no leaderboard standing.

The delta frontier now rejects a noncompetitive scored neighbour before searching
for its insertion position. Three alternating development pairs on an Intel Core
Ultra 9 185H (CPUs 0–7, timing lock, five measured batches per arm) compared
`7db61aa0` with `5c8e8ec`. On the first 1M official clustered rows and 256 official
queries, median query-time ratios were 0.704/0.737 with one thread and 0.809/0.828
with eight threads at ef256/512; every pair improved. The synthetic 100k-row
control ratios were 0.828/0.834 and 0.879/0.830. One-minute load averages ranged
2.16–7.45 on official data and 1.80–3.13 on synthetic data. All full-frontier score
bits and IDs matched across arms, pairs and thread counts. The sort-based oracle
also preserves traversal visit counts, including tied scores and deleted routing
vertices. Per-arm timings, fingerprints and commands are in
[`streaming_frontier_v1.json`](../../benchmarks/results/streaming_frontier_v1.json).
This measures live delta queries; total streaming throughput and the final-runbook
recall still require the post-freeze command above.

## Rescore budget on a base with deletes (2026-11-06, macOS development measurement)

A search on a base with tombstones widened its candidate pool to `ef / (1 - deleted share)`
and rescored every live candidate exactly from the mapped f32 rows, whatever `rerank` said.
It now rescores the `rerank` best live candidates in estimate order (never fewer than `k`);
`rerank = 0` still rescores all of them, and a `rerank` at or above the live candidate count
returns what it returned before. The adapter passes `rerank = ef`, so the pool holds about
`ef` live candidates and the change does nothing at that setting.

Three alternating pairs on an Apple M4 (shared, one-minute load averages 3.4-14.8, no timing
lock, no pinning) compared origin/main `18273ef4` plus only the new fixture with the change,
on the first 1M official clustered rows with 30% of them tombstoned in 10,000-row blocks and
1,000 official queries, R32 L64 alpha 1.0. recall@10 against the exact top 10 over the live
rows was identical in both arms at every setting (0.8590 at ef 256, 0.9230 at ef 512), and
`rerank = ef` returned identical result fingerprints. Median candidate/parent query-time
ratios (the three per-pair ratios ranged 0.67-1.11):

| ef | rerank | 1 thread | 8 threads |
|---|---|---|---|
| 256 | 256 | 1.028 | 0.958 |
| 256 | 128 | 0.911 | 0.844 |
| 256 | 64 | 0.839 | 0.875 |
| 512 | 512 | 1.005 | 0.981 |
| 512 | 256 | 0.900 | 0.845 |
| 512 | 128 | 0.849 | 0.826 |

Against the kill bar (recall@10 drop above 0.0025 at equal ef, or under 10% faster): no recall
change was measured; the speedup is under 10% at the adapter's `rerank = ef`, so the change
does not pay at that setting, and reaches 9-17% at `rerank = ef / 2` and `ef / 4`. The
Linux paired run under the timing lock on writerslogic-02 was not made (host offline), and the
M4 spread between pairs is wide. Raw values, loads and commands are in
[`streaming_rescore_budget_v1.json`](../../benchmarks/results/streaming_rescore_budget_v1.json).

### Consolidation memory peak (not measured)

`consolidation_live_set_peak` now takes `WARM_CONSOLIDATION_DEGREE`, rows up to 10M, and a
two-stage mode (`WARM_CONSOLIDATION_STAGE=build` writes the base, deletions and delta to
`WARM_CONSOLIDATION_INDEX`; `=measure` opens a copy and consolidates it), and prints cgroup
`memory.current` and `memory.peak` at every phase, resetting `memory.peak` before the
consolidation. No R32 or R64 peak at about 8.4M live rows was recorded: writerslogic-02 went
offline during the index builds. No consolidation code was changed. The commands to run
when it is back, per degree D in 32 and 64 (build on cores 8-21, no lock needed):

```sh
WARM_CONSOLIDATION_STAGE=build WARM_CONSOLIDATION_ROWS=8000000 WARM_CONSOLIDATION_DEGREE=$D \
  WARM_CONSOLIDATION_INDEX=~/idx-r$D RAYON_NUM_THREADS=12 taskset -c 8-21 \
  <test-executable> consolidation_live_set_peak --ignored --nocapture --test-threads=1
```

then, per measured run, copy the directory, evict its pages from the page cache, and hold the
lock:

```sh
cp -r ~/idx-r$D ~/idx-run && python3 -I -c 'import os,sys; [os.posix_fadvise(os.open(os.path.join(d,f),os.O_RDONLY),0,0,os.POSIX_FADV_DONTNEED) for d,_,fs in os.walk(sys.argv[1]) for f in fs]' ~/idx-run
flock ~/hms-timing.lock systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 \
  env WARM_CONSOLIDATION_STAGE=measure WARM_CONSOLIDATION_ROWS=8000000 WARM_CONSOLIDATION_DEGREE=$D \
  WARM_CONSOLIDATION_INDEX=~/idx-run RAYON_NUM_THREADS=8 taskset -c 0-7 /usr/bin/time -v \
  <test-executable> consolidation_live_set_peak --ignored --nocapture --test-threads=1
```

The result lines are `consolidation_cgroup_peak_bytes`, `consolidation_peak_rss_kib`,
`consolidation_ns` and `vertex_crc`.

### Still unmeasured (host offline)

* The Linux paired run of the rescore change: build `base_deleted_search_live_set` for the
  parent plus fixture and the candidate, create the 1M index once
  (`WARM_BASE_STAGE=build`), then run parent/candidate, candidate/parent, parent/candidate under
  `flock ~/hms-timing.lock taskset -c 0-7` with the environment of the JSON's
  `development_command`.
* Degree 64 against the 7 GiB bar and the ef that reaches recall@10 0.935 on the 1M and 10M
  prefix replays: the consolidation commands above for D=32 and D=64, then
  `~/bigann/replay2.py 64 64 1.0 <efs> <tag> <maxsteps> 8` and the 10M replay with this
  revision's wheel. The 10M R64 L64 replay at 082766e already scored 0.9353 at ef 256 (table
  above); nothing at this revision has been replayed.
