# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Quality of a fresh WARM ranking against the committed results of every engine, by the paired per-query bootstrap.

  quality_eval.py --ranks DIR --results FILE [FILE ...]

DIR holds `<dataset>/warm-lexical-api.json` written by `quality_runs.py rank` (same queries, same word-set OR
retrieval as the reported run). Each FILE is a committed text results file whose engines carry per-query nDCG@10 and
recall@100. For each file and metric the best other engine is the one with the highest mean; the fresh WARM ranking is
compared with it, and with the committed WARM row, on identical queries. Intervals are Bonferroni-adjusted over every
comparison made in this call.
"""

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from warmbench import registry  # noqa: E402
from warmbench.common import paired_bootstrap  # noqa: E402

METRICS = (("ndcg@10", "per_query_ndcg_at_10", "ndcg10"), ("recall@100", "per_query_recall_at_100", "recall100"))


def verdict(r):
    lo, hi = r["difference_ci"]
    return "WIN" if lo > 0 else "LOSS" if hi < 0 else "TIE"


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("--ranks", type=Path, required=True)
    p.add_argument("--results", type=Path, nargs="+", required=True)
    p.add_argument("--out", type=Path)
    a = p.parse_args(argv)
    loaded = [(f, json.loads(f.read_text())) for f in a.results]
    level = 1 - 0.05 / (len(loaded) * len(METRICS) * 2)
    rows = []
    for path, data in loaded:
        dataset = data["dataset"]["id"].replace(":", "-")
        fresh = json.loads((a.ranks / dataset / "warm-lexical-api.json").read_text())["queries"]
        for metric, committed_key, fresh_key in METRICS:
            per = {s: e["serve_rounds"][0]["quality"][committed_key] for s, e in data["systems"].items() if e["serve_rounds"] and e["serve_rounds"][0].get("quality")}
            new = [q[fresh_key] for q in fresh]
            others = {s: v for s, v in per.items() if registry.system(s)["family"] != "warm"}
            best = max(others, key=lambda s: sum(others[s]))
            old = per[registry.WARM]
            for label, reference in (("best other", others[best]), ("committed WARM", old)):
                r = paired_bootstrap(new, reference, level=level)
                rows.append({"dataset": dataset, "metric": metric, "against": f"{label} ({best})" if label == "best other" else label,
                             "new": r["means"][0], "reference": r["means"][1], "difference": r["difference"], "ci": r["difference_ci"], "level": level,
                             "verdict": verdict(r), "queries": r["queries"]})
    for r in rows:
        print(f"{r['dataset']:22} {r['metric']:11} {r['against']:34} new {r['new']:.4f} ref {r['reference']:.4f} diff {r['difference']:+.4f} "
              f"[{r['ci'][0]:+.4f}, {r['ci'][1]:+.4f}] {r['verdict']}")
    if a.out:
        a.out.write_text(json.dumps(rows, indent=1) + "\n")


if __name__ == "__main__":
    main()
