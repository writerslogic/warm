import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


def read_json(path):
    raw = path.read_bytes()
    return json.loads(raw), {
        "path": str(path.resolve()),
        "sha256": hashlib.sha256(raw).hexdigest(),
        "bytes": len(raw),
    }


def read_jsonl(path):
    raw = path.read_bytes()
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    return rows, {
        "path": str(path.resolve()),
        "sha256": hashlib.sha256(raw).hexdigest(),
        "bytes": len(raw),
    }


def unique_map(rows, key, issues, context):
    ids = [row[key] for row in rows]
    duplicates = [value for value, count in Counter(ids).items() if count > 1]
    if duplicates:
        issues.append({"kind": "duplicate_ids", "context": context, "ids": duplicates})
    return {row[key]: row for row in rows}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--split", type=Path, required=True)
    parser.add_argument("--rankings", type=Path, required=True)
    parser.add_argument("--reports-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    issues = []
    source, source_receipt = read_json(args.source)
    rankings, ranking_receipt = read_json(args.rankings)
    split, split_receipt = read_json(args.split)
    harness_raw = args.harness.read_bytes()
    harness_receipt = {
        "path": str(args.harness.resolve()),
        "sha256": hashlib.sha256(harness_raw).hexdigest(),
        "bytes": len(harness_raw),
    }
    expected = {
        "source": "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442",
        "harness": "ecce9c4c79dc89d99534ac17b383a5cbb5b9f0c69ee98adaf0684742e3d95251",
        "rankings": "edba139e7086ec44ebaa393bd948cf6f1c84de10c1b64a9e87964d2e576fbe66",
        "split": "ef9c00e7bc8d029a4f4cf2c177098c35e3ee347893680e1fe26ee98ed9d16ba7",
    }
    receipts = {
        "source": source_receipt,
        "harness": harness_receipt,
        "rankings": ranking_receipt,
        "split": split_receipt,
    }
    for name, receipt in receipts.items():
        receipt["expected_sha256"] = expected[name]
        receipt["matches_expected"] = receipt["sha256"] == expected[name]
        if not receipt["matches_expected"]:
            issues.append({"kind": "hash_mismatch", "input": name})
    by_id = unique_map(source, "question_id", issues, "source")
    source_ids = set(by_id)
    dev, heldout = set(split["dev"]), set(split["heldout"])
    if len(source) != 500 or len(dev) != 100 or len(heldout) != 400 or dev & heldout or dev | heldout != source_ids:
        issues.append({"kind": "source_split_shape"})
    if set(rankings) != source_ids:
        issues.append({"kind": "ranking_question_set"})
    structures = {}
    for qi, entry in enumerate(source):
        qid = entry["question_id"]
        session_ids = entry["haystack_session_ids"]
        sessions = entry["haystack_sessions"]
        dates = entry["haystack_dates"]
        if len(session_ids) != len(sessions) or len(dates) != len(sessions):
            issues.append({"kind": "session_alignment", "qid": qid})
            continue
        if len(set(session_ids)) != len(session_ids):
            issues.append({"kind": "duplicate_session_id", "qid": qid})
        missing = sorted(set(entry["answer_session_ids"]) - set(session_ids))
        if missing:
            issues.append({"kind": "missing_answer_session", "qid": qid, "ids": missing})
        if len(entry["answer_session_ids"]) != len(set(entry["answer_session_ids"])):
            issues.append({"kind": "duplicate_answer_session_id", "qid": qid})
        mapping = {sid: (si, session) for si, (sid, session) in enumerate(zip(session_ids, sessions))}
        annotated = []
        for si, session in enumerate(sessions):
            for ti, turn in enumerate(session):
                if not isinstance(turn.get("role"), str) or not isinstance(turn.get("content"), str):
                    issues.append({"kind": "turn_schema", "pointer": f"/{qi}/haystack_sessions/{si}/{ti}"})
                if "has_answer" in turn and type(turn["has_answer"]) is not bool:
                    issues.append({"kind": "answer_marker_type", "pointer": f"/{qi}/haystack_sessions/{si}/{ti}"})
                if turn.get("has_answer"):
                    annotated.append((session_ids[si], ti))
        ids = rankings.get(qid, [])
        if len(ids) != len(set(ids)):
            issues.append({"kind": "duplicate_ranked_id", "qid": qid})
        actual_context = set()
        breakdown_context = set()
        for rank, corpus_id in enumerate(ids):
            converted = corpus_id.replace("noans_", "answer_")
            sid, _, number = converted.rpartition("_")
            if sid not in mapping or not number.isdigit() or not 1 <= int(number) <= len(mapping[sid][1]):
                issues.append({"kind": "invalid_ranked_turn", "qid": qid, "id": corpus_id})
                continue
            if rank >= 30:
                continue
            ti = int(number) - 1
            session = mapping[sid][1]
            end = min(ti + 2, len(session))
            start = ti - 1 if ti == 1 and session[0]["role"] == "assistant" else ti
            actual_context.update((sid, i) for i in range(start, end))
            breakdown_context.update(((sid, ti), (sid, ti + 1)))
        marked_sessions = {sid for sid, _ in annotated}
        declared_sessions = set(entry["answer_session_ids"])
        structures[qid] = {
            "source_pointer": f"/{qi}",
            "question_type": entry["question_type"],
            "abstention_id": "_abs" in qid,
            "session_count": len(sessions),
            "turn_count": sum(map(len, sessions)),
            "answer_session_ids": entry["answer_session_ids"],
            "missing_answer_session_ids": missing,
            "has_answer_turn_count": len(annotated),
            "marked_not_declared_session_ids": sorted(marked_sessions - declared_sessions),
            "declared_without_marked_turn_ids": sorted(declared_sessions - marked_sessions),
            "annotation_turns_present_in_preopen_context": len(set(annotated) & actual_context),
            "annotation_turns_present_in_legacy_breakdown": len(set(annotated) & breakdown_context),
            "annotation_pointer_list": [
                f"/{qi}/haystack_sessions/{mapping[sid][0]}/{ti}" for sid, ti in annotated
            ],
            "context_completeness_limit": "Turn-set membership before token truncation; neither semantic sufficiency nor actual full prompt bytes verified.",
        }
    official_sets, inline_sets, run_receipts = [], [], []
    for repeat in (1, 2, 3):
        prefix = args.reports_dir / f"l01-gpt-5-mini-k30-preopen-r{repeat}"
        report, report_receipt = read_json(Path(str(prefix) + ".json"))
        hypotheses, hypothesis_receipt = read_jsonl(Path(str(prefix) + ".hypotheses.jsonl"))
        labels, label_receipt = read_jsonl(Path(str(prefix) + ".hypotheses.jsonl.eval-results-gpt-4o-mini"))
        rows_by_id = unique_map(report["rows"], "qid", issues, f"report_r{repeat}")
        hypotheses_by_id = unique_map(hypotheses, "question_id", issues, f"hypotheses_r{repeat}")
        labels_by_id = unique_map(labels, "question_id", issues, f"labels_r{repeat}")
        for name, mapping in (("report", rows_by_id), ("hypotheses", hypotheses_by_id), ("labels", labels_by_id)):
            if set(mapping) != source_ids:
                issues.append({"kind": "result_question_set", "repeat": repeat, "artifact": name})
        for field, value in (("source_sha256", expected["source"]), ("evaluate_qa_sha256", expected["harness"]), ("rankings_sha256", expected["rankings"]), ("split_sha256", expected["split"])):
            if report.get(field) != value:
                issues.append({"kind": "report_pin_mismatch", "repeat": repeat, "field": field})
        disagreement = []
        for qid in source_ids & set(rows_by_id) & set(hypotheses_by_id) & set(labels_by_id):
            row, hypothesis, label = rows_by_id[qid], hypotheses_by_id[qid], labels_by_id[qid]
            if row["hypothesis"] != hypothesis["hypothesis"] or hypothesis["hypothesis"] != label["hypothesis"]:
                issues.append({"kind": "hypothesis_link_mismatch", "repeat": repeat, "qid": qid})
            if row["qtype"] != by_id[qid]["question_type"]:
                issues.append({"kind": "question_type_mismatch", "repeat": repeat, "qid": qid})
            if type(label["autoeval_label"]["label"]) is not bool:
                issues.append({"kind": "label_type", "repeat": repeat, "qid": qid})
            if type(row["judges"]["gpt-4o-mini"]["correct"]) is not bool:
                issues.append({"kind": "inline_label_type", "repeat": repeat, "qid": qid})
            if label["autoeval_label"]["model"] != "gpt-4o-mini-2024-07-18":
                issues.append({"kind": "judge_model", "repeat": repeat, "qid": qid})
            if row["judges"]["gpt-4o-mini"]["correct"] != label["autoeval_label"]["label"]:
                disagreement.append(qid)
        official_sets.append({qid for qid, row in labels_by_id.items() if row["autoeval_label"]["label"] is False})
        inline_sets.append({qid for qid, row in rows_by_id.items() if row["judges"]["gpt-4o-mini"]["correct"] is False})
        run_receipts.append({
            "repeat": repeat,
            "report": report_receipt,
            "hypotheses": hypothesis_receipt,
            "official_labels": label_receipt,
            "historical_official_negative_count": len(official_sets[-1]),
            "historical_inline_negative_count": len(inline_sets[-1]),
            "inline_vs_official_label_disagreement_ids": sorted(disagreement),
        })
    persistent = set.intersection(*official_sets)
    inline_persistent = set.intersection(*inline_sets)
    output = {
        "hypothesis": "H-C-1",
        "decision": "reject-structural-audit" if issues else "accept-structural-only",
        "kind": "offline provenance and schema audit of existing exposed artifacts; no model calls or new scores",
        "inputs": receipts,
        "structural_question_count": len(source),
        "unique_question_count": len(source_ids),
        "question_type_counts": dict(Counter(row["question_type"] for row in source)),
        "abstention_id_count": sum("_abs" in qid for qid in source_ids),
        "structural_issues": issues,
        "run_artifacts": run_receipts,
        "persistent_selection_definition": "Intersection of false labels in the three official eval-results-gpt-4o-mini files, matching breakdown.py lines 7-13 and 29-30; no new judge invocation.",
        "persistent_official_count": len(persistent),
        "persistent_official_cases": {qid: structures[qid] for qid in sorted(persistent) if qid in structures},
        "persistent_inline_ids": sorted(inline_persistent),
        "official_only_persistent_ids": sorted(persistent - inline_persistent),
        "inline_only_persistent_ids": sorted(inline_persistent - persistent),
        "whole_key_semantics_audited": False,
        "score_ceiling": None,
        "H-C-2_eligibility_count": None,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as handle:
        json.dump(output, handle, indent=2)
        handle.write("\n")
    print(json.dumps({"output": str(args.output), "questions": len(source), "issues": len(issues), "persistent_official": len(persistent), "persistent_inline": len(inline_persistent)}))


if __name__ == "__main__":
    main()
