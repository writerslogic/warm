#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Evaluator-only: what of each annotated evidence set actually reached the reader, per condition,
read from the dispatched prompt rows of the annotated pilot cohort.

Per required state: `present` (its header is in the prompt), `text` (`full` when the state's whole
accessibility tree is in the prompt verbatim; `clipped` when only part of it is, because of the
60000-character state cap or the running budget; `absent`), and `screenshot` (its own screenshot
is attached). Per evidence set and per question:

  verified    every state of some set is present with its full text, and, when the set needs a
              screenshot, every state's screenshot is attached
  incomplete  every set lacks a state, or needs a screenshot none of whose states has one attached
  unknown     neither: a state is clipped (the annotation does not say which lines are required)
              or only some screenshots of a visual set are attached

"Annotated evidence-state coverage" is the older, weaker header-only measure (all states of a set
present), kept for comparison with `selection_eval.py` output before its rename.

    uv run python benchmarks/lme2/r5/evidence_delivery.py --out benchmarks/lme2/r5/results/evidence_delivery.json
"""
import argparse, json, re, sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
L = Path("/Volumes/A/.hms-lb/lme2")
UP = L / "r5x" / "up"
ROWS = {
    "A": [L / "round5/runs/slice12i_web/prompt_rows.jsonl", L / "round5/runs/slice12i_enterprise/prompt_rows.jsonl"],
    "B": [UP / "rows_B.jsonl"], "C": [UP / "rows_C.jsonl"], "D": [UP / "rows_D.jsonl"], "P": [UP / "rows_P.jsonl"],
}
HEAD = re.compile(r"- Trajectory: (\w+) |^State (\d+) \(step[^\n]*\n", re.M)


def segments(row):
    """(tid, state) -> rendered body text, and the set of attached image paths."""
    text = "".join(i["value"] for i in row["memory_context"] if i["type"] == "text")
    images = {i["value"] for i in row["memory_context"] if i["type"] == "image"}
    marks = list(HEAD.finditer(text))
    out, cur = {}, None
    for k, m in enumerate(marks):
        if m.group(1):
            cur = m.group(1)
            continue
        end = marks[k + 1].start() if k + 1 < len(marks) else len(text)
        out[(cur, int(m.group(2)))] = text[m.end():end]
    return out, images


def load_annotations():
    ann = {}
    for f in sorted((HERE / "annotations").glob("batch_*.json")):
        for r in json.load(open(f)):
            ann[r["qid"]] = r
    return ann


def assess(row, ann_rec, states_src):
    seg, images = segments(row)
    sets = []
    for es in ann_rec.get("evidence_sets") or []:
        per = []
        for s in es["states"]:
            tid, i = s.split("/")[0], int(s.split("/")[1])
            src = states_src[(tid, i)]
            body = seg.get((tid, i))
            tree = (src["accessibility_tree"] or "").rstrip()
            text = "absent" if body is None else "full" if tree in body else "clipped"
            shot = src.get("screenshot")
            png = str(L / "data/screenshots" / tid / Path(str(shot)).name) if shot else None
            per.append({"state": s, "present": body is not None, "text": text, "screenshot": png in images})
        vis = bool(es.get("visual_required"))
        shots = [p["screenshot"] for p in per]
        if all(p["text"] == "full" for p in per) and (not vis or all(shots)):
            status = "verified"
        elif any(not p["present"] for p in per) or (vis and not any(shots)):
            status = "incomplete"
        else:
            status = "unknown"
        sets.append({"visual_required": vis, "status": status, "states": per,
                     "state_coverage": all(p["present"] for p in per)})
    st = [x["status"] for x in sets]
    q = "verified" if "verified" in st else "incomplete" if st and all(x == "incomplete" for x in st) else "unknown"
    return {"status": q, "state_coverage": any(x["state_coverage"] for x in sets), "sets": sets}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--scores", default=str(L / "r5x" / "out"), help="dir with score_<C>.json")
    a = ap.parse_args()
    held = set(json.load(open(HERE.parent / "held_ids.json")))
    cohort = [q for q in json.load(open(HERE / "pilot_cohort.json"))["ids"] if q not in held]
    ann = load_annotations()
    need = {(s.split("/")[0], int(s.split("/")[1])) for r in ann.values() for e in r.get("evidence_sets") or [] for s in e["states"]}
    tids = {t for t, _ in need}
    src = {}
    with open(L / "data/trajectories.jsonl") as f:
        for line in f:
            if line[7:15] in tids:
                t = json.loads(line)
                for s in t["states"]:
                    if (t["id"], int(s["state_index"])) in need:
                        src[(t["id"], int(s["state_index"]))] = s
    result = {}
    for c, paths in ROWS.items():
        rows = {}
        for p in paths:
            for line in open(p):
                r = json.loads(line)
                if r["question_id"] in cohort:
                    rows[r["question_id"]] = r
        sc_path = Path(a.scores) / f"score_{c}.json"
        scores = json.load(open(sc_path))["per_question"] if sc_path.exists() else {}
        per = {}
        for q in cohort:
            if q not in rows or ann.get(q, {}).get("sufficient_in_observations") != "yes":
                continue
            rec = assess(rows[q], ann[q], src)
            rec["correct"] = scores.get(q, {}).get("correct")
            per[q] = rec
        cnt = Counter(r["status"] for r in per.values())
        right = Counter(r["status"] for r in per.values() if r["correct"])
        result[c] = {
            "questions": len(per),
            "annotated_evidence_state_coverage": sum(r["state_coverage"] for r in per.values()),
            "delivery": dict(cnt),
            "correct_by_delivery": dict(right),
            "wrong_with_verified_evidence": sorted(q for q, r in per.items() if r["status"] == "verified" and r["correct"] is False),
            "per_question": per,
        }
        sys.stdout.write(f"{c}: n={len(per)} coverage={result[c]['annotated_evidence_state_coverage']} delivery={dict(cnt)} correct={dict(right)} wrong_verified={len(result[c]['wrong_with_verified_evidence'])}\n")
    paired = {}
    for base, new in (("A", "P"), ("A", "C"), ("A", "B"), ("A", "D")):
        b, n = result[base]["per_question"], result[new]["per_question"]
        qs = sorted(set(b) & set(n))
        if set(b) != set(n):
            raise SystemExit(f"{base} and {new} cover different annotated questions")
        moves = Counter(f"{b[q]['status']}->{n[q]['status']}" for q in qs)
        paired[f"{new}_vs_{base}"] = {
            "questions": len(qs),
            "state_coverage_gained": sorted(q for q in qs if n[q]["state_coverage"] and not b[q]["state_coverage"]),
            "state_coverage_lost": sorted(q for q in qs if b[q]["state_coverage"] and not n[q]["state_coverage"]),
            "delivery_transitions": dict(sorted(moves.items())),
        }
        p = paired[f"{new}_vs_{base}"]
        sys.stdout.write(f"{new} vs {base}: coverage +{len(p['state_coverage_gained'])}/-{len(p['state_coverage_lost'])} transitions={p['delivery_transitions']}\n")
    result["paired"] = paired
    Path(a.out).write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
