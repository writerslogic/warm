# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--base", type=Path, required=True)
    p.add_argument("--queries", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--query-count", type=int, default=16)
    p.add_argument("--block-rows", type=int, default=8192)
    args = p.parse_args()
    if not 1 <= args.query_count <= 256 or not 1 <= args.block_rows <= 65536:
        p.error("query-count must be 1..256; block-rows must be 1..65536")
    base = np.load(args.base, mmap_mode="r", allow_pickle=False)
    queries = np.load(args.queries, mmap_mode="r", allow_pickle=False)
    if (base.ndim != 2 or queries.ndim != 2 or base.shape[1] != queries.shape[1]
            or len(base) < 10 or len(queries) < args.query_count
            or not 1 <= base.shape[1] <= 4096
            or base.dtype != np.float32 or queries.dtype != np.float32):
        p.error("expected finite float32 matrices with equal dimension and >=10 base rows")
    n, d = base.shape
    lower = np.full(d, np.inf)
    upper = np.full(d, -np.inf)
    sums = np.zeros(d)
    squares = np.zeros(d)
    for start in range(0, n, args.block_rows):
        x = np.asarray(base[start:start + args.block_rows], dtype=np.float64)
        if not np.isfinite(x).all():
            raise ValueError("base contains nonfinite coordinates")
        lower = np.minimum(lower, x.min(axis=0))
        upper = np.maximum(upper, x.max(axis=0))
        sums += x.sum(axis=0)
        squares += np.square(x).sum(axis=0)
    order = np.argsort(-(squares / n - np.square(sums / n)), kind="stable")
    lower, upper = lower[order], upper[order]
    width = np.maximum((upper - lower) / 16, np.finfo(np.float64).tiny)
    pad = 32 * np.finfo(np.float64).eps * np.maximum(1, np.maximum(abs(lower), abs(upper)))
    total_survivors = 0
    total_dimensions = 0
    false_prunes = 0
    bound_violations = 0
    prefixes = sorted(set([v for v in [1, 2, 4, 8] if v < d] + list(range(16, d, 8)) + [d]))
    prefix_survivors = np.zeros(len(prefixes), dtype=np.int64)
    for raw_q in queries[:args.query_count]:
        q = np.asarray(raw_q, dtype=np.float64)[order]
        if not np.isfinite(q).all():
            raise ValueError("query contains nonfinite coordinates")
        top = np.full(10, np.inf)
        for start in range(0, n, args.block_rows):
            x = np.asarray(base[start:start + args.block_rows], dtype=np.float64)[:, order]
            exact = np.square(x - q).sum(axis=1)
            merged = np.concatenate((top, exact))
            top = np.partition(merged, 9)[:10]
        threshold = top.max()
        margin = 128 * np.finfo(np.float64).eps * (d + 1) * max(1.0, threshold)
        for start in range(0, n, args.block_rows):
            x = np.asarray(base[start:start + args.block_rows], dtype=np.float64)[:, order]
            exact = np.square(x - q).sum(axis=1)
            codes = np.clip(np.floor((x - lower) / width), 0, 15).astype(np.uint8)
            lo = lower + codes * width - pad
            hi = lower + (codes.astype(np.float64) + 1) * width + pad
            if not ((lo <= x) & (x <= hi)).all():
                raise AssertionError("quantized interval does not enclose source coordinate")
            gaps = np.maximum(np.maximum(lo - q, q - hi), 0)
            partial = np.cumsum(np.square(gaps), axis=1)
            alive = np.ones(len(x), dtype=bool)
            previous = 0
            for j, prefix in enumerate(prefixes):
                total_dimensions += int(alive.sum()) * (prefix - previous)
                alive &= partial[:, prefix - 1] <= threshold + margin
                prefix_survivors[j] += alive.sum()
                previous = prefix
            total_survivors += int(alive.sum())
            false_prunes += int(((exact <= threshold + margin) & ~alive).sum())
            bound_violations += int((partial[:, -1] > exact + margin).sum())
    total = n * args.query_count
    mean_bytes = total_dimensions / total / 2
    survivor_fraction = total_survivors / total
    result = {
        "hypothesis": "H-B-2", "rung": 2, "claim": "local synthetic development counter",
        "method": "4-bit scalar interval bound, variance order fitted only to base; oracle kth threshold",
        "numerical_scope": "float64 bounds checked against float64 direct L2; no formal floating certificate or timing claim",
        "base": {"path": str(args.base.resolve()), "sha256": digest(args.base), "rows": n, "dimension": d},
        "queries": {"path": str(args.queries.resolve()), "sha256": digest(args.queries), "prefix_count": args.query_count},
        "survivor_fraction": survivor_fraction,
        "mean_dimensions_touched": total_dimensions / total,
        "ideal_packed_code_bytes_per_base_row_query": mean_bytes,
        "prefixes": prefixes,
        "prefix_survivor_fractions": (prefix_survivors / total).tolist(),
        "false_prunes": false_prunes, "bound_violations": bound_violations,
        "thresholds": {"max_code_bytes": 1.5, "max_survivor_fraction": 0.0003},
        "decision": "accept-next-rung" if mean_bytes <= 1.5 and survivor_fraction <= 0.0003 and not false_prunes and not bound_violations else "reject-this-global-scalar-bound",
        "limits": "oracle thresholds and packed-byte model are optimistic; ignores LUT, IDs, heaps, cache misses, maintenance and actual runtime; synthetic evidence cannot establish track feasibility",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ["decision", "survivor_fraction", "mean_dimensions_touched", "false_prunes", "bound_violations"]}))


if __name__ == "__main__":
    main()
