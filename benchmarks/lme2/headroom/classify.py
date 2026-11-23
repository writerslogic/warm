#!/usr/bin/env python3
"""Classify the round-6 development failures of LongMemEval-V2 by cause, judge-free.

Reads only development material: questions.jsonl is filtered against held_ids.json on load and
held-out rows are never kept, printed or written. Inputs are existing artifacts; nothing is
scored and no model is called.

    uv run python benchmarks/lme2/headroom/classify.py --root /Volumes/A/.hms-lb/lme2
"""
import argparse, collections, json, re
from pathlib import Path

CATEGORIES = {
    1: "input or representation loss",
    2: "action-target or within-observation row-binding loss",
    3: "missing retrieval candidates",
    4: "cross-observation entity correspondence",
    5: "temporal scope, state tracking or supersession",
    6: "incomplete multi-source evidence selection",
    7: "failure-recovery or procedural-advice evidence",
    8: "context packing or evidence presentation",
    9: "unsupported absence or false-premise handling",
    10: "reader interpretation",
    11: "genuinely missing observations",
    12: "mixed or unclassifiable",
}
COMPARE = re.compile(r"\b(vs|versus|compare|comparing|among these|both forms|between)\b", re.I)


def load_dev(root: Path, held: set):
    qs = {}
    for line in Path(root / "data" / "questions.jsonl").read_text().splitlines():
        q = json.loads(line)
        if q["id"] in held:
            continue
        qs[q["id"]] = {k: q[k] for k in ("id", "domain", "question_type", "question", "answer", "eval_function")}
    return qs


def review_kind(row, review):
    t = row["type"]
    if t.endswith("-abs"):
        return "F"
    if t == "errors-gotchas":
        return "G"
    if t == "procedure":
        return "W"
    return review["by_id"][row["id"]]


def classify(row, q, review):
    t, gold_in = row["type"], row["gold_in_ctx"]
    primary, contrib, basis = None, [], "heuristic"
    if t.endswith("-abs"):
        primary = 9
    elif t == "errors-gotchas":
        primary = 7
        contrib.append(3 if gold_in is False else 12)
    elif t == "procedure":
        primary = 7
        if gold_in is True:
            contrib.append(10)
        elif gold_in is False:
            contrib.append(3)
    else:
        if gold_in is True:
            primary = 10
        elif gold_in is False:
            primary = 3
        else:
            primary = 12
        if t == "dynamic-environment":
            contrib.append(5)
        if COMPARE.search(q["question"]):
            contrib.append(6)
    if gold_in is None and primary != 9:
        contrib.append(12)
    return {
        "id": row["id"], "type": t, "domain": q["domain"], "scorer": row["ef"], "r6_score": row["score"],
        "gold_phrase_in_context": gold_in, "primary": primary, "contributing": sorted(set(contrib)),
        # Phrase presence is a proxy: it does not show the evidence was complete or correctly bound.
        # Cause labels come from rules over the question type, a regular expression over the
        # question text and the recorded gold-phrase flag; none is a counterfactual result.
        "cause_basis": basis,
        # A recorded manual judgement, with its rationale in review.json; not computed here.
        "correspondence_candidate": False,
        "correspondence_basis": "manual",
        "correspondence_rationale": review["kinds"][review_kind(row, review)],
        # No identity or evidence annotations exist in the dataset: unknown, not false.
        "identity_oracle_constructible": None,
        "counterfactual": None,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default="/Volumes/A/.hms-lb/lme2")
    ap.add_argument("--held", default=str(Path(__file__).resolve().parent.parent / "held_ids.json"))
    ap.add_argument("--out", default=str(Path(__file__).resolve().parent / "classification.json"))
    a = ap.parse_args()
    root = Path(a.root)
    held = set(json.loads(Path(a.held).read_text()))
    qs = load_dev(root, held)
    assert len(qs) == 228 and not (set(qs) & held)
    rows = json.loads(Path(root / "final-arch" / "forensics_r6_dev.json").read_text())
    assert {r["id"] for r in rows} == set(qs), "forensics must cover exactly the development ids"
    fails = [r for r in rows if r["score"] < 1]
    review = json.loads(Path(Path(__file__).resolve().parent / "review.json").read_text())
    out = [classify(r, qs[r["id"]], review) for r in fails]
    missing = [o["id"] for o in out if o["type"] in ("static-environment", "dynamic-environment") and o["id"] not in review["by_id"]]
    assert not missing, missing
    prim = collections.Counter(o["primary"] for o in out)
    anyc = collections.Counter(c for o in out for c in {o["primary"], *o["contributing"]})
    summary = {
        "population": "LongMemEval-V2 development split, round-6 system",
        "dev_questions": len(qs), "failures": len(fails),
        "primary": {f"{k} {CATEGORIES[k]}": v for k, v in sorted(prim.items())},
        "primary_or_contributing": {f"{k} {CATEGORIES[k]}": v for k, v in sorted(anyc.items())},
        "correspondence_candidates": sum(o["correspondence_candidate"] for o in out),
        "correspondence_review": "manual, question text and gold answer only; no identity counterfactual run",
        "label_bases": {"cause": "heuristic", "correspondence": "manual (review.json)"},
        "by_type": dict(collections.Counter(o["type"] for o in out)),
        "gold_phrase_in_context": dict(collections.Counter(str(o["gold_phrase_in_context"]) for o in out)),
        "source": "final-arch/forensics_r6_dev.json; questions filtered against held_ids.json",
    }
    Path(a.out).write_text(json.dumps({"summary": summary, "failures": out}, indent=1) + "\n")
    print(json.dumps(summary, indent=1))


if __name__ == "__main__":
    main()
