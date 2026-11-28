# VIBE llama and yi with every competitor on its full official grid, 2026-11-28

Objection: in `vibe_same_host_20261127` the competitors ran 2 configs each, so a better-tuned RoarGraph or MLANN might beat WARM. This run gives every competitor its full official VIBE grid and WARM its full committed adapter grid, on one VM, through VIBE's own `run.py --local`.

**What this is and is not.** It is a single-block full-grid screen. It is not the 3-block, min-recall / median-QPS protocol of #544: the full grids took about 4.2 hours, which left no time for confirmation blocks. Each cell below is the fastest config whose strict-ID recall reaches the target in that one run. Selecting the maximum over hundreds of noisy single-run configs favours every system alike (winner's curse), and the competitors have the larger grids, so the selection bias favours them, not WARM. No recall minimum or QPS spread over blocks exists here.

## Setup

- Host: GCP `warm-vibesh-1`, n2-custom-8-16384 (Intel Xeon 2.60 GHz Ice Lake, AVX-512), us-central1-a, the #544 host (`evidence/lscpu.txt`). Nothing else ran on it.
- VIBE `dcondrey/vibe` at the #544 pin; WARM is the `dbe04ad6` wheel with its adapter (`warm-541` in #544); RoarGraph `matchyc/RoarGraph@f2b49b6`, MLANN `ejaasaari/mlann@de8f9d6`, as in #544.
- Grids (`evidence/defs/`): RoarGraph `M_sq` 100, `M_pjbp` 16, 24, 32, 48, 64, 96, `L_pjpq` 500, 18 `L_pq` values (6 builds, 108 configs per dataset). MLANN-rf small group `n_trees` 10, 15, 20 x depth 14 to 18, big group `n_trees` 80 x depth 13 to 16, 20 voting thresholds each. WARM: the full adapter grid for inner product (6 builds, 296 configs per dataset).
- 3 runs per config (`--runs 3`); VIBE stores the neighbours and times of the last run; QPS = 1 / mean stored per-query time. Strict ID recall against float64 inner-product truth over the full corpus (`evidence/extract2.py`, as in #544).
- **Three workers ran at once** (`--parallelism 3`, each pinned to its own CPU, orchestrator on a fourth). This is what made the grids fit; it is not the isolated single-worker condition of #544, so absolute QPS and build times are not comparable with that directory. Systems ran one after another, so every system had the same three-lane contention.
- Peak memory is the kernel peak RSS (`VmHWM`, sampled every 0.25 s by `evidence/rss_sampler.py`) of the worker process that built the index and ran its queries, so it includes dataset load. A worker is attached to its build by the closeness of its exit time to the result file's last write; the mapping is by timing, not by id, so treat per-build RSS as approximate.
- Build times are VIBE's `build_time` for the build; with three workers contending they are inflated against an isolated build.

## Result: fastest config at each recall target, strict IDs (`targets.csv`)

| dataset | target | WARM | RoarGraph | MLANN-rf | WARM / best competitor |
|---|---|---|---|---|---|
| llama | 0.90 | 15,434 | 5,763 | 9,414 | 1.64 |
| llama | 0.95 | 10,193 | 4,029 | 7,450 | 1.37 |
| llama | 0.99 | 4,818 | 758 | 3,623 | 1.33 |
| yi | 0.90 | 8,328 | 3,233 | 4,877 | 1.71 |
| yi | 0.95 | 5,578 | 2,025 | 2,758 | 2.02 |
| yi | 0.99 | 2,753 | not reached | 762 | 3.61 |

WARM wins all six cells. The smallest margin is 1.33x (llama 0.99 against MLANN-rf), where it is also the closest in absolute terms; those two numbers come from single runs, so that margin could shrink or grow in a confirmation block.

Build time and peak RSS of the winning build per cell (`targets.csv` has the config strings):

| dataset | system | 0.90 build s / GiB | 0.95 build s / GiB | 0.99 build s / GiB |
|---|---|---|---|---|
| llama | WARM | 166 / 0.85 | 166 / 0.85 | 413 / 1.13 |
| llama | RoarGraph | 216 / 1.07 | 394 / 1.34 | 394 / 1.34 |
| llama | MLANN-rf | 52 / 1.27 | 68 / 1.49 | 103 / 2.02 |
| yi | WARM | 95 / 0.69 | 290 / 1.01 | 290 / 1.01 |
| yi | RoarGraph | 218 / 0.90 | 348 / 1.03 | not reached |
| yi | MLANN-rf | 123 / 1.91 | 67 / 1.54 | 9,899 / 5.39 |

Losses, plainly: MLANN-rf builds faster than WARM in every llama cell (52 to 103 s against 166 to 413 s) and at yi 0.95 (67 s against 290 s); WARM builds faster at yi 0.90 and 0.99. WARM's winning build has the lowest peak RSS against MLANN-rf in all 6 cells and against RoarGraph in all 5 cells where RoarGraph reaches the target, but RSS here includes dataset load and is matched by timing. At yi 0.99 MLANN-rf needs an 80-tree build of 9,899 s.

## Coverage gaps

- MLANN rejects depth 18 on both corpora and depth 16 with 80 trees on yi ("The depth must belong to the set {1, ... , min(log2(n_train), 29)}", in `evidence/logs/mlann-*.run.log.gz`), so 16 of 19 builds ran on llama and 15 of 19 on yi. The unrun builds are ones MLANN itself refuses at this corpus size.
- RoarGraph does not reach 0.99 on yi in any of its 108 configs.
- WARM is shown without `warm-pre541`; only the current wheel ran.
- One dataset pair, k=100, 1000 queries, one VM, one block, 3 concurrent workers.

## Files

`screening.csv`: every config (system, dataset, build file, query group, strict-ID recall, QPS, build time, peak RSS). `targets.csv`: the cells above. `aggregate.py` builds both from `evidence/` (`rawA.csv` from `extract2.py`, `rssA.csv`, `mtimes.txt`). `evidence/`: driver and run scripts, grid definitions, per-run start and end times (`runs.txt`, `driverA.log`), redacted VIBE logs. VIBE result HDF5 files stay on the VM disk.
