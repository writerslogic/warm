# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

import argparse
import hashlib
import json
import time
from pathlib import Path

import numpy as np


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True, type=Path)
    parser.add_argument("--queries", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    started = time.monotonic()
    result = {"hypothesis": "H-B-5", "rung": 2, "claim": "local non-oracle serving counter; no timing claim"}
    stage = "input"

    def deadline():
        if time.monotonic() - started > 600:
            raise TimeoutError("600-second work cap exceeded")

    def digest(path):
        h = hashlib.sha256()
        with path.open("rb") as f:
            for block in iter(lambda: f.read(1024 * 1024), b""):
                deadline()
                h.update(block)
        return h.hexdigest()

    def quantize(x):
        scale = np.maximum((np.max(np.abs(x), axis=1) / 127).astype(np.float32), np.finfo(np.float32).tiny).astype(np.float64)
        code = np.clip(np.rint(x / scale[:, None]), -127, 127).astype(np.int8)
        reconstructed = code * scale[:, None]
        error = np.sqrt(np.square(x - reconstructed).sum(axis=1))
        error = np.nextafter(error.astype(np.float32), np.float32(np.inf)).astype(np.float64)
        norm2 = np.square(reconstructed).sum(axis=1)
        if not np.isfinite(error).all() or not np.isfinite(norm2).all():
            raise ValueError("nonfinite reconstruction metadata")
        return code, scale, error, norm2

    def top10(distances, ids):
        order = np.lexsort((ids, distances))[:10]
        return distances[order], ids[order]

    try:
        base = np.load(args.base, mmap_mode="r", allow_pickle=False)
        queries = np.load(args.queries, mmap_mode="r", allow_pickle=False)
        if (base.ndim != 2 or queries.ndim != 2 or base.shape[1] != queries.shape[1]
                or not 4096 <= len(base) <= 1_000_000 or len(queries) < 16
                or not 1 <= base.shape[1] <= 128
                or base.dtype != np.float32 or queries.dtype != np.float32):
            raise ValueError("requires4096..1M float32 base rows,dimension1..128,and16 matching queries")
        n, d = base.shape
        block_rows = 4096
        codes = np.empty((n, d), dtype=np.int8)
        scales, errors, norms2 = (np.empty(n, dtype=np.float64) for _ in range(3))
        max_norm = 0.0
        stage = "quantization"
        for start in range(0, n, block_rows):
            deadline()
            x = np.asarray(base[start:start + block_rows], dtype=np.float64)
            if not np.isfinite(x).all():
                raise ValueError("nonfinite source coordinate")
            stop = start + len(x)
            codes[start:stop], scales[start:stop], errors[start:stop], norms2[start:stop] = quantize(x)
            max_norm = max(max_norm, float(np.sqrt(np.square(x).sum(axis=1)).max()))
        stage = "data-only-seed-selection"
        hashes = np.arange(n, dtype=np.uint64) + np.uint64(0x9E3779B97F4A7C15)
        hashes = (hashes ^ (hashes >> 30)) * np.uint64(0xBF58476D1CE4E5B9)
        hashes = (hashes ^ (hashes >> 27)) * np.uint64(0x94D049BB133111EB)
        hashes ^= hashes >> 31
        seed_ids = np.sort(np.argpartition(hashes, 4095)[:4096])
        del hashes
        seeded = np.zeros(n, dtype=bool)
        seeded[seed_ids] = True
        seed_rows = np.asarray(base[seed_ids], dtype=np.float64)
        refinements, totals, oracle_counts, threshold_ratios = [], [], [], []
        violations = false_prunes = mismatches = zero_final_thresholds = correction_mismatches = 0
        stage = "seed-and-refine"
        for raw_q in queries[:16]:
            deadline()
            q = np.asarray(raw_q, dtype=np.float64)
            if not np.isfinite(q).all():
                raise ValueError("nonfinite query coordinate")
            qc, qs, qe, qn = quantize(q[None, :])
            qcode = qc[0].astype(np.int64)
            query_correction = 128 * qcode.sum()
            best_d, best_ids = top10(np.square(seed_rows - q).sum(axis=1), seed_ids)
            initial_threshold = best_d[-1]
            margin = 256 * np.finfo(np.float64).eps * (d + 1) * max(1.0, (max_norm + float(np.linalg.norm(q))) ** 2)
            lower_all = np.empty(n, dtype=np.float64)
            pruned = np.zeros(n, dtype=bool)
            exact_count = 0
            for start in range(0, n, block_rows):
                deadline()
                stop = min(start + block_rows, n)
                signed = codes[start:stop].astype(np.int64)
                dot = (signed + 128) @ qcode - query_correction
                correction_mismatches += int((dot != signed @ qcode).sum())
                reconstructed2 = norms2[start:stop] + qn[0] - 2 * scales[start:stop] * qs[0] * dot
                lower = np.square(np.maximum(np.sqrt(np.maximum(reconstructed2, 0)) - errors[start:stop] - qe[0], 0))
                lower_all[start:stop] = lower
                keep = lower <= best_d[-1] + margin
                pruned[start:stop] = ~keep
                ids = start + np.flatnonzero(keep & ~seeded[start:stop])
                exact_count += len(ids)
                if len(ids):
                    actual = np.square(np.asarray(base[ids], dtype=np.float64) - q).sum(axis=1)
                    best_d, best_ids = top10(np.concatenate((best_d, actual)), np.concatenate((best_ids, ids)))
            stage = "retrospective-oracle-validation"
            exact = np.empty(n, dtype=np.float64)
            for start in range(0, n, block_rows):
                deadline()
                x = np.asarray(base[start:start + block_rows], dtype=np.float64)
                exact[start:start + len(x)] = np.square(x - q).sum(axis=1)
            oracle_ids = np.argsort(exact, kind="stable")[:10]
            oracle_threshold = exact[oracle_ids[-1]]
            mismatches += int(not np.array_equal(best_ids, oracle_ids) or not np.array_equal(best_d, exact[oracle_ids]))
            violations += int((lower_all > exact + margin).sum())
            false_prunes += int((pruned & (exact <= oracle_threshold + margin)).sum())
            oracle_counts.append(int((lower_all <= oracle_threshold + margin).sum()))
            refinements.append(exact_count)
            totals.append(4096 + exact_count)
            if best_d[-1] > 0:
                threshold_ratios.append(float(initial_threshold / best_d[-1]))
            else:
                zero_final_thresholds += 1
            stage = "seed-and-refine"
        mean_refine = float(np.mean(refinements))
        p99_refine = float(np.quantile(refinements, .99))
        inflation = np.asarray(totals) / np.maximum(oracle_counts, 1)
        refine_inflation = np.asarray(refinements) / np.maximum(oracle_counts, 1)
        result.update({
            "decision": "accept-next-rung" if mean_refine <= 2048 and p99_refine <= 8192 and not violations and not false_prunes and not mismatches and not correction_mismatches else "reject-this-seeding-policy",
            "base": {"path": str(args.base.resolve()), "sha256": digest(args.base), "rows": n, "dimension": d},
            "queries": {"path": str(args.queries.resolve()), "sha256": digest(args.queries), "prefix_count": 16},
            "seed_policy": "4096 smallest SplitMix64 hashes of live row indices; fixed generic source order for4096-row blocks",
            "seed_exact_rows_per_query": 4096, "seed_selection_live_ids_examined": n,
            "mean_additional_exact_rows": mean_refine, "sample_p99_additional_exact_rows": p99_refine,
            "mean_total_exact_rows": float(np.mean(totals)), "sample_p99_total_exact_rows": float(np.quantile(totals, .99)),
            "mean_total_vs_oracle_count_ratio": float(inflation.mean()), "sample_p99_total_vs_oracle_count_ratio": float(np.quantile(inflation, .99)),
            "mean_additional_vs_oracle_count_ratio": float(refine_inflation.mean()),
            "initial_to_final_threshold_mean_ratio": float(np.mean(threshold_ratios)) if threshold_ratios else None,
            "zero_final_threshold_queries": zero_final_thresholds,
            "full_run_seed_row_distances": 640 * 10000 * 4096,
            "projected_full_run_additional_row_distances_from_local_mean": 640 * 10000 * mean_refine,
            "projection_scope": "arithmetic projection only; synthetic refinement rates are not track predictions",
            "false_prunes": false_prunes, "bound_violations": violations, "top10_mismatches": mismatches,
            "signed_correction_mismatches": correction_mismatches,
            "thresholds": {"mean_additional_max": 2048, "sample_p99_additional_max": 8192, "correctness_failures_max": 0, "seconds_cap": 600},
            "unproved": ["formal native floating enclosures", "matched fused throughput including corrections/epilogue", "all runbook memory/update/selection costs", "untouched-query generalization"],
        })
    except Exception as error:
        result.update({"decision": "failed-no-measurement", "failure_type": type(error).__name__, "failure": str(error), "stage": stage})
        raise
    finally:
        result["counter_wall_seconds_not_benchmark"] = time.monotonic() - started
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
        print(json.dumps({k: result[k] for k in ["decision", "mean_additional_exact_rows", "sample_p99_additional_exact_rows", "false_prunes", "bound_violations", "top10_mismatches", "failure_type"] if k in result}))


if __name__ == "__main__":
    main()
