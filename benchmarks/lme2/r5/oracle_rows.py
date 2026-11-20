#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Diagnostic oracle conditions B and D. Evaluator-only: reads the evidence annotations, which
production memory never sees; no memory module imports this file.

B: the annotated evidence states (first evidence set) ranked ahead of R5's own ranking, R5
   rendering. D: the same states with the bound rendering of condition C. Both use R5's radius,
   per-state cap and screenshot count (the first six ranked states, so evidence screenshots
   first), and a text budget equal to the characters R5 delivered for that question, so the
   oracle never reads more text than the baseline.

    uv run --python /Volumes/A/.hms-lb/lme2/venv/bin/python benchmarks/lme2/r5/oracle_rows.py \
        --condition B --out /Volumes/A/.hms-lb/lme2/r5x/oracle_B
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import prompts  # noqa: E402
from memory_modules.memory import register_memory  # noqa: E402
from warm_bound import WarmBoundMemory  # noqa: E402


@register_memory
class OracleMemory(WarmBoundMemory):
    memory_type = "warm_oracle"

    def _rank(self, query):
        base = super()._rank(query)
        index = {}
        for sid, (tid, pos) in enumerate(self._s_keys):
            index[(tid, int(self._states[tid][pos]["i"]))] = sid
        ev = [index[(t, s)] for t, s in self.p["evidence"].get(query, []) if (t, s) in index]
        return ev + [s for s in base if s not in ev]

    def query(self, query, query_image=None):
        self.p["max_context_chars"] = int(self.p["budgets"][query])
        return super().query(query, query_image)


def load_annotations():
    out = {}
    for f in sorted((HERE / "annotations").glob("batch_*.json")):
        for rec in json.load(open(f)):
            out[rec["qid"]] = rec
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--condition", required=True, choices=["B", "D"])
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    held = set(json.load(open(HERE.parent / "held_ids.json")))
    cohort = [q for q in json.load(open(HERE / "pilot_cohort.json"))["ids"] if q not in held]
    ann = load_annotations()
    rows = {}
    for d in ("web", "enterprise"):
        for line in open(prompts.LME2 / f"round5/runs/slice12i_{d}/prompt_rows.jsonl"):
            r = json.loads(line)
            if r["question_id"] in cohort:
                rows[r["question_id"]] = (d, r)
    built, skipped = {}, {}
    for d in ("web", "enterprise"):
        evidence, budgets, ids = {}, {}, []
        for q, (dom, r) in sorted(rows.items()):
            if dom != d:
                continue
            rec = ann.get(q)
            sets = (rec or {}).get("evidence_sets") or []
            if not rec or rec.get("sufficient_in_observations") != "yes" or not sets or not sets[0]["states"]:
                skipped[q] = "no complete evidence annotated" if rec else "not annotated"
                continue
            evidence[r["question_text"]] = [(s.split("/")[0], int(s.split("/")[1])) for s in sets[0]["states"]]
            budgets[r["question_text"]] = sum(len(i["value"]) for i in r["memory_context"] if i["type"] == "text")
            ids.append(q)
        if not ids:
            continue
        params = {**prompts.R5, "render": "r5" if a.condition == "B" else "bound", "evidence": evidence, "budgets": budgets}
        out = Path(a.out).resolve() / d
        path = prompts.build("warm_oracle", params, d, out, Path(prompts.LME2 / "embed_cache"), question_ids=ids)
        for line in open(path):
            r = json.loads(line)
            text = "".join(i["value"] for i in r["memory_context"] if i["type"] == "text")
            shown, cur = set(), None
            for tid, st in re.findall(r"- Trajectory: (\w+) |^State (\d+) \(step", text, re.M):
                if tid:
                    cur = tid
                else:
                    shown.add((cur, int(st)))
            ev = evidence[r["question_text"]]
            built[r["question_id"]] = {
                "evidence_states": len(ev),
                "evidence_states_shown": sum((t, s) in shown for t, s in ev),
                "text_chars": len(text),
                "images": sum(i["type"] == "image" for i in r["memory_context"]),
                "state_cap_hit": "[...truncated]" in text,
            }
    summary = {"condition": a.condition, "built": len(built), "skipped": skipped, "rows": built}
    (Path(a.out) / "build.json").write_text(json.dumps(summary, indent=1, sort_keys=True) + "\n")
    sys.stdout.write(json.dumps({k: v for k, v in summary.items() if k != "rows"}, indent=1) + "\n")


if __name__ == "__main__":
    main()
