**Draft.** The msturing-30M-clustered result below was measured on a workstation, not on the Azure evaluation VM, so it is not a board result.

Adds `neurips23/streaming/warm` (algorithm key `warm`). Source: https://github.com/writerslogic/warm, pinned to commit `e29b62fc53c1e5f652520e6f63288bc53796324f` (Apache-2.0) by the single `ARG WARM_COMMIT` in `neurips23/streaming/warm/Dockerfile`, built with `--features python,experimental` on a pinned Rust 1.99.0 toolchain.

Files: `Dockerfile`, `config.yaml`, `warm.py` (`Warm(BaseStreamingANN)`), `streaming_io.py`, and the CI entry (`warm` on `random-xs`).

## Index

For msturing-30M-clustered, `config.yaml` selects `"mode": "partition"` (`writerslogic_warm.PartitionIndex`):
- Until 1,000,000 vectors are live, search is exact brute force.
- At the first search past that point, the index trains an IVF with 5,000 lists, SOAR spill and PQ-50 residual codes. The centres are then fixed; inserts are assigned and deletes are tombstoned.
- Search probes 300 lists with a 4-bit LUT scan, then reorders 317 candidates exactly in f32.

The graph mode (`R`/`L`/`alpha`, `UpdatableIndex`) stays available for the other datasets.

Dataset override: importing `warm.py` replaces the harness's `msturing-10M-clustered` and `msturing-30M-clustered` dataset classes with subclasses whose `get_data_in_range` reads only the requested rows from the `.fbin` file (after strict header and size checks) instead of memory-mapping it. Result paths are unchanged. The organisers may drop `streaming_io.py` and the `DATASETS.update` call without affecting the index.

## Measured (not the evaluation VM)

The run was `python run.py --nodocker --neurips23track streaming --algorithm warm --dataset msturing-30M-clustered --runbook_path neurips23/runbooks/streaming/msturing-30M-clustered_final_runbook.yaml`. Conditions:
- harness at this branch's parent (5d6225a);
- 8 CPUs (Intel, WSL2), under a systemd scope with an 8 GiB memory limit;
- the wheel built from a `git archive` of the pinned commit.

| quantity | value |
|---|---|
| steps | 1,280 of 1,280 completed, exit 0, 640 searches |
| mean recall@10 | **0.9726** |
| minimum recall@10 after training | 0.9543 (training happened at search 16, with 1,239,816 vectors live) |
| total of the step times | 2,550 s (insert 160 s, delete 36 s, search 2,353 s); process elapsed 42:31 |
| peak RSS | 6.45 GiB |

An earlier run of the same tier at nprobe 250 scored 0.9651 mean recall in 2,276 s.

For reference, the `neurips23/README.md` baselines table quotes DiskANN at 0.883 on this runbook. This hardware is not the Azure D8lds_v5 VM, so wall time on that VM is unmeasured. Wall time and memory on that VM decide whether this entry qualifies.

## CI

Fork CI on this head (https://github.com/dcondrey/big-ann-benchmarks/actions/runs/42456893463):
- The `neurips23-streaming-warm` image builds, and `Warm` instantiates.
- The random-xs step then fails with `FileNotFoundError: 'neurips23/streaming/simple_runbook.yaml'`, raised by the harness's default runbook path in `benchmark/runner.py`.
- Every streaming entry in that run fails the same way (diskann, puck, pyanns, cufe, hwtl_sdu_anns_stream), so this is not specific to WARM.

## Not checked

- Any run on the evaluation VM.
