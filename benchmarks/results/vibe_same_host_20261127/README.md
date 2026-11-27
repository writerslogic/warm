# Same-host VIBE comparison: WARM, RoarGraph, MLANN on llama-128-ip and yi-128-ip

All systems ran on one VM, one session, through VIBE's own `run.py --local` (adapter, dataset loader, per-query timing, result files).

Host: GCP `warm-vibesh-1` in `us-central1-a`, a clone of the `warm-bench-icelake` boot disk (n2-custom-8-16384, Intel Xeon 2.60 GHz Ice Lake with AVX-512 and VNNI, 8 vCPU, 16 GB). Nothing else ran on it during the timed blocks. Hardware class: the committed `query_aware_graph_v1` rows came from writerslogic-02, an Intel Core Ultra 9 185H laptop under WSL2 (AVX2, no AVX-512). No GCP class matches that, so n2 Ice Lake (the class of `vibe_lists_20261126`) was used. Absolute QPS here is therefore not comparable with the writerslogic-02 rows; only the ratios within this directory are claims.

## Protocol

- VIBE: `dcondrey/vibe` at `6e5fb9daf276ecba73d87dcdd6a14448d434dba7` (the pin in `benchmarks/vibe/modal_vibe.py`), data from the HF dataset (llama and yi full corpora, 1000 test queries, k=100), `run.py --local --runs 5 --algorithm <a> --definitions <dir>`. `numactl` is absent, so VIBE's worker pinned itself with taskset to one CPU. Math-library threads are 1; RoarGraph's adapter builds with 1 thread.
- Systems: `warm-541` is origin/main `dbe04ad6` (contains #541) with its adapter `benchmarks/vibe/warm/{module.py,config.yml}`; `warm-pre541` is `a3c3f0ef` (the parent of #541) with its own adapter. Both wheels built with `maturin build --release --locked`, `-C target-cpu=native`, no `mimalloc`. RoarGraph is `matchyc/RoarGraph@f2b49b6` (add_pyroar, built `-march=native`), MLANN is `ejaasaari/mlann@de8f9d6` (built `-march=native`, AVX-512 flags), both as in `modal_vibe.py`.
- Competitor configs: roargraph48 and roargraph96 (`M_pjbp` 48 and 96, `M_sq` 100, `L_pjpq` 500, all 18 `L_pq` values of the VIBE grid); mlann_rf10_depth14 and mlann_rf80_depth13 (all 20 voting thresholds). Definitions are in `evidence/defs/`.
- 3 blocks. Each block runs every system on yi then llama, strictly serially, and the system order rotates (block 1 new, base, roar, mlann; block 2 mlann, roar, base, new; block 3 base, new, mlann, roar). Order is in `evidence/driver.sh` (block 1 roar/yi was started by hand with the same `run_one.sh` before the driver) and times in `evidence/driver.txt`.
- Per block and config: VIBE runs 5 passes and stores the neighbours and per-query times of the last (fifth) pass. QPS = 1 / mean(stored per-query time). Strict neighbour-ID recall is computed from those stored IDs against float64 inner-product truth over the full corpus (descending score, base-id ties). VIBE's own HDF5 `neighbors` give the same recall in every row (`recall_h5` in `raw_per_block.csv`).
- Reported per config: recall = minimum over the 3 blocks, QPS = median over the 3 blocks, with min and max. Each cell below is the fastest config whose minimum recall meets the target (no interpolation).
- Indexes are rebuilt in every block (VIBE has no build cache), so build variance is inside the repetitions.

## QPS at recall 0.90, 0.95, 0.99 (median [min-max] over 3 blocks, one thread)

**llama-128-ip**

| recall | warm-541 | warm-pre541 | roargraph48 | roargraph96 | mlann_rf10_depth14 | mlann_rf80_depth13 | winner |
|---|---:|---:|---:|---:|---:|---:|---|
| 0.90 | 16,691 [16,453-16,836] | 4,832 [4,795-4,854] | 6,511 [6,432-6,560] | 4,360 [4,312-4,369] | 8,438 [8,398-8,658] | 3,204 [3,166-3,267] | warm-541 (1.98x best competitor) |
| 0.95 | 11,314 [11,291-11,326] | 4,832 [4,795-4,854] | 2,331 [2,250-2,374] | 4,360 [4,312-4,369] | 7,901 [7,868-8,022] | 3,034 [2,966-3,041] | warm-541 (1.43x best competitor) |
| 0.99 | 4,884 [4,838-5,004] | 2,337 [2,330-2,338] | unreached | 894 [886-921] | unreached | 2,421 [2,314-2,454] | warm-541 (2.02x best competitor) |

**yi-128-ip**

| recall | warm-541 | warm-pre541 | roargraph48 | roargraph96 | mlann_rf10_depth14 | mlann_rf80_depth13 | winner |
|---|---:|---:|---:|---:|---:|---:|---|
| 0.90 | 8,550 [8,442-8,567] | 5,804 [5,693-5,849] | 3,779 [3,721-3,796] | 3,276 [3,022-3,325] | 5,079 [4,869-5,124] | 2,416 [2,378-2,452] | warm-541 (1.68x best competitor) |
| 0.95 | 5,701 [5,683-5,923] | 4,038 [4,013-4,072] | 1,109 [1,099-1,127] | 2,560 [2,560-2,607] | unreached | 1,807 [1,779-1,821] | warm-541 (2.23x best competitor) |
| 0.99 | 2,901 [2,633-2,915] | 1,266 [1,265-1,286] | unreached | unreached | unreached | 893 [850-894] | warm-541 (3.25x best competitor) |

Configs behind each cell are in `targets.csv`. `warm-541` is the fastest system in all six cells, and its min-max range does not overlap any competitor's range in any cell.

## Losses and limits, stated plainly

- No cell in this table is lost to RoarGraph or MLANN. Where a competitor has no row, it did not reach the recall within its configured grid ("unreached"), which is a grid fact, not a statement that no setting could.
- Competitors were run on the configurations named in the task, not on VIBE's full tuning grids (RoarGraph other `M_pjbp`, MLANN other tree counts and depths). A better competitor build may exist.
- The WARM grid was trimmed to fit the time budget, identically for both commits: ip `base` group degree 32 only, `ef` in {150, 200, 250, 300, 400, 650, 1000}, `rerank` in {150, 200, 400}; `learn_mid`/`learn_tail` dropped; `lists_small` and `lists_large` unchanged (`evidence/config-*-run.yml`, full grid kept as `evidence/config-new-full.yml`). The first block of `warm-541` on yi ran the full committed grid; only configs present in all 3 blocks are reported.
- `warm-pre541` rows at 0.90 and 0.95 are limited by that grid: the `ef` floor of 150 excludes the `ef` 100 to 125 rows that `vibe_lists_20261126` used (6,958 and 5,871 QPS on llama). The llama 0.90 and 0.95 cells show the same ef=150 row (4,832, recall 0.9648). Use the `vibe_lists_20261126` baseline for the pre-#541 0.90 and 0.95 figures; the 0.99 baselines here (2,337 llama, 1,266 yi) match it (2,360 and 1,225).
- `warm-541` yi at 0.99 is 2,901 here against 3,218 in `vibe_lists_20261126`: the adapter grid lists `ef` 300 only for the list groups, and the earlier row used `ef` 150, which is not in the committed adapter config. The wheel here also lacks `mimalloc`. Against `vibe_lists_20261126` the other `warm-541` cells are llama 0.90 -4.0 percent (16,691 vs 17,393), llama 0.95 -3.2 percent, llama 0.99 +1.2 percent, yi 0.90 -1.9 percent, yi 0.95 -5.4 percent (best rows differ because the adapter grid is narrower).
- Cross-host: the committed RoarGraph (6,363 at 0.957) and MLANN (3,221 at 0.990) rows from writerslogic-02 differ from the numbers here (roargraph96 llama 0.95: 4,360; mlann_rf80 llama 0.99: 2,421), as expected from different hardware. Do not mix them.
- One VM, one session, 3 blocks; the claim covers these two datasets, k=100, one thread, 1000 test queries.
- No figure: `query_aware_graph_v1/generate_report.py` is bound to that run's schema.

## Files

- `targets.csv`: the table above (also with config strings). `pareto.csv`: per system and dataset non-dominated points in the `query_aware_graph_v1/pareto.csv` format (`phase` is `same-host`). `full.csv`: every config present in all 3 blocks.
- `raw_per_block.csv`: one row per block, system, dataset and config (`qps_last`, VIBE `best_qps`, recall by both truths, build time). `aggregate.py` builds the three CSVs from it.
- `evidence/`: per-run `start`/`end`/`exit` and gzipped VIBE logs, driver and run scripts, configs, `lscpu.txt`, source revisions, compile flags. Raw result HDF5 files stay on the VM disk and are not committed.
