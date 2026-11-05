# SPDX-License-Identifier: Apache-2.0
"""Summarize qgraph-repair-bench reports: per-ef recall and paired QPS ratios, and the best
measured QPS reaching each recall target (no interpolation). Usage: summarize.py REPORT..."""
from __future__ import annotations

import json
from pathlib import Path
import math
import statistics
import sys

TARGETS = (0.80, 0.90, 0.95, 0.98)


def summarize(path: str) -> dict:
    report = json.loads(Path(path).read_text())
    arms = [a["arm"] for a in report["arms"]]
    points: dict[tuple[str, int], dict] = {}
    by_rep: dict[tuple[str, int, int], float] = {}
    for r in report["records"]:
        p = points.setdefault((r["arm"], r["ef"]), {"recall": set(), "qps": [], "evals": []})
        p["recall"].add(r["recall_at_10"])
        p["qps"].append(r["qps"])
        p["evals"].append(r["mean_evaluations"])
        by_rep[(r["arm"], r["ef"], r["repetition"])] = r["qps"]
    efs = sorted({ef for _, ef in points})
    reps = sorted({rep for _, _, rep in by_rep})
    rows = []
    for ef in efs:
        row = {"ef": ef}
        for arm in arms:
            p = points[(arm, ef)]
            assert len(p["recall"]) == 1, "recall must not vary across repetitions"
            ratios = [by_rep[(arm, ef, rep)] / by_rep[("base", ef, rep)] for rep in reps]
            row[arm] = {"recall": p["recall"].pop(), "qps_median": statistics.median(p["qps"]),
                        "paired_qps_ratio_vs_base": sorted(ratios), "evals": statistics.mean(p["evals"])}
        rows.append(row)
    frontier = {}
    for arm in arms:
        frontier[arm] = {}
        for t in TARGETS:
            best = [(r[arm]["qps_median"], r["ef"], r[arm]["recall"]) for r in rows if r[arm]["recall"] >= t]
            frontier[arm][t] = max(best) if best else None
    return {"path": path, "n": report["n"], "metric": report["metric"], "arms": arms,
            "stats": {a["arm"]: {k: a.get(k) for k in ("repair_seconds", "index_bytes", "stats", "train_recall_at_repair_ef_served")} for a in report["arms"]},
            "build_seconds": report["build"]["build_seconds"], "rows": rows, "frontier": frontier}


def main() -> None:
    for path in sys.argv[1:]:
        s = summarize(path)
        print(f"## {path}  n={s['n']} metric={s['metric']} base build {s['build_seconds']:.1f}s")
        for arm, st in s["stats"].items():
            stats = st["stats"] or {}
            print(f"  {arm:16} repair {st['repair_seconds']:6.1f}s bytes {st['index_bytes']:>11} "
                  f"added {stats.get('added_edges', 0):>7} skipped {stats.get('skipped_full', 0):>6} "
                  f"in-sample served recall {st['train_recall_at_repair_ef_served'] or float('nan'):.4f}")
        print("  ef  " + "  ".join(f"{a:>24}" for a in s["arms"]))
        for r in s["rows"]:
            cells = []
            for a in s["arms"]:
                c = r[a]
                ratio = statistics.median(c["paired_qps_ratio_vs_base"])
                cells.append(f"{c['recall']:.4f} {c['qps_median']:7.0f} x{ratio:.3f}")
            print(f"  {r['ef']:>4} " + "  ".join(f"{x:>24}" for x in cells))
        for t in TARGETS:
            print(f"  recall>={t}: " + ", ".join(
                f"{a} {s['frontier'][a][t][0]:.0f}qps@ef{s['frontier'][a][t][1]}" if s["frontier"][a][t]
                else f"{a} unreached" for a in s["arms"]))
        for a in s["arms"][1:]:
            print(f"  {a:16} recall minus base recall at matched cost (interpolated on base, log scale): "
                  + " ".join(f"ef{ef}:{fmt(dq)}/{fmt(de)}" for ef, dq, de in matched(s, a)))


def fmt(x: float | None) -> str:
    return "n/a" if x is None else f"{x:+.4f}"


def interpolate(xs: list[float], ys: list[float], x: float) -> float | None:
    """Linear in log x between bracketing measured base points; None outside the range."""
    pts = sorted(zip(xs, ys))
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        if x0 <= x <= x1:
            if x1 == x0:
                return max(y0, y1)
            w = (math.log(x) - math.log(x0)) / (math.log(x1) - math.log(x0))
            return y0 + w * (y1 - y0)
    return None


def matched(s: dict, arm: str) -> list[tuple[int, float, float]]:
    """Per ef of `arm`: its recall minus base recall at the same median QPS, and at the same
    mean evaluations per query (load independent); None outside the measured base range."""
    rows = s["rows"]
    base_q = [r["base"]["qps_median"] for r in rows]
    base_e = [r["base"]["evals"] for r in rows]
    base_r = [r["base"]["recall"] for r in rows]
    out = []
    for r in rows:
        c = r[arm]
        at_q = interpolate(base_q, base_r, c["qps_median"])
        at_e = interpolate(base_e, base_r, c["evals"])
        out.append((r["ef"], None if at_q is None else c["recall"] - at_q,
                    None if at_e is None else c["recall"] - at_e))
    return out


if __name__ == "__main__":
    main()
