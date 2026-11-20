#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Invariants of the matched rows, checked on the actual prompt rows (development ids only).

  equal     --a R5.jsonl --b X.jsonl : every messages_sha256 is equal (reproduction, r5check)
  presented --a R5.jsonl --b C.jsonl : a presentation-only variant keeps R5's states in order, the
                                       same screenshots, every kept AXTree fragment verbatim, and
                                       reports the text overhead

Exits non-zero on any violation.
"""
import argparse, hashlib, json, re, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
STATE = re.compile(r"- Trajectory: (\w+) |^State (\d+) \(step", re.M)


def load(paths, held):
    out = {}
    for p in paths:
        for line in open(p):
            r = json.loads(line)
            if r["question_id"] not in held:
                out[r["question_id"]] = r
    return out


def text(r):
    return "".join(i["value"] for i in r["memory_context"] if i["type"] == "text")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["equal", "presented"])
    ap.add_argument("--a", nargs="+", required=True)
    ap.add_argument("--b", nargs="+", required=True)
    a = ap.parse_args()
    held = set(json.load(open(HERE.parent / "held_ids.json")))
    A, B = load(a.a, held), load(a.b, held)
    bad = []
    if set(A) != set(B):
        bad.append(f"question ids differ: {len(A)} vs {len(B)}")
    over = []
    for q in sorted(set(A) & set(B)):
        ra, rb = A[q], B[q]
        if a.mode == "equal":
            h = lambda r: hashlib.sha256(json.dumps(r["messages"], sort_keys=True).encode()).hexdigest()
            if h(ra) != h(rb):
                bad.append(f"{q}: messages differ")
            continue
        ta, tb = text(ra), text(rb)
        if STATE.findall(ta) != STATE.findall(tb):
            bad.append(f"{q}: state order differs")
        img = lambda r: sorted(i["value"] for i in r["memory_context"] if i["type"] == "image")
        if img(ra) != img(rb):
            bad.append(f"{q}: screenshots differ")
        for piece in re.split(r"^State \d+ \(step [^\n]*\n", ta, flags=re.M)[1:]:
            body = piece.split("- AXTree:\n", 1)
            if len(body) == 2:
                frag = body[1].split("\n### Memory result")[0].rstrip()
                if frag and frag not in tb:
                    bad.append(f"{q}: an AXTree fragment is not kept verbatim")
                    break
        if rb["memory_context_was_truncated"] != ra["memory_context_was_truncated"]:
            bad.append(f"{q}: harness truncation differs")
        over.append(len(tb) / max(1, len(ta)) - 1)
    report = {"mode": a.mode, "questions": len(set(A) & set(B)), "violations": bad[:20], "violation_count": len(bad)}
    if over:
        report["text_overhead"] = {"mean": round(sum(over) / len(over), 4), "max": round(max(over), 4)}
    sys.stdout.write(json.dumps(report, indent=1) + "\n")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
