# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Tables and verdicts of the hybrid contract from `hybrid_run.py eval` results.

  hybrid_report.py --results DIR --out FILE.md [--json FILE]

Quality: WARM against the best comparator row (highest mean) by the paired per-query bootstrap. Speed: ratio of
per-round median QPS against the comparator row with the highest median, bootstrap interval over the rounds. Intervals are
Bonferroni-adjusted over the corpora and metrics of each family, so a verdict is WIN or LOSS only when the adjusted
interval excludes no difference (difference 0, ratio 1).
"""

import argparse
import json
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from warmbench.common import paired_bootstrap, ratio_ci  # noqa: E402

ORDER = ["fiqa", "scifact", "nfcorpus", "memory", "arguana", "scidocs"]


def med(values):
    return statistics.median(values)


def per_round(row):
    out = {"qps_incl": [], "qps_excl": [], "p50_incl": [], "p99_incl": [], "p50_excl": [], "p99_excl": [], "embed_p50": []}
    for r in row["rounds"]:
        i = r["including"]
        out["qps_incl"].append(i["queries"] / (i["total_ns"] / 1e9))
        out["p50_incl"].append(i["p50_ns"] / 1e3)
        out["p99_incl"].append(i["p99_ns"] / 1e3)
        out["embed_p50"].append(r["embed"]["p50_ns"] / 1e3)
        out["qps_excl"].append(med([p["queries"] / (p["total_ns"] / 1e9) for p in r["excluding"]]))
        out["p50_excl"].append(med([p["p50_ns"] / 1e3 for p in r["excluding"]]))
        out["p99_excl"].append(med([p["p99_ns"] / 1e3 for p in r["excluding"]]))
    return out


def verdict_difference(lo, hi):
    return "WIN" if lo > 0 else "LOSS" if hi < 0 else "TIE"


def verdict_ratio(lo, hi):
    return "WIN" if lo > 1 else "LOSS" if hi < 1 else "TIE"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--results", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--json")
    a = p.parse_args()
    files = {n: json.loads((Path(a.results) / f"hybrid-{n}.json").read_text()) for n in ORDER if (Path(a.results) / f"hybrid-{n}.json").exists()}
    family = max(1, len(files) * 2)
    level = 1 - 0.05 / family
    lines = [f"Intervals: 95 percent family-wise (each of {family} comparisons per family at {level:.4f}). One machine; not official results.", ""]
    report = {}
    for name, data in files.items():
        systems = data["systems"]
        warm = systems["warm"]
        rows = {}
        for row, body in systems.items():
            q = body["rounds"][0]["quality"]
            rows[row] = {"ndcg10": q["ndcg_at_10"], "recall100": q["recall_at_100"], "speed": per_round(body),
                         "build_s": med([b["ingest_s"] for b in data["builds"][{"warm": "warm", "lucene-hnsw": "lucene", "lucene-exact": "lucene"}.get(body["system"], body["system"])]]),
                         "index_mib": body["index_bytes"] / 2**20, "rss_mib": med([r["peak_rss_bytes"] for r in body["rounds"]]) / 2**20,
                         "foreign_max": max(r["foreign_cpu_cores"] for r in body["rounds"]), "mismatch": sum(r["including_id_mismatches"] for r in body["rounds"])}
        others = [r for r in rows if r != "warm" and r != "warm-previous"]
        lines += [f"### {name} ({data['dataset']}, {data['queries']} queries)", "",
                  "| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |", "|---|---|---|---|---|---|---|---|---|---|---|---|"]
        for r, v in rows.items():
            s = v["speed"]
            lines.append(f"| {r} | {v['ndcg10']:.4f} | {v['recall100']:.4f} | {med(s['qps_excl']):.0f} | {med(s['qps_incl']):.1f} | {med(s['p50_excl']):.0f} | {med(s['p99_excl']):.0f} | {med(s['p50_incl']):.0f} | {med(s['embed_p50']):.0f} | {v['build_s']:.1f} | {v['index_mib']:.0f} | {v['rss_mib']:.0f} |")
        out = {"rows": {r: {k: v for k, v in d.items() if k != "speed"} | {"speed_median": {m: med(x) for m, x in d["speed"].items()}} for r, d in rows.items()}, "verdicts": {}}
        for metric, key in (("nDCG@10", "per_query_ndcg_at_10"), ("recall@100", "per_query_recall_at_100")):
            best = max(others, key=lambda r: sum(systems[r]["rounds"][0]["quality"][key]))
            r = paired_bootstrap(warm["rounds"][0]["quality"][key], systems[best]["rounds"][0]["quality"][key], level=level)
            v = verdict_difference(*r["difference_ci"])
            out["verdicts"][metric] = {"best_comparator": best, "warm": r["means"][0], "best": r["means"][1], "difference": r["difference"], "ci": r["difference_ci"], "verdict": v}
            lines.append(f"\n{metric}: WARM {r['means'][0]:.4f} vs best comparator {best} {r['means'][1]:.4f}, paired difference {r['difference']:+.4f} [{r['difference_ci'][0]:+.4f}, {r['difference_ci'][1]:+.4f}] **{v}**")
        for metric, key in (("QPS excluding embedding", "qps_excl"), ("QPS including embedding", "qps_incl")):
            best = max(others, key=lambda r: med(rows[r]["speed"][key]))
            ratio, ci = ratio_ci(rows["warm"]["speed"][key], rows[best]["speed"][key], level=level)
            v = verdict_ratio(*ci)
            out["verdicts"][metric] = {"best_comparator": best, "ratio": ratio, "ci": ci, "verdict": v}
            lines.append(f"\n{metric}: WARM {med(rows['warm']['speed'][key]):.1f} vs {best} {med(rows[best]['speed'][key]):.1f}, ratio {ratio:.2f} [{ci[0]:.2f}, {ci[1]:.2f}] **{v}**")
        lines.append(f"\nforeign CPU max {max(v['foreign_max'] for v in rows.values()):.4f} cores; including-pass id mismatches {sum(v['mismatch'] for v in rows.values())}\n")
        report[name] = out
    Path(a.out).write_text("\n".join(lines) + "\n")
    if a.json:
        Path(a.json).write_text(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
