#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Paired comparison of two locally scored runs on the same development questions.

    uv run python benchmarks/lme2/r5/paired.py --base A.json --new C.json [--ids ids.json] [--name C-vs-A]

Prints and returns: n, correct in each, retained/recovered/regressed/unresolved, net, and the exact
two-sided McNemar p-value on the discordant pairs.
"""
import argparse, json, sys
from math import comb


def mcnemar(b: int, c: int) -> float:
    n = b + c
    if n == 0:
        return 1.0
    k = min(b, c)
    return min(1.0, 2 * sum(comb(n, i) for i in range(k + 1)) / 2 ** n)


def compare(base: dict, new: dict, ids=None) -> dict:
    keys = sorted(set(base) & set(new) & (set(ids) if ids is not None else set(base)))
    c = {"retained": 0, "recovered": 0, "regressed": 0, "unresolved": 0}
    for k in keys:
        b, n = base[k]["correct"], new[k]["correct"]
        c["retained" if b and n else "recovered" if n else "regressed" if b else "unresolved"] += 1
    return {"n": len(keys), "base_correct": sum(base[k]["correct"] for k in keys),
            "new_correct": sum(new[k]["correct"] for k in keys), **c,
            "net": c["recovered"] - c["regressed"], "mcnemar_p": round(mcnemar(c["recovered"], c["regressed"]), 4)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--ids")
    ap.add_argument("--name", default="")
    a = ap.parse_args()
    base = json.load(open(a.base))["per_question"]
    new = json.load(open(a.new))["per_question"]
    ids = json.load(open(a.ids)) if a.ids else None
    if isinstance(ids, dict):
        ids = ids["ids"]
    sys.stdout.write(json.dumps({"name": a.name, **compare(base, new, ids)}) + "\n")


if __name__ == "__main__":
    main()
