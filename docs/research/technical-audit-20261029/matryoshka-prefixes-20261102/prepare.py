# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Freeze provenance and row identities without computing retrieval scores."""

import gzip
import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path

import numpy as np

from probe import canonical_rows

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
CACHE = Path("/Volumes/A/.hms-bench-cache/holo_dev")
DATA = Path("/Volumes/A/.hms-bench-cache/downloads/longmemeval_s_cleaned.json")
HISTORY = "8ed8ba7ffcf3fb8db1b2d59b3de67351ab3b7a51"


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    if (HERE / "plan.json").exists():
        raise RuntimeError("plan already frozen")
    exported = json.loads((CACHE / "turns_dev.json").read_text())
    raw = {q["question_id"]: q for q in json.loads(DATA.read_text())}
    split_path = ROOT / "benchmarks/public/longmemeval_split.json"
    dev = set(json.loads(split_path.read_text())["dev"])
    assert len(exported["questions"]) == len(dev) == 100
    assert {q["qid"] for q in exported["questions"]} == dev
    mapping = []
    duplicates = []
    vectors = np.memmap(CACHE / "turn_emb.f16", dtype="<f2", mode="r", shape=(24391, 1024))
    row = 0
    for qi, question in enumerate(exported["questions"]):
        source = raw[question["qid"]]
        assert question["question"] == source["question"]
        assert question["qtype"] == source["question_type"]
        assert question["date"] == source["question_date"]
        expected = []
        text_by_id = {}
        represented_answer_sessions = set()
        gold_turns, gold_sessions = [], []
        for sid, date, turns in zip(
            source["haystack_session_ids"], source["haystack_dates"], source["haystack_sessions"], strict=True
        ):
            tids = []
            for i, turn in enumerate(turns):
                if turn["role"] != "user":
                    continue
                tid = f"{sid}_{i + 1}"
                if "answer" in sid and not turn.get("has_answer"):
                    tid = tid.replace("answer", "noans")
                tids.append(tid)
                if tid in text_by_id:
                    assert text_by_id[tid] == turn["content"], "duplicate ID has different source text"
                text_by_id[tid] = turn["content"]
                if "answer" in sid and turn.get("has_answer"):
                    assert sid in source["answer_session_ids"]
                    gold_turns.append(tid)
            if not tids:
                continue
            mapped_sid = sid
            if "answer" in sid and not any("answer" in tid for tid in tids):
                mapped_sid = sid.replace("answer", "noans")
            if sid in source["answer_session_ids"]:
                gold_sessions.append(mapped_sid)
                represented_answer_sessions.add(sid)
            expected.append({"sid": mapped_sid, "date": date, "tids": tids})
        assert expected == question["sessions"], question["qid"]
        ids = [tid for session in expected for tid in session["tids"]]
        keep = canonical_rows(ids, vectors[row:row + len(ids)])
        for i, identity in enumerate(ids):
            if i not in keep:
                duplicates.append({"qid": question["qid"], "turn_id": identity,
                                   "retained_global_row": row + ids.index(identity), "duplicate_global_row": row + i,
                                   "source_content_equal": True, "cached_vector_bytes_equal": True})
        abstention = question["qid"].endswith("_abs")
        assert abstention or gold_sessions
        assert abstention or represented_answer_sessions == set(source["answer_session_ids"]), question["qid"]
        mapping.append({
            "qid": question["qid"], "qtype": question["qtype"], "query_row": qi,
            "row_start": row, "row_end": row + len(ids), "abstention": abstention,
            "eligible_local_rows": keep,
            "sessions": [{"sid": s["sid"], "tids": s["tids"]} for s in expected],
            "gold_turn_ids": sorted(set(gold_turns)), "gold_session_ids": sorted(set(gold_sessions)),
        })
        row += len(ids)
    assert row == 24391
    assert sum(q["abstention"] for q in mapping) == 6
    turn_gold_missing = [q["qid"] for q in mapping if not q["abstention"] and not q["gold_turn_ids"]]
    assert len(turn_gold_missing) == 10
    assert (CACHE / "turn_emb.f16").stat().st_size == row * 1024 * 2
    assert (CACHE / "q_emb.f16").stat().st_size == 100 * 1024 * 2
    mapping_path = HERE / "mapping.json.gz"
    mapping_path.write_bytes(gzip.compress(json.dumps(mapping, separators=(",", ":")).encode(), mtime=0))
    inputs = {
        "turn_vectors": CACHE / "turn_emb.f16", "query_vectors": CACHE / "q_emb.f16",
        "export_mapping": CACHE / "turns_dev.json", "source_dataset": DATA, "dev_split": split_path,
    }
    assert digest(DATA) == "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442"
    historical = {}
    for name in ["holographic_memory_proto.py", "longmemeval_modal.py", "longmemeval_pipeline.py"]:
        path = "benchmarks/public/" + name
        content = subprocess.check_output(["git", "show", f"{HISTORY}:{path}"], cwd=ROOT)
        historical[path] = {"commit": HISTORY, "sha256": hashlib.sha256(content).hexdigest()}
    plan = {
        "schema": 1,
        "purpose": "Quality-only development evidence for prefix candidate search followed by full-vector reranking",
        "base_commit": "81f9a126ead5617e4bdfae3be7d69b0bb6f79d1e",
        "dimensions": [128, 256, 512, 1024], "candidate_limits": [20, 50, 100], "final_k": 10,
        "queries": 100, "positive_gold_queries": 94, "abstention_queries": 6, "turn_occurrences": row,
        "positive_turn_gold_queries": 84,
        "positive_session_gold_queries": 94,
        "answer_session_support": "Every source answer_session_id for all 94 non-abstention queries has at least one cached eligible user turn; all were verified before scoring. Session-level evidence coverage does not establish retrieval of an answer-bearing user turn.",
        "non_abstention_queries_without_eligible_gold_user_turn": turn_gold_missing,
        "eligible_turns_total": sum(len(q["eligible_local_rows"]) for q in mapping),
        "deduplication": {"policy": "Only identical IDs within one question collapse to their first source occurrence after exact content and cached-vector byte equality checks; no cross-question deduplication", "original_occurrences_preserved_in_mapping": True, "duplicates": duplicates},
        "source_dimension": 1024, "source_dtype": "<f2", "math_dtype": "float64",
        "ranking": "L2-normalize each prefix independently; exact cosine; ties by ascending turn ID then local row; rerank shortlisted rows with the same full-vector cosine scores as the full-search baseline",
        "gold": "Original user-turn has_answer labels restricted to answer sessions; original answer_session_ids map to the cached session IDs even when the evidence is assistant-only. Session coverage is measured among the final ten returned turns, not a separate top-ten-session search. Exclude six abstentions from positive-gold metrics; ten assistant-only questions lack any eligible gold user turn, so turn metrics are undefined for them and use denominator 84, while session metrics use denominator 94. All 100 remain in agreement metrics.",
        "metrics": ["candidate coverage of full-vector top10", "ordered final top10 equality", "final top10 overlap", "missed full-top10 IDs", "turn and session recall any/all/fraction at ten returned turns"],
        "decision": "Report every predeclared cell and per-query failures. No tuned acceptance threshold, no unconditional recall guarantee; root decides whether implementation warrants further work.",
        "model": {
            "id": "Qwen/Qwen3-Embedding-0.6B", "revision": "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3",
            "query_prompt": "Instruct: Given a question a user asks about their earlier conversations with an AI assistant, retrieve the user messages that contain the information needed to answer it\nQuery:",
            "document_prompt": "", "pooling": "last token", "max_input_tokens": 2048,
            "cache_generation": "Historical sentence-transformers T4 fp16 inference, L2-normalized then cached fp16; original questions only, no rewritten queries",
            "provenance_limit": "Export has no embedded per-file model-revision receipt; attribution follows the recovered exporter and its pinned model/cache-key implementation, not a new inference verification",
        },
        "dataset": {
            "id": "xiaowu0162/longmemeval-cleaned/longmemeval_s_cleaned.json",
            "revision": "98d7416c24c778c2fee6e6f3006e7a073259d48f",
            "partition": "Previously used frozen DEVELOPMENT split; not held-out validation",
        },
        "inputs": {key: {"path": str(path), "bytes": path.stat().st_size, "sha256": digest(path)} for key, path in inputs.items()},
        "mapping": {"file": mapping_path.name, "sha256": digest(mapping_path), "verification": "All cached question texts/types/dates and every ordered session/turn ID matched the source dataset before scoring; corpus text is not committed"},
        "historical_sources": historical,
        "environment": {"python": platform.python_version(), "numpy": np.__version__, "executable": sys.executable, "uv": subprocess.check_output(["uv", "--version"], text=True).strip()},
        "constraints": ["Single frozen run after independent fixture validation and signed plan commit", "No model downloads, inference, paid services, Modal, LLM judges or native builds", "No latency, memory-performance, answer-accuracy, native-Candle-parity, long-context, held-out or 100K-scale claim"],
    }
    plan["scripts"] = {name: digest(HERE / name) for name in ["prepare.py", "probe.py", "test_probe.py"]}
    (HERE / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    print(json.dumps({"questions": 100, "turn_rows": row, "mapping_sha256": digest(mapping_path), "source_sha256": plan["inputs"]["source_dataset"]["sha256"]}))


if __name__ == "__main__":
    main()
