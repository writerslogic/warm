# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import argparse
import ast
import base64
import codecs
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import sys
import tempfile


PREFIX_CAP = 32 * 1024 * 1024
HISTORY_CAP = 126200
INPUT_CAP = 127200
DATA_BYTES = 2737100077
DATA_SHA = "9d79e5524794a2e6900a3aa9cb7d9152c5a3e8319c9a87c25494ba1eacee495f"
REVISION = "98d7416c24c778c2fee6e6f3006e7a073259d48f"
DATA_URL = f"https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/{REVISION}/longmemeval_m_cleaned.json"
UPSTREAM_COMMIT = "9e0b455f4ef0e2ab8f2e582289761153549043fc"
UPSTREAM_SHA = "4f1eb3c69d7ad40f04065b9c0bc86f6582441018fc6ff751d162d66c95baf672"
BPE_SHA = "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d"
SESSION_TEMPLATE = "\n### Session {}:\nSession Date: {}\nSession Content:\n{}\n"
PROMPT_TEMPLATE = ("I will give you several history chats between you and a user. Please answer the question based on the "
                   "relevant chat history. Answer the question step by step: first extract all the relevant information, "
                   "and then reason over the information to get the answer.\n\n\nHistory Chats:\n\n{}\n\nCurrent Date: {}\n"
                   "Question: {}\nAnswer (step by step):")
PATTERN = "|".join([
    r"[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]*[\p{Ll}\p{Lm}\p{Lo}\p{M}]+(?i:'s|'t|'re|'ve|'m|'ll|'d)?",
    r"[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]+[\p{Ll}\p{Lm}\p{Lo}\p{M}]*(?i:'s|'t|'re|'ve|'m|'ll|'d)?",
    r"\p{N}{1,3}", r" ?[^\s\p{L}\p{N}]+[\r\n/]*", r"\s*[\r\n]+", r"\s+(?!\S)", r"\s+",
])
SPECIAL = {"<|endoftext|>": 199999, "<|endofprompt|>": 200018}
FIELDS = {"question_id", "question_type", "question", "answer", "question_date", "haystack_dates",
          "haystack_session_ids", "haystack_sessions", "answer_session_ids"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True, allow_nan=False).encode("utf-8")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON object key")
        result[key] = value
    return result


def reject_constant(_):
    raise ValueError("nonfinite JSON constant")


DECODER = json.JSONDecoder(object_pairs_hook=unique_object, parse_constant=reject_constant)


def bounded_file(path, limit):
    require(path.is_file() and 0 < path.stat().st_size <= limit, "input missing or outside byte limit")
    with path.open("rb") as handle:
        data = handle.read(limit + 1)
    require(0 < len(data) <= limit, "input changed beyond byte limit")
    return data


def input_receipt(path, data):
    return {"path": str(path.resolve()), "bytes": len(data), "sha256": sha(data)}


def first_record(data):
    utf8 = codecs.getincrementaldecoder("utf-8")("strict")
    text = ""
    for offset in range(0, len(data), 1024 * 1024):
        text += utf8.decode(data[offset:offset + 1024 * 1024], final=False)
        start = len(text) - len(text.lstrip(" \t\r\n"))
        require(start < len(text) and text[start] == "[", "prefix must start at JSON array byte zero")
        start += 1
        while start < len(text) and text[start] in " \t\r\n":
            start += 1
        if start == len(text):
            continue
        require(text[start] == "{", "first array record must be an object")
        try:
            entry, end = DECODER.raw_decode(text, start)
        except json.JSONDecodeError:
            continue
        after = end
        while after < len(text) and text[after] in " \t\r\n":
            after += 1
        if after == len(text):
            continue
        require(text[after] == ",", "first M record must be followed by array delimiter")
        begin_byte = len(text[:start].encode("utf-8"))
        end_byte = len(text[:end].encode("utf-8"))
        return entry, {"ordinal": 0, "begin_byte_inclusive": begin_byte, "end_byte_exclusive": end_byte,
                       "bytes": end_byte - begin_byte, "sha256": sha(data[begin_byte:end_byte])}
    raise ValueError("first complete record and delimiter unavailable within fixed prefix")


def validate_provenance(value, data):
    require(isinstance(value, dict) and value.get("experiment") == "E42"
            and value.get("decision") == "accept-pinned-counting-inputs", "E42 fetch receipt required")
    require(value.get("revision") == REVISION, "pinned dataset revision")
    require(value.get("requested_range") == {"start": 0, "end": PREFIX_CAP - 1}, "fixed prefix request required")
    require(all(type(value["requested_range"][field]) is int for field in ("start", "end")), "invalid range endpoint type")
    prefix = value.get("prefix")
    require(isinstance(prefix, dict) and prefix.get("source_url") == DATA_URL, "pinned dataset URL")
    require(type(prefix.get("http_status")) is int and prefix["http_status"] == 206, "HTTP partial response required")
    require(prefix.get("content_encoding") == "identity", "encoded response forbidden")
    content_range = prefix.get("content_range")
    require(isinstance(content_range, str), "missing Content-Range")
    match = re.fullmatch(r"bytes 0-([0-9]+)/([0-9]+)", content_range)
    require(match is not None, "invalid Content-Range")
    require(int(match[1]) + 1 == len(data) == PREFIX_CAP and int(match[2]) == DATA_BYTES, "Content-Range size mismatch")
    require(type(prefix.get("bytes")) is int and prefix["bytes"] == len(data), "prefix byte receipt mismatch")
    require(prefix.get("sha256") == sha(data), "prefix digest mismatch")
    require(isinstance(prefix.get("retrieved_at"), str)
            and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]+)?(?:Z|\+00:00)", prefix["retrieved_at"]), "retrieval timestamp")
    require(value.get("full_file_sha256_verified") is False, "prefix cannot verify full dataset digest")


def validate_dependency_receipt(provenance, key, data, url):
    receipt = provenance.get(key)
    require(isinstance(receipt, dict) and receipt.get("source_url") == url, "dependency source URL mismatch")
    require(type(receipt.get("http_status")) is int and receipt["http_status"] == 200
            and receipt.get("content_encoding") == "identity", "dependency response mismatch")
    require(type(receipt.get("bytes")) is int and receipt["bytes"] == len(data)
            and receipt.get("sha256") == sha(data), "dependency receipt digest or size mismatch")


def clean_entry(entry):
    require(isinstance(entry, dict) and set(entry) == FIELDS, "unexpected M record fields")
    for field in ("question_id", "question_type", "question", "question_date", "answer"):
        require(isinstance(entry[field], str), "invalid metadata type")
    require(0 < len(entry["question"]) <= PREFIX_CAP and 0 < len(entry["question_date"]) <= 128, "invalid question framing")
    require(isinstance(entry["answer_session_ids"], list)
            and all(isinstance(item, str) for item in entry["answer_session_ids"]), "invalid excluded answer metadata")
    sessions, dates, ids = (entry[field] for field in ("haystack_sessions", "haystack_dates", "haystack_session_ids"))
    require(all(isinstance(items, list) for items in (sessions, dates, ids)), "invalid history arrays")
    require(0 < len(sessions) == len(dates) == len(ids) <= 10000, "unaligned or oversized history arrays")
    copied, expected, turns = [], [], 0
    for si, (session, date, sid) in enumerate(zip(sessions, dates, ids, strict=True)):
        require(isinstance(date, str) and 0 < len(date) <= 128 and all(ord(c) >= 32 for c in date), "invalid session date")
        require(isinstance(sid, str) and 0 < len(sid) <= 4096, "invalid excluded session identity")
        require(isinstance(session, list) and 0 < len(session) <= 100000, "invalid session turns")
        neutral = []
        for ti, turn in enumerate(session):
            require(isinstance(turn, dict) and {"role", "content"} <= set(turn) <= {"role", "content", "has_answer"}, "unknown turn fields")
            require(turn["role"] in ("user", "assistant") and isinstance(turn["content"], str), "invalid role or content")
            require("has_answer" not in turn or type(turn["has_answer"]) is bool, "invalid excluded annotation")
            turn["content"].encode("utf-8", "strict")
            neutral.append({key: value for key, value in turn.items() if key in ("role", "content")})
            expected.append([si, ti, date, turn["role"], turn["content"]])
            turns += 1
            require(turns <= 100000, "too many physical turns")
        copied.append((si, date, neutral))
    copied.sort(key=lambda item: item[1])
    observed = sorted([[si, ti, date, turn["role"], turn["content"]]
                       for si, date, session in copied for ti, turn in enumerate(session)], key=lambda row: (row[0], row[1]))
    require(expected == observed, "physical occurrence/date/role/text conservation failure")
    coverage = {"sessions": len(sessions), "turns": turns, "serialized_sessions": len(copied),
                "serialized_turns": len(observed), "physical_coverage": 1.0,
                "identity_date_role_text_losses": 0, "source_session_id_duplicates_preserved": len(ids) - len(set(ids)),
                "occurrence_payload_sha256": sha(canonical(expected)), "ordering": "stable sort by original date string"}
    return copied, coverage


def load_encoding(path, provenance):
    require(importlib.metadata.version("tiktoken") == "0.14.0", "requires tiktoken0.14.0")
    from tiktoken import Encoding, _tiktoken
    import tiktoken.core
    raw = bounded_file(path, 8 * 1024 * 1024)
    require(sha(raw) == BPE_SHA, "o200k_base artifact digest mismatch")
    validate_dependency_receipt(provenance, "tokenizer", raw,
                                "https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken")
    ranks = {}
    for line in raw.splitlines():
        token, rank = line.split()
        token = base64.b64decode(token, validate=True)
        require(token not in ranks, "duplicate BPE token")
        ranks[token] = int(rank)
    require(len(ranks) == 199998 and set(ranks.values()) == set(range(199998)), "BPE rank domain mismatch")
    encoding = Encoding("o200k_base", pat_str=PATTERN, mergeable_ranks=ranks, special_tokens=SPECIAL)
    receipt = {"package": "tiktoken", "version": "0.14.0", "encoding": "o200k_base",
               "artifact": input_receipt(path, raw), "pattern_sha256": sha(PATTERN.encode()), "special_tokens": SPECIAL,
               "construction": "local validated BPE bytes and Encoding constructor; no get_encoding or network loader",
               "encode_options": {"allowed_special": ["<|endoftext|>"], "disallowed_special": "all except allowed"},
               "core_source_sha256": sha(Path(tiktoken.core.__file__).read_bytes()),
               "native_extension_sha256": sha(Path(_tiktoken.__file__).read_bytes())}
    return encoding, receipt


def count(args):
    prefix = bounded_file(args.prefix, PREFIX_CAP)
    provenance_bytes = bounded_file(args.provenance, 1024 * 1024)
    provenance = DECODER.decode(provenance_bytes.decode("utf-8"))
    validate_provenance(provenance, prefix)
    upstream = bounded_file(args.upstream_source, 1024 * 1024)
    require(sha(upstream) == UPSTREAM_SHA, "pinned renderer source digest mismatch")
    validate_dependency_receipt(provenance, "generation", upstream,
                                f"https://raw.githubusercontent.com/xiaowu0162/LongMemEval/{UPSTREAM_COMMIT}/src/generation/run_generation.py")
    constants = {node.value for node in ast.walk(ast.parse(upstream)) if isinstance(node, ast.Constant) and isinstance(node.value, str)}
    require(SESSION_TEMPLATE in constants and PROMPT_TEMPLATE in constants, "renderer template mismatch")
    entry, record = first_record(prefix)
    copied, coverage = clean_entry(entry)
    history = "".join(SESSION_TEMPLATE.format(i + 1, date, "\n" + json.dumps(session))
                      for i, (_, date, session) in enumerate(copied))
    prompt = PROMPT_TEMPLATE.format(history, entry["question_date"], entry["question"])
    encoding, tokenizer = load_encoding(args.tokenizer_artifact, provenance)
    history_tokens = len(encoding.encode(history, allowed_special={"<|endoftext|>"}))
    prompt_tokens = len(encoding.encode(prompt, allowed_special={"<|endoftext|>"}))
    rejected = history_tokens > HISTORY_CAP
    return {"schema_version": 1, "hypothesis": "H-C-3", "decision": "reject-C3-fixed-serialization-budget" if rejected else "inconclusive-first-record-fits",
            "scope": "one fixed first physical record; infrastructure feasibility only; no model or scores",
            "counterexample_to_all500_history_gate": rejected, "all500_gate_passed": False,
            "limits": {"prefix_bytes": PREFIX_CAP, "history_tokens": HISTORY_CAP, "aggregate_input_tokens": INPUT_CAP, "aggregate_output_tokens": 800},
            "inputs": {"prefix": input_receipt(args.prefix, prefix), "response_provenance": input_receipt(args.provenance, provenance_bytes),
                       "upstream_source": input_receipt(args.upstream_source, upstream), "counter_sha256": sha(Path(__file__).read_bytes())},
            "source_identity": {"dataset_url": DATA_URL, "revision": REVISION, "expected_full_bytes": DATA_BYTES,
                                "expected_full_sha256_metadata_only": DATA_SHA, "full_file_sha256_verified": False,
                                "authentication": "pinned HTTPS range provenance and local prefix digest; not independent full-object verification",
                                "http_status": provenance["prefix"]["http_status"], "content_range": provenance["prefix"]["content_range"], "retrieved_at_utc": provenance["prefix"]["retrieved_at"]},
            "record": record, "coverage": coverage, "tokenizer": tokenizer,
            "serialization": {"upstream_commit": UPSTREAM_COMMIT, "options": "orig-session/json/cot=true/useronly=false/con=false/merge=none; all physical sessions; no truncation",
                              "history_bytes": len(history.encode()), "history_sha256": sha(history.encode()), "history_tokens": history_tokens,
                              "untruncated_prompt_text_tokens": prompt_tokens, "prompt_text_sha256": sha(prompt.encode()),
                              "prompt_text_exceeds_aggregate_input_cap": prompt_tokens > INPUT_CAP,
                              "provider_chat_envelope_tokens_counted": False, "prompt_count_is_complete_user_text_only": True},
            "claim_limits": ["Counterexample applies only to the fixed serialization and encoding, not every lossless encoding or query-conditioned method.",
                             "Partitioning or repeated calls cannot lower an additive history accounting total that must charge this serialized history at least once; no arbitrary repartition-tokenization bound is claimed.",
                             "A fitting first record cannot establish all500 coverage or the full cost envelope; provider framing and further system work remain uncounted.",
                             "No key, judge, accuracy, semantic completeness, global competitor or fresh holdout claim."],
            "paid_calls": 0, "model_calls": 0, "final_answers_generated_or_replayed": False}


def write_new(path, value):
    require(not path.exists(), "output already exists")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent, delete=False) as handle:
            temporary = Path(handle.name)
            json.dump(value, handle, indent=2, ensure_ascii=False, allow_nan=False)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.link(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser()
    for name in ("prefix", "provenance", "upstream-source", "tokenizer-artifact", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    require(args.output.resolve() not in {getattr(args, name).resolve() for name in ("prefix", "provenance", "upstream_source", "tokenizer_artifact")}, "output aliases input")
    try:
        result = count(args)
    except (ValueError, OSError, UnicodeError, RecursionError, ImportError, KeyError, TypeError) as error:
        result = {"schema_version": 1, "hypothesis": "H-C-3", "decision": "reject-counter-integrity",
                  "error_type": type(error).__name__, "error": str(error) if type(error) is ValueError else "input or dependency validation failed",
                  "counterexample_to_all500_history_gate": False, "all500_gate_passed": False,
                  "counter_sha256": sha(Path(__file__).read_bytes()), "model_calls": 0, "paid_calls": 0}
    write_new(args.output, result)
    print(json.dumps({"decision": result["decision"], "output": str(args.output)}))
    return 2 if result["decision"] == "reject-counter-integrity" else 0


if __name__ == "__main__":
    sys.exit(main())
