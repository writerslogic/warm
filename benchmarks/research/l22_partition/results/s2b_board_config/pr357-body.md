**Draft.** The results below were measured on an Ice Lake GCP VM of the evaluation VM's class, not on the Azure evaluation VM, so they are not board results.

Adds `neurips23/streaming/warm` (algorithm key `warm`). Source: https://github.com/writerslogic/warm, pinned to commit `459a54031a7293be86227c62755d6b82de81dd2f` (Apache-2.0) by the single `ARG WARM_COMMIT` in `neurips23/streaming/warm/Dockerfile`. It is built with `--features python,experimental` on a pinned Rust 1.99.0 toolchain.

Files: `Dockerfile`, `config.yaml`, `warm.py` (`Warm(BaseStreamingANN)`), `streaming_io.py`, and the CI entry (`warm` on `random-xs`).

## Index

For msturing-30M-clustered, `config.yaml` selects `"mode": "partition"`, which uses `writerslogic_warm.PartitionIndex`:
- Until 1,000,000 vectors are live, search is exact brute force.
- At the first search past that point, the index trains an IVF with 5,000 lists, SOAR spill and PQ-50 residual codes. The centres then stay fixed.
- Search probes **400** lists with a 4-bit LUT scan. On AVX-512 VBMI+VNNI CPUs, four queries share each code block. The threshold test is fused into the scan. Search then reorders **1,000** candidates exactly in f32.

The graph mode (`R`/`L`/`alpha`, `UpdatableIndex`) stays available for the other datasets.

Dataset override: importing `warm.py` replaces the harness's `msturing-10M-clustered` and `msturing-30M-clustered` dataset classes with subclasses whose `get_data_in_range` reads only the requested rows of the `.fbin` (after strict header and size checks). The organisers may drop `streaming_io.py` and the `DATASETS.update` call without affecting the index.

## Measured (not the evaluation VM)

Command: `python run.py --nodocker --neurips23track streaming --algorithm warm --dataset msturing-30M-clustered --runbook_path neurips23/runbooks/final_runbook.yaml`, on a GCP n2-custom-8-16384:
- Intel Xeon @ 2.60 GHz, Ice Lake, 4 cores / 8 threads, 16 GiB, AVX-512 VBMI/VNNI;
- an 8 GiB systemd scope with no swap.

The harness was at this branch with `config.yaml` set as listed; each run is one measurement.

| WARM build | nprobe / reorder | mean recall@10 | min after training | total of step times | elapsed | peak RSS |
|---|---|---|---|---|---|---|
| `e29b62fc` (before the scan change) | 300 / 317 | 0.9726 | 0.9543 | 5,618 s | 1:33:40 | 6.41 GiB |
| `5bba104f` (scan change) | 300 / 317 | 0.9726 (identical) | 0.9543 | 2,187 s | 36:28 | 6.47 GiB |
| `5bba104f` | 350 / 1000 | 0.9808 | 0.9627 | 2,466 s | 41:08 | 6.58 GiB |
| **`5bba104f`, this config** | **400 / 1000** | **0.9848** | **0.9686** | **2,666 s** (search 2,318, insert 311, delete 37) | **44:27** | **6.56 GiB** |

Notes:
- All 1,280 steps completed, with exit 0, in every run.
- `459a5403` is `5bba104f` merged to main. It adds only dead-code lint attributes, and the scan code is the same.
- For reference, `neurips23/README.md` quotes DiskANN at 0.883 on this runbook.
- Wall time on D8lds_v5 (Xeon 8370C, 2.8 GHz) is unmeasured.

## CI

Fork CI on the previous head (https://github.com/dcondrey/big-ann-benchmarks/actions/runs/42456893463):
- The `neurips23-streaming-warm` image builds and `Warm` instantiates.
- The random-xs step then fails with `FileNotFoundError: 'neurips23/streaming/simple_runbook.yaml'`, the harness's default runbook path in `benchmark/runner.py`.
- Every streaming entry in that run fails the same way (diskann, puck, pyanns, cufe, hwtl_sdu_anns_stream).

## Not checked

- Any run on the Azure evaluation VM.
- A Docker build of this head.
