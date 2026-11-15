# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Fusion ablation on development data only (rule: fusion_rule.json).

  hybrid_fusion.py --dev DIR --out-ablation FILE --out-frozen FILE

DIR holds `<set>-warm.json` (competitive_hybrid dump: lexical-only, vector-only and engine-fused lists, one JSON line per
query), `<set>-<comparator>.json` (hybrid_worker.py / LuceneHybrid `lists`) and `<set>-qrels.json`.
"""

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from warmbench.common import ndcg_at, recall_at  # noqa: E402

SETS = ["fiqa-train", "nfcorpus-train", "memory-selection"]
COMPARATORS = ["lucene-hnsw", "lucene-exact", "sqlite-vec", "lancedb", "tantivy-usearch"]
ALPHAS = [i / 10 for i in range(1, 10)]
DEPTHS = [100, 300, 1000]
RRF_K = [1, 5, 10, 20, 40, 60, 100, 200]
COMPARATOR_K = [1, 2, 5, 10, 20, 30, 40, 60, 100, 200]
ENGINE_DEFAULT = {"kind": "blend_minmax", "alpha": 0.5, "depth": 100}


def minmax(items):
    if not items:
        return {}
    high, low = items[0][1], items[-1][1]
    return {i: ((s - low) / (high - low) if high > low else 1.0) for i, s in items}


def maxnorm(items):
    if not items or items[0][1] <= 0:
        return {i: 0.0 for i, _ in items}
    return {i: s / items[0][1] for i, s in items}


def fuse(candidate, lex, sem, k=100):
    depth = candidate["depth"]
    lex, sem = lex[:depth], sem[:depth]
    out = {}
    if candidate["kind"] == "rrf":
        for items in (lex, sem):
            for rank, (i, _) in enumerate(items, 1):
                out[i] = out.get(i, 0.0) + 1.0 / (candidate["k"] + rank)
    else:
        norm = minmax if candidate["kind"] == "blend_minmax" else maxnorm
        a = candidate["alpha"]
        for weight, items in ((1 - a, lex), (a, sem)):
            for i, s in norm(items).items():
                out[i] = out.get(i, 0.0) + weight * s
    return [i for i, _ in sorted(out.items(), key=lambda kv: -kv[1])[:k]]


def score(found_lists, qrels):
    n = len(found_lists)
    ndcg = sum(ndcg_at(f, qrels[q], 10) for q, f in found_lists) / n
    recall = sum(recall_at(f, qrels[q], 100) for q, f in found_lists) / n
    return ndcg, recall


def load_warm(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line]


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--dev", required=True)
    p.add_argument("--out-ablation", required=True)
    p.add_argument("--out-frozen", required=True)
    a = p.parse_args()
    dev = Path(a.dev)
    qrels = {s: json.loads((dev / f"{s}-qrels.json").read_text()) for s in SETS}
    warm = {s: load_warm(dev / f"{s}-warm.json") for s in SETS}

    candidates = [{"kind": kind, "alpha": al, "depth": d} for kind in ("blend_minmax", "convex_tm2") for al in ALPHAS for d in DEPTHS]
    candidates += [{"kind": "rrf", "k": k, "depth": d} for k in RRF_K for d in (100, 1000)]
    results = []
    for c in candidates:
        per = {}
        for s in SETS:
            per[s] = score([(r["id"], fuse(c, r["lex"], r["sem"])) for r in warm[s]], qrels[s])
        results.append({"candidate": c, "per_set": per, "macro_ndcg10": sum(v[0] for v in per.values()) / len(SETS),
                        "macro_recall100": sum(v[1] for v in per.values()) / len(SETS)})
    # Replica check: the Python fusion of the default against the engine's own `blend100` ranking.
    replica = {}
    for s in SETS:
        same = sum(1 for r in warm[s] if fuse(ENGINE_DEFAULT, r["lex"], r["sem"], 10) == [i for i, _ in r["engine"]["blend100"][:10]])
        engine = score([(r["id"], [i for i, _ in r["engine"]["blend100"]]) for r in warm[s]], qrels[s])
        replica[s] = {"top10_identical_queries": same, "queries": len(warm[s]), "engine_ndcg10": engine[0], "engine_recall100": engine[1]}
    default = next(r for r in results if r["candidate"] == ENGINE_DEFAULT)
    eligible = [r for r in results if r["macro_ndcg10"] - default["macro_ndcg10"] >= 0.003
                and all(r["per_set"][s][0] - default["per_set"][s][0] >= -0.005 for s in SETS)]
    best = max(eligible, key=lambda r: r["macro_ndcg10"]) if eligible else None

    comparators = {}
    for system in COMPARATORS:
        lists = {s: json.loads((dev / f"{s}-{system}.json").read_text())["lists"] for s in SETS}
        table = {}
        for k in COMPARATOR_K:
            c = {"kind": "rrf", "k": k, "depth": 100}
            per = {s: score([(r["id"], fuse(c, r["lex"], r["vec"])) for r in lists[s]], qrels[s]) for s in SETS}
            table[k] = {"per_set": per, "macro_ndcg10": sum(v[0] for v in per.values()) / len(SETS)}
        top = max(COMPARATOR_K, key=lambda k: (table[k]["macro_ndcg10"], k == 60))
        comparators[system] = {"table": table, "chosen": top}
    Path(a.out_ablation).write_text(json.dumps({"default": default, "best_eligible": best, "all": results, "replica": replica, "comparators": comparators}, indent=1))
    frozen = {"selected_on": SETS, "rule": "fusion_rule.json", "engine_default_before": ENGINE_DEFAULT,
              "adopted": best["candidate"] if best else None, "adopted_macro_ndcg10": best["macro_ndcg10"] if best else None,
              "default_macro_ndcg10": default["macro_ndcg10"], "comparator_rrf": {s: comparators[s]["chosen"] for s in COMPARATORS},
              "warm_flags": [], "warm_previous_flags": []}
    Path(a.out_frozen).write_text(json.dumps(frozen, indent=1))
    print(json.dumps({"replica": replica, "default": default["macro_ndcg10"], "best": best and (best["candidate"], best["macro_ndcg10"], best["per_set"]),
                      "comparator_rrf": frozen["comparator_rrf"]}, indent=1))
    top = sorted(results, key=lambda r: -r["macro_ndcg10"])[:8]
    for r in top:
        print(r["candidate"], round(r["macro_ndcg10"], 4), {s: round(v[0], 4) for s, v in r["per_set"].items()})


if __name__ == "__main__":
    main()
