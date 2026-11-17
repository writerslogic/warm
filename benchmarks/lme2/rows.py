#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Reader-row bookkeeping between the two reader passes.
  pending   --rows a.jsonl... --have cache.json... --out todo.jsonl : rows whose messages have no reader output yet, question_id -> qid|hash12
  assemble  --rows a.jsonl... --have cache.json... --out rc.json    : reader cache keyed by plain question id for the final rows, matched by messages hash"""
import argparse, hashlib, json


def mhash(r): return hashlib.sha256(json.dumps(r["messages"], sort_keys=True).encode()).hexdigest()


def main():
    ap = argparse.ArgumentParser(); ap.add_argument("cmd", choices=["pending", "assemble"])
    ap.add_argument("--rows", nargs="+", required=True); ap.add_argument("--have", nargs="*", default=[]); ap.add_argument("--out", required=True); a = ap.parse_args()
    H = {}
    for f in a.have:
        for v in json.load(open(f)).values(): H[v["messages_sha256"]] = v
    rows = [json.loads(l) for f in a.rows for l in open(f) if l.strip()]
    if a.cmd == "pending":
        seen = set(); n = 0
        with open(a.out, "w") as o:
            for r in rows:
                h = mhash(r)
                if h in H or h in seen: continue
                seen.add(h); r["question_id"] = f"{r['question_id']}|{h[:12]}"; o.write(json.dumps(r) + "\n"); n += 1
        print(a.out, "pending rows", n)
    else:
        rc = {}; miss = 0
        for r in rows:
            h = mhash(r)
            if h in H: rc[r["question_id"]] = H[h]
            else: miss += 1
        json.dump(rc, open(a.out, "w")); print(a.out, len(rc), "missing", miss)
        if miss: raise SystemExit(1)


if __name__ == "__main__":
    main()
