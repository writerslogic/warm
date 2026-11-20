#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Checks each reader cache of the experiment against its frozen manifest before anything is scored.

For every condition: the cache holds exactly the registered ids (no missing, no unexpected, no
held-out id); every entry's `messages_sha256` equals the hash of the row that was dispatched; and
outputs that are empty (a bad request recorded as no answer) are counted. Exits non-zero on any
violation, so a partial run cannot be scored as if it were complete.

    uv run python benchmarks/lme2/r5/validate_runs.py --out benchmarks/lme2/r5/results/validation.json
"""
import argparse, hashlib, json, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
L = Path("/Volumes/A/.hms-lb/lme2")
OUT = L / "r5x" / "out"
UP = L / "r5x" / "up"
A_ROWS = [L / "round5/runs/slice12i_web/prompt_rows.jsonl", L / "round5/runs/slice12i_enterprise/prompt_rows.jsonl"]
CONDITIONS = {
    "A": (A_ROWS, ["pilot_ids.json", "rest_ids.json"]),
    "B": ([UP / "rows_B.jsonl"], ["pilot_ids.json"]),
    "C": ([UP / "rows_C.jsonl"], ["pilot_ids.json", "rest_ids.json"]),
    "D": ([UP / "rows_D.jsonl"], ["pilot_ids.json"]),
    "P": ([UP / "rows_P.jsonl"], ["rest_ids.json", "pilot_ids.json"]),
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--conditions", nargs="*", default=list(CONDITIONS))
    a = ap.parse_args()
    held = set(json.load(open(HERE.parent / "held_ids.json")))
    local = set(json.load(open(HERE / "r5_historical_local.json"))["per_question"])
    pilot = json.load(open(UP / "pilot_ids.json"))
    rest = json.load(open(UP / "rest_ids.json"))
    assert len(pilot) == len(set(pilot)) == 36 and len(rest) == len(set(rest)) == 113
    assert not set(pilot) & set(rest) and set(pilot) | set(rest) == local and len(local) == 149
    assert not (set(pilot) | set(rest)) & held
    report, bad = {}, []
    for c in a.conditions:
        rows_paths, id_files = CONDITIONS[c]
        dispatched = set()
        for f in id_files:
            dispatched |= set(json.load(open(UP / f)))
        hashes, dup = {}, 0
        for p in rows_paths:
            for line in open(p):
                r = json.loads(line)
                q = r["question_id"]
                if q not in dispatched:
                    continue
                if q in hashes:
                    dup += 1
                hashes[q] = hashlib.sha256(json.dumps(r["messages"], sort_keys=True).encode()).hexdigest()
        # Rows exist for every dispatched id except where none was built (B and D: b7cc4da2).
        expected = set(hashes)
        cache = json.load(open(OUT / f"cache_{c}.json"))
        missing = sorted(expected - set(cache))
        unexpected = sorted(set(cache) - expected)
        mismatch = sorted(q for q in expected & set(cache) if cache[q]["messages_sha256"] != hashes[q])
        empty = sorted(q for q in expected & set(cache) if not (cache[q]["output"].get("response_raw") or "").strip())
        rec = {"registered": len(dispatched), "rows_built": len(expected), "not_built": sorted(dispatched - expected),
               "duplicate_rows": dup, "cached": len(cache), "missing": missing, "unexpected": unexpected,
               "hash_mismatch": mismatch, "held_out_in_cache": sorted(set(cache) & held), "empty_outputs": empty}
        report[c] = rec
        for k in ("missing", "unexpected", "hash_mismatch", "held_out_in_cache"):
            if rec[k]:
                bad.append(f"{c}: {k} {rec[k][:3]}")
        if dup:
            bad.append(f"{c}: {dup} duplicate rows")
    report["violations"] = bad
    Path(a.out).write_text(json.dumps(report, indent=1, sort_keys=True) + "\n")
    sys.stdout.write(json.dumps({c: {k: (len(v) if isinstance(v, list) else v) for k, v in r.items()} for c, r in report.items() if c != "violations"}, indent=1) + f"\nviolations: {bad}\n")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
