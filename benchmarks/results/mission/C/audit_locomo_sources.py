# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Validate external audit provenance and build source review packets; never score answers."""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re


def require(value, message):
    if not value:
        raise ValueError(message)


def read_json(path):
    require(path.stat().st_size <= 20_000_000, f"Input exceeds audit bound: {path.name}")
    return json.loads(path.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = read_json(args.sources / "manifest.json")
    for source in manifest["sources"]:
        name = source["file"]
        require(Path(name).name == name, "Invalid manifest path")
        payload = (args.sources / name).read_bytes()
        require(hashlib.sha256(payload).hexdigest() == source["sha256"], f"Source hash mismatch: {name}")
    data = read_json(args.sources / "locomo10.json")
    require(isinstance(data, list) and len(data) == 10, "Expected ten source conversations")
    packets, mismatches, seen = [], [], set()
    for index, conversation in enumerate(data):
        turns = {}
        sessions = conversation["conversation"]
        for key, session in sessions.items():
            if not re.fullmatch(r"session_\d+", key):
                continue
            require(isinstance(session, list), f"Invalid session in conversation {index}")
            for turn in session:
                identity = turn["dia_id"]
                require(identity not in turns, f"Duplicate source turn in conversation {index}")
                turns[identity] = {"session_date": sessions[key + "_date_time"], **turn}
        for issue in read_json(args.sources / f"errors_conv_{index}.json"):
            identity = issue["question_id"]
            require(identity not in seen, "Duplicate audit question")
            seen.add(identity)
            match = re.fullmatch(r"locomo_(\d+)_qa(\d+)", identity)
            require(match and int(match[1]) == index, "Invalid audit question identity")
            qa_index = int(match[2])
            require(0 <= qa_index < len(conversation["qa"]), "Audit question index outside source")
            qa = conversation["qa"][qa_index]
            errors = []
            for field, expected in (("question", issue["question"]), ("category", issue["category"]), ("answer", issue["golden_answer"])):
                if str(qa.get(field)) != str(expected):
                    errors.append("source_" + field + "_mismatch")
            if qa["evidence"] != issue["cited_evidence"]:
                errors.append("source_citation_mismatch")
            refs = list(dict.fromkeys(issue["cited_evidence"] + issue["correct_evidence"]))
            missing = [ref for ref in refs if ref not in turns]
            if missing:
                errors.append("missing_evidence_reference")
            if errors:
                mismatches.append({"id": identity, "errors": errors, "missing": missing})
            packets.append({"id": identity, "official": qa, "external_proposal": issue,
                            "source_evidence": {ref: turns[ref] for ref in refs if ref in turns},
                            "validation_errors": errors,
                            "review_status": "pending_semantic_review"})
    summary = {"schema_version": 1, "hypothesis": "H-C-1", "claim": "local",
               "dataset_sha256": hashlib.sha256((args.sources / "locomo10.json").read_bytes()).hexdigest(),
               "nonadversarial_questions": sum(qa["category"] in (1, 2, 3, 4) for item in data for qa in item["qa"]),
               "external_issue_count": len(packets),
               "external_issue_types": dict(sorted(Counter(p["external_proposal"]["error_type"] for p in packets).items())),
               "source_link_mismatch_count": len(mismatches), "source_link_mismatches": mismatches,
               "accepted_corrections": 0, "audited_score": None, "official_score": None,
               "limitations": ["Structural agreement is not semantic validation.",
                               "The external 99-error claim and any implied ceiling remain unverified.",
                               "All packets are exposed development data; no fresh holdout was opened.",
                               "No benchmark answer was generated, replayed, graded or altered."]}
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (args.output / "review-packets.json").write_text(json.dumps(packets, indent=2) + "\n")
    print(json.dumps({"issues": len(packets), "source_link_mismatches": len(mismatches),
                      "accepted_corrections": 0, "summary": str(args.output / "summary.json")}))


if __name__ == "__main__":
    main()
