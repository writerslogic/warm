#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Memory-side comparison of state selections at R5's budgets. Evaluator-only: the annotations and
gold phrases are read here to score what each selection delivers; the selections never see them.

For every variant: on the annotated pilot cohort, annotated evidence-state coverage (the headers of
every state of some annotated set are in the prompt; it does not check clipped text or screenshots,
see evidence_delivery.py; renamed from `cohort_evidence_complete`, results/selection_eval_v1.json); on all locally scored phrase questions, whether every gold phrase
is in the delivered text (a weak locator); delivered characters, screenshots, trajectories and
memory query time.

    uv run --python /Volumes/A/.hms-lb/lme2/venv/bin/python benchmarks/lme2/r5/selection_eval.py \
        --out benchmarks/lme2/r5/selection_eval.json
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import prompts  # noqa: E402
import warm_select  # noqa: E402,F401
from memory_modules.memory import MEMORY_TYPES  # noqa: E402

VARIANTS = {
    "r5": {"select": "r5"},
    "expand_r2": {"select": "r5", "radius": 2},
    "mmr": {"select": "mmr"},
    "pagecover": {"select": "pagecover"},
    "pagecover_noid": {"select": "pagecover", "identity_weight": 0.0},
    "pagecover_t2": {"select": "pagecover", "per_trajectory": 2},
}


def delivered_states(items):
    text = "".join(i["value"] for i in items if i["type"] == "text")
    shown, cur = set(), None
    for tid, st in re.findall(r"- Trajectory: (\w+) |^State (\d+) \(step", text, re.M):
        if tid:
            cur = tid
        else:
            shown.add(f"{cur}/{st}")
    return text, shown


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--variants", nargs="*", default=list(VARIANTS))
    a = ap.parse_args()
    sys.path.insert(0, str(prompts.LME2 / "upstream"))
    from evaluation.qa_eval_metrics import DEFAULT_SEPARATORS, normalize_phrase, split_phrases
    import evidence_locate

    held = set(json.load(open(HERE.parent / "held_ids.json")))
    local = json.load(open(HERE / "r5_historical_local.json"))["per_question"]
    ann = {}
    for f in sorted((HERE / "annotations").glob("batch_*.json")):
        for r in json.load(open(f)):
            if r.get("sufficient_in_observations") == "yes":
                ann[r["qid"]] = [set(e["states"]) for e in r["evidence_sets"] if e["states"]]
    rows = {}
    for d in ("web", "enterprise"):
        for line in open(prompts.LME2 / f"round5/runs/slice12i_{d}/prompt_rows.jsonl"):
            r = json.loads(line)
            if r["question_id"] in local and r["question_id"] not in held:
                rows[r["question_id"]] = (d, r)
    need = {t for _, r in rows.values() for t in r["haystack_ids"]}
    trajs = {}
    with open(prompts.LME2 / "data/trajectories.jsonl") as f:
        for line in f:
            if line[7:15] in need:
                t = json.loads(line)
                trajs[t["id"]] = t
    result = {}
    for name in a.variants:
        per = {}
        for d in ("web", "enterprise"):
            qs = [(q, r) for q, (dom, r) in rows.items() if dom == d]
            params = {"cache_dir": str(prompts.LME2 / "embed_cache"), **prompts.R5, "render": "r5", **VARIANTS[name]}
            mem = MEMORY_TYPES["warm_select"](params)
            for tid in qs[0][1]["haystack_ids"]:
                mem.insert(trajs[tid])
            for q, r in qs:
                t0 = time.perf_counter()
                items = mem.query(r["question_text"], r["question_image"])
                dt = time.perf_counter() - t0
                text, shown = delivered_states(items)
                rec = {"chars": len(text), "images": sum(i["type"] == "image" for i in items),
                       "trajectories": len({s.split('/')[0] for s in shown}), "query_s": round(dt, 4)}
                if q in ann:
                    rec["evidence_state_coverage"] = any(e <= shown for e in ann[q])
                spec = r["eval_function"]
                if spec.startswith("norm_phrase"):
                    seps, norm = evidence_locate.spec_kwargs(spec)
                    phrases = set(split_phrases(r["answer_gold"], separators=seps or DEFAULT_SEPARATORS, **norm))
                    norm_text = " " + normalize_phrase(text, **norm) + " "
                    rec["phrases_delivered"] = all(re.search(r"\b%s\b" % re.escape(p), norm_text) for p in phrases)
                if name == "r5":
                    rec["equals_stored_r5"] = items == r["memory_context"]
                per[q] = rec
        vals = list(per.values())
        ev = [v["evidence_state_coverage"] for v in vals if "evidence_state_coverage" in v]
        ph = [v["phrases_delivered"] for v in vals if "phrases_delivered" in v]
        result[name] = {
            "params": VARIANTS[name],
            "annotated_evidence_state_coverage": [sum(ev), len(ev)],
            "phrase_delivered": [sum(ph), len(ph)],
            "mean_chars": round(sum(v["chars"] for v in vals) / len(vals)),
            "mean_images": round(sum(v["images"] for v in vals) / len(vals), 2),
            "mean_trajectories": round(sum(v["trajectories"] for v in vals) / len(vals), 2),
            "mean_query_s": round(sum(v["query_s"] for v in vals) / len(vals), 4),
            **({"equals_stored_r5": sum(v.get("equals_stored_r5", False) for v in vals)} if name == "r5" else {}),
            "per_question": per,
        }
        sys.stdout.write(json.dumps({name: {k: v for k, v in result[name].items() if k != "per_question"}}) + "\n")
    Path(a.out).write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
