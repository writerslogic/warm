# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Offline, fixed-prefix quality study. Never imports model or network clients."""

import gzip
import hashlib
import json
import platform
import subprocess
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical_rows(ids, rows):
    seen, keep = {}, []
    if len(ids) != len(rows):
        raise ValueError("row identities do not align")
    for i, identity in enumerate(ids):
        if identity in seen:
            if rows[i].tobytes() != rows[seen[identity]].tobytes():
                raise ValueError("repeated turn ID has different cached vector bytes")
        else:
            seen[identity] = i
            keep.append(i)
    return keep


def cosine(rows, query, dimension):
    rows, query = np.asarray(rows, dtype=np.float64), np.asarray(query, dtype=np.float64)
    if rows.ndim != 2 or query.ndim != 1 or rows.shape[1] != query.size:
        raise ValueError("incompatible vector shapes")
    if not 1 <= dimension <= query.size:
        raise ValueError("invalid prefix dimension")
    matrix, vector = rows[:, :dimension], query[:dimension]
    if not np.isfinite(matrix).all() or not np.isfinite(vector).all():
        raise ValueError("non-finite vector")
    lengths = np.sqrt(np.sum(matrix * matrix, axis=1, dtype=np.float64))
    length = np.sqrt(np.sum(vector * vector, dtype=np.float64))
    if np.any(lengths == 0) or length == 0:
        raise ValueError("zero-norm prefix")
    normalized = matrix / lengths[:, None]
    return np.sum(normalized * (vector / length), axis=1, dtype=np.float64)


def order(scores, ids, subset=None):
    if len(scores) != len(ids) or len(set(ids)) != len(ids):
        raise ValueError("IDs must be unique and aligned with scores")
    candidates = range(len(ids)) if subset is None else subset
    return sorted(candidates, key=lambda i: (-float(scores[i]), ids[i], i))


def retrieve(prefix_scores, full_scores, ids, candidates, k):
    if k < 1 or candidates < k:
        raise ValueError("candidate limit must cover positive k")
    shortlist = order(prefix_scores, ids)[:candidates]
    return shortlist, order(full_scores, ids, shortlist)[:k]


def gold_metrics(ids, session_of, gold_turns, gold_sessions):
    if not gold_turns and not gold_sessions:
        raise ValueError("positive-gold metrics need positive labels")
    result = {}
    for level, returned, gold in [
        ("turn", set(ids), set(gold_turns)),
        ("session", {session_of[tid] for tid in ids}, set(gold_sessions)),
    ]:
        if not gold:
            result.update({f"{level}_any": None, f"{level}_all": None, f"{level}_fraction": None})
            continue
        hits = len(returned & gold)
        result.update({f"{level}_any": float(hits > 0), f"{level}_all": float(hits == len(gold)), f"{level}_fraction": hits / len(gold)})
    return result


def mean_metrics(rows):
    return {name: float(np.mean([r[name] for r in rows if r[name] is not None])) for name in rows[0]}


def run():
    plan_path = HERE / "plan.json"
    plan = json.loads(plan_path.read_text())
    if any((HERE / name).exists() for name in ["per-query.jsonl.gz", "summary.json", "run.json"]):
        raise RuntimeError("frozen result already exists; do not rerun")
    for spec in plan["inputs"].values():
        assert sha256(spec["path"]) == spec["sha256"], spec["path"]
    assert sha256(HERE / plan["mapping"]["file"]) == plan["mapping"]["sha256"]
    for name, expected in plan["scripts"].items():
        assert sha256(HERE / name) == expected, name
    assert platform.python_version() == plan["environment"]["python"]
    assert np.__version__ == plan["environment"]["numpy"]
    assert not subprocess.check_output(["git", "status", "--porcelain", "--", str(HERE)], cwd=HERE).strip()
    plan_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=HERE, text=True).strip()
    mapping = json.loads(gzip.decompress((HERE / plan["mapping"]["file"]).read_bytes()))
    turns = np.memmap(plan["inputs"]["turn_vectors"]["path"], dtype="<f2", mode="r", shape=(plan["turn_occurrences"], plan["source_dimension"]))
    queries = np.memmap(plan["inputs"]["query_vectors"]["path"], dtype="<f2", mode="r", shape=(plan["queries"], plan["source_dimension"]))
    records, cells, baseline_gold = [], {}, []
    for q in mapping:
        all_ids = [tid for session in q["sessions"] for tid in session["tids"]]
        session_of = {tid: session["sid"] for session in q["sessions"] for tid in session["tids"]}
        original_rows = turns[q["row_start"]:q["row_end"]]
        assert canonical_rows(all_ids, original_rows) == q["eligible_local_rows"]
        ids = [all_ids[i] for i in q["eligible_local_rows"]]
        rows, query = original_rows[q["eligible_local_rows"]], queries[q["query_row"]]
        assert len(rows) == len(ids) >= plan["final_k"]
        full = cosine(rows, query, plan["source_dimension"])
        baseline = order(full, ids)[:plan["final_k"]]
        baseline_ids = [ids[i] for i in baseline]
        baseline_metrics = None if q["abstention"] else gold_metrics(baseline_ids, session_of, q["gold_turn_ids"], q["gold_session_ids"])
        if baseline_metrics is not None:
            baseline_gold.append(baseline_metrics)
        record = {"qid": q["qid"], "qtype": q["qtype"], "abstention": q["abstention"], "eligible_turns": len(ids), "full_top10": baseline_ids, "baseline_gold": baseline_metrics, "routes": []}
        for dimension in plan["dimensions"]:
            prefix = full if dimension == plan["source_dimension"] else cosine(rows, query, dimension)
            for limit in plan["candidate_limits"]:
                shortlist, ranked = retrieve(prefix, full, ids, limit, plan["final_k"])
                missed = [ids[i] for i in baseline if i not in set(shortlist)]
                ranked_ids = [ids[i] for i in ranked]
                coverage = (len(baseline) - len(missed)) / len(baseline)
                overlap = len(set(ranked_ids) & set(baseline_ids)) / len(baseline)
                assert coverage == overlap
                agreement = ranked_ids == baseline_ids
                assert agreement == (not missed)
                if dimension == plan["source_dimension"]:
                    assert agreement
                metrics = None if q["abstention"] else gold_metrics(ranked_ids, session_of, q["gold_turn_ids"], q["gold_session_ids"])
                route = {"dimension": dimension, "candidates": limit, "candidate_coverage": coverage, "final_overlap": overlap, "ordered_agreement": agreement, "missed_full_top10_ids": missed, "final_top10": ranked_ids, "gold": metrics}
                record["routes"].append(route)
                cells.setdefault((dimension, limit), []).append((q["qid"], route))
        records.append(record)
    assert len(records) == plan["queries"] and len(baseline_gold) == plan["positive_gold_queries"]
    baseline_summary = mean_metrics(baseline_gold)
    assert sum(r["turn_any"] is not None for r in baseline_gold) == plan["positive_turn_gold_queries"]
    summary = {"questions": len(records), "positive_session_gold_questions": len(baseline_gold), "positive_turn_gold_questions": plan["positive_turn_gold_queries"], "abstentions_in_agreement": sum(q["abstention"] for q in records), "baseline_gold": baseline_summary, "routes": []}
    for (dimension, limit), values in cells.items():
        worst = min(route["candidate_coverage"] for _, route in values)
        gold = mean_metrics([route["gold"] for _, route in values if route["gold"] is not None])
        summary["routes"].append({
            "dimension": dimension, "candidates": limit,
            "mean_candidate_coverage": float(np.mean([r["candidate_coverage"] for _, r in values])),
            "worst_candidate_coverage": worst,
            "worst_query_ids": [qid for qid, r in values if r["candidate_coverage"] == worst],
            "missed_full_top10_total": sum(len(r["missed_full_top10_ids"]) for _, r in values),
            "ordered_agreement_queries": sum(r["ordered_agreement"] for _, r in values),
            "mean_final_overlap": float(np.mean([r["final_overlap"] for _, r in values])),
            "gold": gold, "gold_delta_from_full": {key: gold[key] - baseline_summary[key] for key in gold},
        })
    payload = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in records)
    (HERE / "per-query.jsonl.gz").write_bytes(gzip.compress(payload.encode(), mtime=0))
    (HERE / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    receipt = {"plan_commit": plan_commit, "plan_sha256": sha256(plan_path), "single_frozen_run": True, "python": platform.python_version(), "numpy": np.__version__, "scripts": plan["scripts"], "outputs": {name: sha256(HERE / name) for name in ["per-query.jsonl.gz", "summary.json"]}}
    (HERE / "run.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"questions": len(records), "routes": len(cells), "positive_gold": len(baseline_gold), "plan_commit": plan_commit}))


if __name__ == "__main__":
    run()
