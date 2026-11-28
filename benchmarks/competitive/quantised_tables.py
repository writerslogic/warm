# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Markdown tables of WARM over every build of every comparator in a `warm-bench run` heavy-ann file, per recall target.

Factor = median-over-rounds QPS ratio of WARM's best operating point over the build's best operating point (grid), and the
same on log-QPS-interpolated curves, each with the paired-by-round bootstrap interval of `ann_mechanism.factor`.
usage: quantised_tables.py FILE [FILE ...] > tables.md
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from ann_mechanism import TARGETS, factor, interpolated, operating_point, pick  # noqa: E402


def best_point(rounds, target):
    found, _ = pick(rounds, target)
    return operating_point(rounds, target) if found else {"reached": False, "best_select_recall": pick(rounds, target)[1][0]}


def cell(f):
    if not f["reached"]:
        return "not reached"
    lo, hi = f["ci95"]
    return f"{f['factor']:.2f} [{lo:.2f}, {hi:.2f}]"


def main():
    for path in sys.argv[1:]:
        r = json.loads(Path(path).read_text())
        warm = r["systems"]["warm"]["builds"]
        warm_rounds = {b: v["serve_rounds"] for b, v in warm.items()}
        print(f"\n### {r['hardware']['class']}, {r['dataset']['name']} n={r['dataset']['n_train']}, {r['protocol']['rounds']} rounds, commit {r['commit'][:8]}\n")
        print("| recall | comparator | build | traversal | their QPS | their params | recall (report half) | WARM / theirs, grid | WARM / theirs, interpolated |")
        print("|---|---|---|---|---|---|---|---|---|")
        for target in TARGETS:
            wb = max(warm_rounds, key=lambda b: (best_point(warm_rounds[b], target).get("qps_median", 0)))
            w = best_point(warm_rounds[wb], target)
            wi = interpolated({wb: warm_rounds[wb]}, target)
            for system, entry in r["systems"].items():
                if system == "warm":
                    continue
                for build, v in entry["builds"].items():
                    theirs = best_point(v["serve_rounds"], target)
                    traversal = "float32" if system == "faiss-hnsw" else {"sq8": "8-bit SQ + refine", "sq4": "4-bit SQ + refine"}.get(build[:3], {"_i8": "int8", "f16": "float16"}.get(build[-3:], "float32"))
                    if not theirs["reached"]:
                        print(f"| {target} | {system} | {build} | {traversal} | not reached (best recall {theirs['best_select_recall']:.3f}) | | | | |")
                        continue
                    fg = factor(w, theirs)
                    fi = factor(wi, interpolated({build: v["serve_rounds"]}, target))
                    print(f"| {target} | {system} | {build} | {traversal} | {theirs['qps_median']:.0f} | {theirs['params']} | {theirs['report_recall']:.4f} | {cell(fg)} | {cell(fi)} |")
            print(f"| {target} | WARM | {wb} | 8-bit codes + exact rerank | {w['qps_median']:.0f} | {w['params']} | {w['report_recall']:.4f} | 1 | 1 |")


if __name__ == "__main__":
    main()
