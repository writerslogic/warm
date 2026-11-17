# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Ranked lists and per-query quality of WARM and the lexical comparators, for diagnosis and for choosing ranker settings
on development data.

  quality_runs.py rank  --out DIR  DATASET SYSTEM [SYSTEM ...]        store top-100 lists and per-query metrics
  quality_runs.py score --out DIR  DATASET SYSTEM [SYSTEM ...]        print mean nDCG@10 and recall@100
  quality_runs.py diff  --out DIR  DATASET WARM_SYSTEM OTHER_SYSTEM   per-query table where OTHER is better

DATASET is a `warm-bench` text dataset: `beir:<name>` (reported split), `beir:<name>:<split>` (any other split),
`memory`, `memory-selection`. SYSTEM is a registry key (warm-lexical-api, sqlite-fts5, lucene) optionally followed by
`+FLAG=VALUE,...`, extra `competitive_text` serve flags for WARM variants (the variant is part of the result name).
Every system runs the same word-set OR queries the reported comparison uses.
"""

import argparse
import json
import os
import shutil
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from warmbench import registry, runner, textdata  # noqa: E402
from warmbench.common import ndcg_at, recall_at  # noqa: E402

THREADS = dict(runner.SINGLE_THREAD)


def _name(dataset):
    return dataset.replace(":", "-")


def _split(system):
    """`base+flag=value,env.NAME=value`: serve flags and environment variables of a development variant."""
    base, _, spec = system.partition("+")
    pairs = [kv.split("=", 1) for kv in spec.split(",") if kv]
    return (base, [a for k, v in pairs if not k.startswith("env.") for a in ("--" + k, v)],
            {k[4:]: v for k, v in pairs if k.startswith("env.")})


def _store(dataset, base):
    return runner.WORK / f"quality-{_name(dataset)}" / base


def rank(dataset, system, out):
    base, flags, env = _split(system)
    folder = textdata.load(dataset)
    target = out / _name(dataset) / (system.replace(":", "_") + ".json")
    if target.exists():
        return json.loads(target.read_text())
    store = _store(dataset, base)
    if not store.exists():
        store.parent.mkdir(parents=True, exist_ok=True)
        runner._text_cmd(base, "build", folder / "input.json", store, store.with_suffix(".build.json"), THREADS)
    serve_input = folder / "serve-input.json"
    serve_input.write_text(json.dumps({"documents": [], "queries": json.loads((folder / "input.json").read_text())["queries"]}))
    serve_out = store.with_suffix(".serve.json")
    runner._text_cmd(base, "serve", serve_input, store, serve_out, THREADS | env, extra=flags)
    rows = json.loads(serve_out.read_text())["rows"]
    qrels = json.loads((folder / "qrels.json").read_text())
    result = {"dataset": dataset, "system": system, "queries": []}
    for row in rows:
        found = [h["documentId"] for h in row["results"]]
        rel = qrels[row["id"]]
        result["queries"].append({"id": row["id"], "found": found, "scores": [h["lexicalScore"] for h in row["results"]],
                                  "ndcg10": ndcg_at(found, rel, 10), "recall100": recall_at(found, rel, 100)})
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(result))
    return result


def summary(result):
    q = result["queries"]
    return statistics.fmean(x["ndcg10"] for x in q), statistics.fmean(x["recall100"] for x in q), len(q)


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("cmd", choices=["rank", "score", "diff"])
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("dataset")
    p.add_argument("systems", nargs="+")
    a = p.parse_args(argv)
    if a.cmd in ("rank", "score"):
        for s in a.systems:
            n, r, count = summary(rank(a.dataset, s, a.out))
            print(f"{a.dataset}\t{s}\tndcg@10 {n:.4f}\trecall@100 {r:.4f}\tqueries {count}")
    else:
        mine, other = (rank(a.dataset, s, a.out) for s in a.systems[:2])
        folder = textdata.load(a.dataset)
        qrels = json.loads((folder / "qrels.json").read_text())
        for m, o in zip(mine["queries"], other["queries"]):
            rel = {k for k, g in qrels[m["id"]].items() if g > 0}
            missed = [d for d in o["found"] if d in rel and d not in set(m["found"])]
            if o["ndcg10"] > m["ndcg10"] or missed:
                print(json.dumps({"id": m["id"], "ndcg_warm": m["ndcg10"], "ndcg_other": o["ndcg10"], "relevant_missed_by_warm_top100": missed}))


if __name__ == "__main__":
    main()
