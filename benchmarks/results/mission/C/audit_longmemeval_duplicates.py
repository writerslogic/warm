import argparse
import hashlib
import json
from collections import defaultdict
from pathlib import Path


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--rankings", type=Path, required=True)
    parser.add_argument("--provenance", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source_bytes = args.source.read_bytes()
    source = json.loads(source_bytes)
    rankings = json.loads(args.rankings.read_text())
    previous = json.loads(args.provenance.read_text())
    if hashlib.sha256(source_bytes).hexdigest() != previous["inputs"]["source"]["sha256"]:
        raise ValueError("source hash differs from E25")
    if hashlib.sha256(args.rankings.read_bytes()).hexdigest() != previous["inputs"]["rankings"]["sha256"]:
        raise ValueError("ranking hash differs from E25")
    expected = {issue["qid"] for issue in previous["structural_issues"] if issue["kind"] == "duplicate_session_id"}
    records = []
    for qi, question in enumerate(source):
        positions = defaultdict(list)
        for si, sid in enumerate(question["haystack_session_ids"]):
            positions[sid].append(si)
        for sid, indexes in positions.items():
            if len(indexes) < 2:
                continue
            occurrences = []
            for si in indexes:
                session = question["haystack_sessions"][si]
                occurrences.append({
                    "session_index": si,
                    "session_pointer": f"/{qi}/haystack_sessions/{si}",
                    "date_pointer": f"/{qi}/haystack_dates/{si}",
                    "date": question["haystack_dates"][si],
                    "full_payload_sha256": digest(session),
                    "role_content_sha256": digest([{key: turn[key] for key in ("role", "content")} for turn in session]),
                    "turn_count": len(session),
                })
            identical_full = len({item["full_payload_sha256"] for item in occurrences}) == 1
            identical_content = len({item["role_content_sha256"] for item in occurrences}) == 1
            identical_dates = len({item["date"] for item in occurrences}) == 1
            relevant = []
            for rank, corpus_id in enumerate(rankings[question["question_id"]]):
                converted = corpus_id.replace("noans_", "answer_")
                ranked_sid, _, turn = converted.rpartition("_")
                if ranked_sid == sid:
                    relevant.append({"rank_1based": rank + 1, "corpus_id": corpus_id, "turn_index": int(turn) - 1})
            classification = "identical_payload_and_date" if identical_full and identical_dates else "identical_reader_payload_and_date_annotations_differ" if identical_content and identical_dates else "different_reader_payload_or_date"
            records.append({
                "question_id": question["question_id"], "question_index": qi, "session_id": sid,
                "occurrences": occurrences, "classification": classification,
                "payload_equal": identical_full, "reader_payload_equal": identical_content, "date_equal": identical_dates,
                "legacy_last_write_winner_session_index": indexes[-1],
                "legacy_shadowed_session_indexes": indexes[:-1],
                "declared_answer_session": sid in question["answer_session_ids"],
                "ranked_references": relevant,
                "top30_reference_count": sum(item["rank_1based"] <= 30 for item in relevant),
                "persistent_21_member": question["question_id"] in previous["persistent_official_cases"],
            })
    observed = {row["question_id"] for row in records}
    if observed != expected:
        raise ValueError("duplicate question set differs from E25")
    conflicts = [row for row in records if row["classification"] == "different_reader_payload_or_date"]
    output = {
        "hypothesis": "H-C-1",
        "decision": "reject-lossy-legacy-session-identity" if conflicts else "accept-duplicate-content-equivalence-only",
        "source_sha256": hashlib.sha256(source_bytes).hexdigest(),
        "provenance_receipt_sha256": hashlib.sha256(args.provenance.read_bytes()).hexdigest(),
        "duplicate_question_count": len(observed), "duplicate_id_group_count": len(records),
        "different_payload_or_date_group_count": len(conflicts),
        "same_reader_payload_and_date_group_count": sum(row["reader_payload_equal"] and row["date_equal"] for row in records),
        "records": records,
        "limits": "No model calls, answer inspection or scoring; equality is canonical decoded-object equality, not raw JSON byte-slice equality. Source-index identities remain necessary even for equivalent payloads. Ranking membership is pre-token-truncation only.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as handle:
        json.dump(output, handle, indent=2)
        handle.write("\n")
    print(json.dumps({key: output[key] for key in ("decision", "duplicate_question_count", "duplicate_id_group_count", "different_payload_or_date_group_count")}))


if __name__ == "__main__":
    main()
