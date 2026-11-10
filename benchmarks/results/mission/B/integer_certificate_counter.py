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
    result = {"hypothesis": "H-B-4", "rung": 2, "claim": "local numerical counter; no performance or track claim"}
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

    try:
        base = np.load(args.base, mmap_mode="r", allow_pickle=False)
        queries = np.load(args.queries, mmap_mode="r", allow_pickle=False)
        if (base.ndim != 2 or queries.ndim != 2 or base.shape[1] != queries.shape[1]
                or not 10 <= len(base) <= 1_000_000 or len(queries) < 16
                or not 1 <= base.shape[1] <= 128
                or base.dtype != np.float32 or queries.dtype != np.float32):
            raise ValueError("requires10..1M float32 base rows,dimension1..128,and16 matching queries")
        n, d = base.shape
        block_rows = 4096
        codes = np.empty((n, d), dtype=np.int8)
        scales = np.empty(n, dtype=np.float64)
        errors = np.empty(n, dtype=np.float64)
        norms2 = np.empty(n, dtype=np.float64)
        stage = "quantization"
        for start in range(0, n, block_rows):
            deadline()
            x = np.asarray(base[start:start + block_rows], dtype=np.float64)
            if not np.isfinite(x).all():
                raise ValueError("nonfinite source coordinate")
            stop = start + len(x)
            codes[start:stop], scales[start:stop], errors[start:stop], norms2[start:stop] = quantize(x)
        survivors = []
        false_prunes = 0
        bound_violations = 0
        exact_dot_max = 0
        stage = "oracle-and-certificate"
        for raw_q in queries[:16]:
            deadline()
            q = np.asarray(raw_q, dtype=np.float64)
            if not np.isfinite(q).all():
                raise ValueError("nonfinite query coordinate")
            qcodes, qscales, qerrors, qnorms2 = quantize(q[None, :])
            qcode = qcodes[0].astype(np.int64)
            qscale, qerror, qnorm2 = qscales[0], qerrors[0], qnorms2[0]
            exact = np.empty(n, dtype=np.float64)
            for start in range(0, n, block_rows):
                deadline()
                x = np.asarray(base[start:start + block_rows], dtype=np.float64)
                exact[start:start + len(x)] = np.square(x - q).sum(axis=1)
            threshold = np.partition(exact, 9)[9]
            margin = 256 * np.finfo(np.float64).eps * (d + 1) * max(1.0, float(exact.max()))
            count = 0
            for start in range(0, n, block_rows):
                deadline()
                stop = min(start + block_rows, n)
                dot = codes[start:stop].astype(np.int64) @ qcode
                exact_dot_max = max(exact_dot_max, int(np.max(np.abs(dot))))
                approximate2 = norms2[start:stop] + qnorm2 - 2 * scales[start:stop] * qscale * dot
                lower = np.square(np.maximum(np.sqrt(np.maximum(approximate2, 0)) - errors[start:stop] - qerror, 0))
                keep = lower <= threshold + margin
                true_distance = exact[start:stop]
                count += int(keep.sum())
                false_prunes += int(((true_distance <= threshold + margin) & ~keep).sum())
                bound_violations += int((lower > true_distance + margin).sum())
            survivors.append(count)
        mean = float(np.mean(survivors))
        p99 = float(np.quantile(survivors, .99))
        result.update({
            "decision": "accept-next-rung" if mean <= 64 and p99 <= 256 and not false_prunes and not bound_violations else "reject-this-integer-certificate",
            "base": {"path": str(args.base.resolve()), "sha256": digest(args.base), "rows": n, "dimension": d},
            "queries": {"path": str(args.queries.resolve()), "sha256": digest(args.queries), "prefix_count": 16},
            "quantization": "symmetric8bit; per-vector maxabs/127 rounded to float32; nearest integer; reconstruction error from source only, rounded upward to float32",
            "mean_surviving_rows": mean, "sample_p99_surviving_rows": p99,
            "max_surviving_rows": max(survivors), "survivor_fraction": sum(survivors) / (n * 16),
            "false_prunes": false_prunes, "bound_violations": bound_violations,
            "maximum_absolute_signed_integer_dot": exact_dot_max,
            "unsigned_signed_vnni_absolute_accumulator_bound": d * 255 * 127,
            "counter_code_and_metadata_bytes": codes.nbytes + scales.nbytes + errors.nbytes + norms2.nbytes,
            "counter_scope": "int64 dot and float64 bound checking, not a fused VNNI implementation; oracle kth threshold is optimistic and forbidden in serving; no cross-checkpoint cached scores",
            "thresholds": {"mean_rows_max": 64, "sample_p99_rows_max": 256, "false_prunes_max": 0, "bound_violations_max": 0, "seconds_cap": 600},
            "unproved": ["formal outward-rounded serving epilogue", "non-oracle threshold inflation", "sustained fused10.713Gpairs/s", "complete8GiB/3600s live workload", "transfer to untouched source queries"],
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
