# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Version-effect report for interleaved runs that measured each comparator at its new and
previous pin (`warm-bench run --systems <c> <c>-prev ...`).

Usage: repin_compare.py <results dir> > report.md

Per dataset, two tables: (1) new/previous ratio of each comparator on every metric, paired per
serve round where the metric is per round (the rounds are interleaved, so adjacent runs share
machine state); (2) WARM's verdict against the previous and the new pin, using the harness's own
advantage and verdict rules (warmbench.table), flagging the rows whose class changes.
"""

import json
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from warmbench import analyze, table  # noqa: E402

PAIRS = {"sqlite-fts5": "sqlite-fts5-prev", "tantivy": "tantivy-prev", "lucene": "lucene-prev"}
METRICS = ["qps", "p50_latency", "p99_latency", "build_seconds", "build_peak_rss_mib", "serving_rss_mib",
           "cold_start_s", "disk_mib", "replace_ms_per_op", "delete_ms_per_op", "ndcg@10", "recall@100"]


def emit(text=""):
    sys.stdout.write(text + "\n")


def _per_round(entry, metric):
    out = []
    for r in entry["serve_rounds"]:
        passes = analyze._passes(r)
        if metric == "qps":
            out.append(statistics.median(p["queries"] / (p["total_ns"] / 1e9) for p in passes))
        elif metric == "p50_latency":
            out.append(statistics.median(p["p50_ns"] for p in passes))
        elif metric == "p99_latency":
            out.append(statistics.median(p["p99_ns"] for p in passes))
        elif metric == "serving_rss_mib":
            out.append(r["peak_rss_bytes"])
        elif metric == "cold_start_s":
            out.append(r["cold_start_s"])
    return out


def version_ratio(new, old, metric, new_cell, old_cell):
    """Median new/old ratio, or None when either side has no value. Per-round metrics are paired."""
    series_new, series_old = _per_round(new, metric), _per_round(old, metric)
    if series_new and len(series_new) == len(series_old):
        return statistics.median(a / b for a, b in zip(series_new, series_old))
    a, b = new_cell["value"], old_cell["value"]
    return None if a is None or b is None or b == 0 else a / b


def _version(results, system):
    v = results["systems"][system].get("version", {})
    return v.get("lucene") or v.get("tantivy") or v.get("python-sqlite3") or "?"


def report(path):
    results = json.loads(path.read_text())
    if results.get("kind") != "text":
        return
    warm = "warm" if "warm" in results["systems"] else "hms"
    metrics = analyze.text_metrics(str(path), results)
    hw = results["hardware"]
    pairs = {new: old for new, old in PAIRS.items() if old in results["systems"] and new in results["systems"]}
    emit(f"## {results['dataset']['id']} ({hw['class']}, load_1m {hw.get('load_1m', float('nan')):.2f}, {results['protocol']['rounds']} rounds)\n")
    emit("Version effect, new / previous (time, RSS and size: lower is better; qps and nDCG: higher is better):\n")
    emit("| Metric | " + " | ".join(f"{n} {_version(results, n)} / {_version(results, o)}" for n, o in pairs.items()) + " |")
    emit("|---|" + "---|" * len(pairs))
    for metric in METRICS:
        row = []
        for new, old in pairs.items():
            ratio = version_ratio(results["systems"][new], results["systems"][old], metric, metrics[metric][new], metrics[metric][old])
            row.append("n/a" if ratio is None else f"{ratio:.3f}")
        emit(f"| {metric} | " + " | ".join(row) + " |")
    emit("\nWARM verdict against the previous pin and the new pin (advantage factor above 1 means WARM is better):\n")
    emit("| Metric | Comparator | vs previous | vs new | Changed |")
    emit("|---|---|---|---|---|")
    for metric in METRICS:
        for new, old in pairs.items():
            a_old = table.advantage(metric, metrics[metric][warm], metrics[metric][old])
            a_new = table.advantage(metric, metrics[metric][warm], metrics[metric][new])
            v_old, v_new = table.verdict(a_old), table.verdict(a_new)
            f_old = table.fx(a_old["factor"]) if a_old else "n/m"
            f_new = table.fx(a_new["factor"]) if a_new else "n/m"
            emit(f"| {metric} | {new} | {v_old} {f_old} | {v_new} {f_new} | {'FLIP' if v_old != v_new else ''} |")
    emit()


def main(directory):
    emit("# Comparator re-pin: version effect and verdict changes\n")
    for path in sorted(Path(directory).glob("*.json")):
        report(path)


if __name__ == "__main__":
    main(sys.argv[1])
