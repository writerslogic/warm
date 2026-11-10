# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

import argparse
import hashlib
import json
import time
from pathlib import Path

import numpy as np


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--base", required=True, type=Path)
    p.add_argument("--queries", required=True, type=Path)
    p.add_argument("--output", required=True, type=Path)
    args = p.parse_args()
    started = time.monotonic()
    result = {"hypothesis": "H-B-3", "rung": 2, "claim": "local optimistic cover counter"}
    stage = "input"

    def deadline():
        if time.monotonic() - started > 600:
            raise TimeoutError("600-second work cap exceeded")

    def digest(path):
        h = hashlib.sha256()
        with path.open("rb") as f:
            for chunk in iter(lambda: f.read(1024 * 1024), b""):
                deadline()
                h.update(chunk)
        return h.hexdigest()

    def assign(x, centers):
        deadline()
        distance = np.square(x).sum(axis=1)[:, None] + np.square(centers).sum(axis=1)
        distance -= 2 * (x @ centers.T)
        return np.argmin(distance, axis=1)

    try:
        base = np.load(args.base, mmap_mode="r", allow_pickle=False)
        queries = np.load(args.queries, mmap_mode="r", allow_pickle=False)
        if (base.ndim != 2 or queries.ndim != 2 or base.shape[1] != queries.shape[1]
                or not 8192 <= len(base) <= 1_000_000 or len(queries) < 16
                or not 1 <= base.shape[1] <= 128
                or base.dtype != np.float32 or queries.dtype != np.float32):
            raise ValueError("requires 8192..1M float32 base rows, dimension1..128, >=16 matching queries")
        n, d = base.shape
        block_rows = 4096
        sample = np.asarray(base[np.linspace(0, n - 1, 8192, dtype=np.int64)], dtype=np.float64)
        if not np.isfinite(sample).all() or not np.isfinite(queries[:16]).all():
            raise ValueError("nonfinite training sample or query")
        stage = "coarse-training"
        coarse = sample[np.linspace(0, len(sample) - 1, 64, dtype=np.int64)].copy()
        for _ in range(3):
            labels = assign(sample, coarse)
            for j in range(64):
                members = sample[labels == j]
                if len(members):
                    coarse[j] = members.mean(axis=0)
        labels = assign(sample, coarse)
        centers = []
        child_ranges = []
        for j in range(64):
            members = sample[labels == j]
            if not len(members):
                members = coarse[j:j + 1]
            child = members[np.linspace(0, len(members) - 1, min(78, len(members)), dtype=np.int64)].copy()
            child_labels = assign(members, child)
            for k in range(len(child)):
                hit = members[child_labels == k]
                if len(hit):
                    child[k] = hit.mean(axis=0)
            child_ranges.append((len(centers), len(centers) + len(child)))
            centers.extend(child)
        centers = np.asarray(centers)
        sub_starts = np.arange(0, d, 4)
        radii = np.zeros((len(centers), len(sub_starts)))
        full_radii = np.zeros(len(centers))
        counts = np.zeros(len(centers), dtype=np.int64)
        leaf_of = np.empty(n, dtype=np.uint32)
        stage = "assignment-and-covers"
        for start in range(0, n, block_rows):
            deadline()
            x = np.asarray(base[start:start + block_rows], dtype=np.float64)
            if not np.isfinite(x).all():
                raise ValueError("nonfinite base coordinate")
            coarse_labels = assign(x, coarse)
            leaves = np.empty(len(x), dtype=np.int64)
            for j in np.unique(coarse_labels):
                hit = coarse_labels == j
                lo, hi = child_ranges[j]
                leaves[hit] = lo + assign(x[hit], centers[lo:hi])
            leaf_of[start:start + len(x)] = leaves
            residual2 = np.square(x - centers[leaves])
            sub_norm = np.sqrt(np.add.reduceat(residual2, sub_starts, axis=1))
            full_norm = np.sqrt(residual2.sum(axis=1))
            np.maximum.at(radii, leaves, sub_norm)
            np.maximum.at(full_radii, leaves, full_norm)
            np.add.at(counts, leaves, 1)
        eps = np.finfo(np.float64).eps
        radii += 128 * eps * (d + 1) * np.maximum(1, radii)
        full_radii += 128 * eps * (d + 1) * np.maximum(1, full_radii)
        nonempty = counts > 0
        survivor_rows = []
        survivor_cells = []
        full_survivor_rows = []
        sub_survivor_rows = []
        false_prunes = 0
        bound_violations = 0
        stage = "oracle-and-bounds"
        for raw_q in queries[:16]:
            deadline()
            q = np.asarray(raw_q, dtype=np.float64)
            exact = np.empty(n, dtype=np.float64)
            for start in range(0, n, block_rows):
                deadline()
                x = np.asarray(base[start:start + block_rows], dtype=np.float64)
                exact[start:start + len(x)] = np.square(x - q).sum(axis=1)
            top = np.argpartition(exact, 9)[:10]
            threshold = exact[top].max()
            qc2 = np.square(centers - q)
            full = np.square(np.maximum(np.sqrt(qc2.sum(axis=1)) - full_radii, 0))
            sub = np.square(np.maximum(np.sqrt(np.add.reduceat(qc2, sub_starts, axis=1)) - radii, 0)).sum(axis=1)
            bound = np.maximum(full, sub)
            margin = 256 * eps * (d + 1) * max(1.0, threshold, float(exact.max()))
            keep = bound <= threshold + margin
            survivor_rows.append(int(counts[keep].sum()))
            survivor_cells.append(int((keep & nonempty).sum()))
            full_survivor_rows.append(int(counts[full <= threshold + margin].sum()))
            sub_survivor_rows.append(int(counts[sub <= threshold + margin].sum()))
            false_prunes += int((~keep[leaf_of[top]]).sum())
            bound_violations += int((bound[leaf_of] > exact + margin).sum())
        mean_rows = float(np.mean(survivor_rows))
        p99_rows = float(np.quantile(survivor_rows, .99))
        result.update({
            "decision": "accept-next-rung" if mean_rows <= 1000 and p99_rows <= 5000 and len(centers) <= 5000 and not false_prunes and not bound_violations else "reject-this-cover-configuration",
            "base": {"path": str(args.base.resolve()), "sha256": digest(args.base), "rows": n, "dimension": d},
            "queries": {"path": str(args.queries.resolve()), "sha256": digest(args.queries), "prefix_count": 16},
            "partition_policy": {"training_rows": 8192, "coarse_centers": 64, "coarse_iterations": 3, "max_children": 78, "child_iterations": 1, "subspace_dimensions": 4},
            "leaf_centers": len(centers), "nonempty_leaves": int(nonempty.sum()),
            "mean_surviving_rows": mean_rows, "sample_p99_surviving_rows": p99_rows,
            "mean_surviving_cells": float(np.mean(survivor_cells)),
            "full_ball_mean_rows": float(np.mean(full_survivor_rows)),
            "subspace_cover_mean_rows": float(np.mean(sub_survivor_rows)),
            "false_prunes": false_prunes, "bound_violations": bound_violations,
            "cover_bytes_float64": centers.nbytes + radii.nbytes + full_radii.nbytes + counts.nbytes,
            "raw_live_bytes": n * d * 4,
            "resource_scope": "counter only; final runbook max_live10292043 *100D*4B=4116817200 raw bytes; full30M source=12GB must not remain resident; all other index/harness state still unmeasured",
            "thresholds": {"mean_rows_max": 1000, "sample_p99_rows_max": 5000, "centers_max": 5000, "seconds_cap": 600},
            "limits": "oracle thresholds optimistic; float64 numerical checks are not formal certification; no timing/resource feasibility or benchmark score claim; no synthetic generator labels used",
        })
    except Exception as error:
        result.update({"decision": "failed-no-measurement", "failure_type": type(error).__name__, "failure": str(error), "stage": stage})
        raise
    finally:
        result["counter_wall_seconds_not_benchmark"] = time.monotonic() - started
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps({k: result[k] for k in ["decision", "mean_surviving_rows", "sample_p99_surviving_rows", "false_prunes", "bound_violations", "failure_type"] if k in result}))


if __name__ == "__main__":
    main()
