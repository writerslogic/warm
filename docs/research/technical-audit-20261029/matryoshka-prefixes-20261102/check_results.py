# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Audit saved identities and aggregates with stdlib only; never recompute scores."""

import gzip
import hashlib
import itertools
import json
import math
import statistics
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def close(actual, expected, message):
    require(math.isclose(actual, expected, rel_tol=0, abs_tol=1e-12), message)


def check_gold(returned, mapping, observed):
    if mapping["abstention"]:
        require(observed is None, "abstention entered gold metrics")
        return
    sessions = {tid: session["sid"] for session in mapping["sessions"] for tid in session["tids"]}
    for level, hits in [("turn", set(returned)), ("session", {sessions[tid] for tid in returned})]:
        gold = set(mapping[f"gold_{level}_ids"])
        if not gold:
            require(all(observed[f"{level}_{metric}"] is None for metric in ["any", "all", "fraction"]), "empty gold denominator")
            continue
        found = hits & gold
        expected = {"any": int(bool(found)), "all": int(gold <= hits), "fraction": len(found) / len(gold)}
        for metric, value in expected.items():
            close(observed[f"{level}_{metric}"], value, "gold metric differs from identities")


def gold_mean(rows):
    return {key: statistics.mean(row[key] for row in rows if row[key] is not None) for key in rows[0]}


def main():
    plan = json.loads((HERE / "plan.json").read_text())
    receipt = json.loads((HERE / "run.json").read_text())
    summary = json.loads((HERE / "summary.json").read_text())
    mapping = json.loads(gzip.decompress((HERE / "mapping.json.gz").read_bytes()))
    rows = [json.loads(line) for line in gzip.decompress((HERE / "per-query.jsonl.gz").read_bytes()).splitlines()]
    require(sha(HERE / "plan.json") == receipt["plan_sha256"], "plan hash changed")
    frozen_plan = subprocess.check_output(["git", "show", f"{receipt['plan_commit']}:docs/research/technical-audit-20261029/matryoshka-prefixes-20261102/plan.json"], cwd=HERE)
    require(hashlib.sha256(frozen_plan).hexdigest() == receipt["plan_sha256"], "plan differs from signed pre-run commit")
    require(sha(HERE / "mapping.json.gz") == plan["mapping"]["sha256"], "mapping changed")
    for name, expected in receipt["outputs"].items():
        require(sha(HERE / name) == expected, "result hash changed")
    require(receipt["scripts"] == plan["scripts"], "source receipt differs")
    for name, expected in plan["scripts"].items():
        require(sha(HERE / name) == expected, "frozen script changed")
    require(len(rows) == len(mapping) == plan["queries"] == 100, "question count changed")
    require(len({row["qid"] for row in rows}) == 100, "duplicate question")
    combinations = list(itertools.product(plan["dimensions"], plan["candidate_limits"]))
    cells = {cell: [] for cell in combinations}
    for row, identities in zip(rows, mapping, strict=True):
        require(row["qid"] == identities["qid"], "question order changed")
        require(row["abstention"] == identities["abstention"], "abstention classification changed")
        original = [tid for session in identities["sessions"] for tid in session["tids"]]
        eligible = [original[i] for i in identities["eligible_local_rows"]]
        require(len(eligible) == len(set(eligible)) == row["eligible_turns"], "eligible identities changed")
        baseline = row["full_top10"]
        require(len(baseline) == len(set(baseline)) == plan["final_k"], "baseline size differs")
        require(set(baseline) <= set(eligible), "baseline contains ineligible ID")
        check_gold(baseline, identities, row["baseline_gold"])
        require([(r["dimension"], r["candidates"]) for r in row["routes"]] == combinations, "study cells changed")
        for route in row["routes"]:
            final = route["final_top10"]
            require(len(final) == len(set(final)) == plan["final_k"], "final size differs")
            require(set(final) <= set(eligible), "route contains ineligible ID")
            missed = [tid for tid in baseline if tid not in final]
            require(missed == route["missed_full_top10_ids"], "missed IDs differ")
            overlap = len(set(baseline) & set(final)) / plan["final_k"]
            close(route["candidate_coverage"], overlap, "candidate coverage differs")
            close(route["final_overlap"], overlap, "final overlap differs")
            require(route["ordered_agreement"] == (baseline == final) == (not missed), "ordered agreement differs")
            check_gold(final, identities, route["gold"])
            if route["dimension"] == plan["source_dimension"]:
                require(route["ordered_agreement"], "full dimension control failed")
            cells[(route["dimension"], route["candidates"])].append((row["qid"], route))
    baseline_gold = [row["baseline_gold"] for row in rows if row["baseline_gold"] is not None]
    require(len(baseline_gold) == summary["positive_session_gold_questions"] == 94, "session denominator changed")
    require(sum(row["turn_any"] is not None for row in baseline_gold) == summary["positive_turn_gold_questions"] == 84, "turn denominator changed")
    require(sum(row["abstention"] for row in rows) == summary["abstentions_in_agreement"] == 6, "abstention denominator changed")
    for key, expected in gold_mean(baseline_gold).items():
        close(summary["baseline_gold"][key], expected, "baseline aggregate differs")
    require([(r["dimension"], r["candidates"]) for r in summary["routes"]] == combinations, "summary cells changed")
    for aggregate in summary["routes"]:
        values = cells[(aggregate["dimension"], aggregate["candidates"])]
        worst = min(row["candidate_coverage"] for _, row in values)
        close(aggregate["mean_candidate_coverage"], statistics.mean(row["candidate_coverage"] for _, row in values), "mean coverage differs")
        close(aggregate["mean_final_overlap"], statistics.mean(row["final_overlap"] for _, row in values), "mean overlap differs")
        close(aggregate["worst_candidate_coverage"], worst, "worst coverage differs")
        require(aggregate["worst_query_ids"] == [qid for qid, row in values if row["candidate_coverage"] == worst], "worst-query IDs differ")
        require(aggregate["missed_full_top10_total"] == sum(len(row["missed_full_top10_ids"]) for _, row in values), "miss count differs")
        require(aggregate["ordered_agreement_queries"] == sum(row["ordered_agreement"] for _, row in values), "agreement aggregate differs")
        for key, expected in gold_mean([row["gold"] for _, row in values if row["gold"] is not None]).items():
            close(aggregate["gold"][key], expected, "gold aggregate differs")
            close(aggregate["gold_delta_from_full"][key], expected - summary["baseline_gold"][key], "gold delta differs")
    print(json.dumps({"status": "passed", "method": "stdlib audit of saved identities, hashes, denominators and aggregates; no vector scoring", "questions": len(rows), "query_routes": sum(map(len, cells.values())), "plan_commit": receipt["plan_commit"], "checker_sha256": sha(__file__), "validated_artifacts": {name: sha(HERE / name) for name in ["plan.json", "mapping.json.gz", "per-query.jsonl.gz", "summary.json", "run.json"]}}, indent=2))


if __name__ == "__main__":
    main()
