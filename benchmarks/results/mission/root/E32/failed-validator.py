import argparse
import hashlib
import json
import re
from pathlib import Path

from audit_longmemeval_provenance import read_json, read_jsonl


def require(condition, message):
    if not condition:
        raise ValueError(message)


def at(document, pointer):
    require(isinstance(pointer, str) and pointer.startswith("/"), "invalid pointer")
    value = document
    for key in pointer[1:].split("/"):
        if isinstance(value, list):
            require(re.fullmatch(r"0|[1-9][0-9]*", key) is not None, "invalid index: " + pointer)
            value = value[int(key)]
        else:
            value = value[key]
    return value


def repo_file(repo, relative):
    path = Path(relative)
    require(not path.is_absolute(), "expected repository-relative path")
    resolved = (repo / path).resolve()
    require(resolved.is_relative_to(repo), "path outside repository")
    return resolved


def validate(args):
    audit, audit_input = read_json(args.audit)
    source, source_input = read_json(args.source)
    provenance, provenance_input = read_json(args.provenance)
    require(provenance_input["sha256"] == "9c092c70d6db745f73f7de6ad1d16822a6d424bded047f2048cc02a4d0fab41e", "E25 receipt hash")
    require(source_input["sha256"] == provenance["inputs"]["source"]["sha256"] == audit["source"]["expected_sha256"], "source hash")
    require(len(source) == provenance["structural_question_count"] == 500, "source count")
    repo = args.repo.resolve()
    harness = args.audit.parent / audit["harness"]["local_path"]
    require(hashlib.sha256(harness.read_bytes()).hexdigest() == provenance["inputs"]["harness"]["sha256"] == audit["harness"]["expected_sha256"], "harness hash")
    reports, labels = {}, {}
    inputs = {"audit": audit_input, "source": source_input, "provenance": provenance_input}
    for run in provenance["run_artifacts"]:
        repeat = run["repeat"]
        prefix = f"{audit['selection']['report_directory']}/l01-gpt-5-mini-k30-preopen-r{repeat}"
        report, report_input = read_json(repo_file(repo, prefix + ".json"))
        label, label_input = read_jsonl(repo_file(repo, prefix + ".hypotheses.jsonl.eval-results-gpt-4o-mini"))
        require(report_input["sha256"] == run["report"]["sha256"], "report hash")
        require(label_input["sha256"] == run["official_labels"]["sha256"], "label hash")
        reports[repeat], labels[repeat] = report, label
        inputs[f"report_r{repeat}"], inputs[f"labels_r{repeat}"] = report_input, label_input
    require(set(reports) == {1, 2, 3}, "repeat set")
    expected_ids = set(provenance["persistent_official_cases"])
    reviews = audit["reviewed"]
    ids = [review["id"] for review in reviews]
    require(len(ids) == len(set(ids)) == audit["reviewed_count"] == 21, "review uniqueness/count")
    require(set(ids) == expected_ids == set(audit["selection"]["selected_ids"]), "review selection")
    require(audit["selection"]["selected_count"] == len(audit["selection"]["selected_ids"]) == 21, "selected count")
    require(audit["accepted_corrections"] == 0 and type(audit["accepted_corrections"]) is int, "accepted correction claim")
    for key in ["applied_to_system", "applied_to_official_key", "whole_key_audited"]:
        require(audit[key] is False, "unexpected true flag: " + key)
    for key in ["official_score", "audited_score", "score_ceiling", "H-C-2_eligibility_count"]:
        require(audit[key] is None, "unexpected score/eligibility claim: " + key)
    session_count = evidence_count = output_count = search_match_count = 0
    official_fields = ["question_id", "question_type", "question", "question_date", "answer", "answer_session_ids"]
    for review in reviews:
        pointer = review["official_pointer"]
        require(re.fullmatch(r"/(0|[1-9][0-9]*)", pointer) is not None, "question pointer shape")
        entry = at(source, pointer)
        require(entry["question_id"] == review["id"], "question pointer identity")
        require(pointer == provenance["persistent_official_cases"][review["id"]]["source_pointer"], "E25 question linkage")
        require(review["official"] == {key: entry[key] for key in official_fields}, "official metadata: " + review["id"])
        for key in ["decision", "key_assessment", "judge_assessment", "source_sufficiency", "rationale", "proposed_audited_treatment"]:
            require(isinstance(review[key], str) and review[key].strip(), "missing review rationale")
        sessions = review["full_sessions_reviewed"]
        session_pointers = [item["pointer"] for item in sessions]
        require(len(session_pointers) == len(set(session_pointers)), "duplicate full session")
        covered_ids = set()
        for item in sessions:
            require(item["pointer"].startswith(pointer + "/haystack_sessions/"), "session question mismatch")
            require(len(item["pointer"].split("/")) == 4, "session pointer shape")
            session = at(source, item["pointer"])
            si = int(item["pointer"].split("/")[-1])
            canonical = json.dumps(session, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()
            require(hashlib.sha256(canonical).hexdigest() == item["canonical_json_sha256"], "session content hash")
            require(len(session) == item["turn_count"], "session turn count")
            require(item["session_id"] == entry["haystack_session_ids"][si] and item["date"] == entry["haystack_dates"][si], "session ID/date")
            covered_ids.add(item["session_id"])
        require(set(entry["answer_session_ids"]).issubset(covered_ids), "declared answer session not reviewed")
        evidence_pointers = [item["pointer"] for item in review["evidence"]]
        require(len(evidence_pointers) == len(set(evidence_pointers)), "duplicate evidence pointer")
        for item in review["evidence"]:
            parts = item["pointer"].split("/")
            require(len(parts) == 5 and "/".join(parts[:-1]) in session_pointers, "evidence outside reviewed session")
            turn = at(source, item["pointer"])
            si = int(parts[-2])
            require(item["role"] == turn["role"] and item["content"] == turn["content"], "evidence role/content")
            require(item["has_answer"] is turn.get("has_answer", False), "evidence answer marker")
            require(item["session_id_pointer"] == f"{pointer}/haystack_session_ids/{si}", "ID pointer shape")
            require(item["session_date_pointer"] == f"{pointer}/haystack_dates/{si}", "date pointer shape")
            require(item["session_id"] == at(source, item["session_id_pointer"]), "evidence session ID")
            require(item["session_date"] == at(source, item["session_date_pointer"]), "evidence session date")
        search = review["counterevidence_search"]
        require(search["fields"] == ["content"] and search["roles"] == "all" and search["flags"] == "IGNORECASE", "countersearch scope")
        pattern = re.compile(search["pattern"], re.IGNORECASE)
        expected_matches = [{"pointer": f"{pointer}/haystack_sessions/{si}/{ti}", "role": turn["role"]} for si, session in enumerate(entry["haystack_sessions"]) for ti, turn in enumerate(session) if pattern.search(turn["content"])]
        require(search["matching_turns"] == expected_matches, "countersearch references")
        outputs = review["historical_outputs"]
        require(sorted(output["repeat"] for output in outputs) == [1, 2, 3], "output repeats")
        for output in outputs:
            repeat = output["repeat"]
            prefix = f"{audit['selection']['report_directory']}/l01-gpt-5-mini-k30-preopen-r{repeat}"
            require(output["report"] == prefix + ".json" and output["official_label_file"] == prefix + ".hypotheses.jsonl.eval-results-gpt-4o-mini", "historical file path")
            require(re.fullmatch(r"/rows/(0|[1-9][0-9]*)", output["report_pointer"]) is not None, "report pointer shape")
            row = at(reports[repeat], output["report_pointer"])
            line = output["official_label_line_1based"]
            require(type(line) is int and 1 <= line <= len(labels[repeat]), "label line")
            label = labels[repeat][line - 1]
            require(row["qid"] == label["question_id"] == review["id"], "historical question linkage")
            require(output["hypothesis"] == row["hypothesis"], "hypothesis content")
            require(output["inline_judge"] == row["judges"]["gpt-4o-mini"] and output["inline_judge"]["correct"] is False, "inline negative label")
            require(output["official_label"] == label["autoeval_label"] and output["official_label"]["label"] is False, "official negative label")
            require(output["inline_judge"]["model"] == output["official_label"]["model"] == audit["harness"]["historical_judge"], "judge model metadata")
            require(reports[repeat]["official_commit"] == audit["harness"]["commit"], "harness commit metadata")
        for key in ["accepted_correction", "applied", "scored"]:
            require(review[key] is False, "unexpected item acceptance")
        require(review["score_gain_claim"] is None and review["H-C-2_eligibility"] is None, "unexpected item score claim")
        session_count += len(sessions)
        evidence_count += len(review["evidence"])
        output_count += len(outputs)
        search_match_count += len(expected_matches)
    summary = audit["summary"]
    categorized = [qid for group in summary["categories"].values() for qid in group]
    require(len(categorized) == len(set(categorized)) == 21 and set(categorized) == expected_ids, "category partition")
    require(summary["category_counts"] == {key: len(value) for key, value in summary["categories"].items()}, "category counts")
    require(summary["accepted_corrections"] == 0 and summary["whole_500_question_key_reviewed"] is False and summary["questions_outside_this_semantic_review"] == 479, "summary scope")
    return {"hypothesis": "H-C-1", "decision": "accept-reference-integrity-only", "inputs": inputs, "reviewed_count": len(ids), "full_session_reference_count": session_count, "evidence_turn_reference_count": evidence_count, "historical_output_count": output_count, "countersearch_match_count": search_match_count, "review_category_counts": summary["category_counts"], "semantic_acceptance": False, "accepted_corrections": 0, "scoring_performed": False, "whole_key_audited": False, "H-C-2_eligibility_count": None, "limits": "Checks exact artifact references and metadata only; does not certify semantic judgments, exhaustive counterevidence review, historical retained prompts or scores."}


def main():
    parser = argparse.ArgumentParser()
    for name in ["audit", "source", "provenance", "repo", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    try:
        result = validate(args)
    except (KeyError, IndexError, TypeError, ValueError, OSError) as error:
        result = {"hypothesis": "H-C-1", "decision": "reject-reference-integrity", "error_type": type(error).__name__, "error": str(error), "semantic_acceptance": False, "scoring_performed": False}
    with args.output.open("x") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps({"decision": result["decision"], "output": str(args.output)}))


if __name__ == "__main__":
    main()
