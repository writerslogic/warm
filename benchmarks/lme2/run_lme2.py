#!/usr/bin/env python3
"""Stage driver around the upstream LongMemEval-V2 harness.

stages:
  prompts  build memory + queries, write prompt_rows.jsonl, stop before the reader
  full     rerun the harness; reader outputs come from --reader-cache, judge calls go
           through a resumable cache with a spend ledger (or are collected for batching)
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = ROOT / "upstream"
sys.path.insert(0, str(UPSTREAM))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import memory_modules  # noqa: E402,F401
import warm_hybrid  # noqa: E402,F401
import warm_slice  # noqa: E402,F401
import warm_notes  # noqa: E402,F401
from data.public_data import materialize_runtime_haystack, materialize_runtime_questions, write_json  # noqa: E402

PRESETS = {
    "warm_balanced": {"top_chunks": 14, "max_context_chars": 24000},
    "warm_wide": {"top_chunks": 40, "max_context_chars": 90000},
    "slice8": {"top_states": 8},
    "slice_r0_24": {"top_states": 24, "radius": 0, "max_context_chars": 320000},
    "slice12_sb3": {"top_states": 12, "state_bm25_weight": 3.0, "max_context_chars": 320000},
    "slice12i": {"top_states": 12, "max_context_chars": 320000, "screens_root": "/Volumes/A/.hms-lb/lme2/data/screenshots", "image_hits": 6},
    "slice6": {"top_states": 6},
    "slice12": {"top_states": 12, "max_context_chars": 320000},
    "warm_wide_nb": {"top_chunks": 30, "max_context_chars": 90000, "neighbors": 1},
}
R6 = "/Volumes/A/.hms-lb/lme2/round6"
SCR = "/Volumes/A/.hms-lb/lme2/data/screenshots"
_NO = {"use_notes": False, "use_support": False, "max_context_chars": 320000}
_WN = {"notes_path": R6 + "/notes.json", "support_cache": R6 + "/support_cache.json", "support_pending": R6 + "/support_pending.json"}
PRESETS.update({
    "wn_a": {**_NO, "top_states": 6, "expand_hits": 6},
    "wn_b": {**_NO, "top_states": 8, "expand_hits": 4},
    "wn_c": {**_NO, "top_states": 10, "expand_hits": 3},
    "wn_d": {**_NO, "top_states": 12, "expand_hits": 4},
    "wn_e": {**_NO, "top_states": 6, "expand_hits": 3, "radius": 1},
})
for _k in ("a", "b", "c", "d", "e"):   # notes + support variants
    PRESETS[f"wns_{_k}"] = {**PRESETS[f"wn_{_k}"], **_WN, "use_notes": True, "use_support": True, "screens_root": SCR, "image_hits": 3}
    PRESETS[f"wnn_{_k}"] = {**PRESETS[f"wn_{_k}"], **_WN, "use_notes": True, "use_support": False, "screens_root": SCR, "image_hits": 3}
PRICE_IN, PRICE_OUT = 1.75 / 1e6, 14.0 / 1e6
LEDGER = ROOT / "openai-spend.json"
LEDGER_LOCK = threading.Lock()


def memory_config(system: str, cache_dir: Path) -> dict:
    if system == "no_retrieval":
        return {"memory_type": "no_retrieval", "memory_params": {}}
    if system.startswith("wn"):
        return {"memory_type": "warm_notes", "memory_params": {"cache_dir": str(cache_dir), **PRESETS[system]}}
    if system.startswith("slice"):
        return {"memory_type": "warm_slice", "memory_params": {"cache_dir": str(cache_dir), **PRESETS[system]}}
    return {"memory_type": "warm_hybrid", "memory_params": {"cache_dir": str(cache_dir), **PRESETS[system]}}


def ledger_read() -> dict:
    if LEDGER.exists():
        return json.loads(LEDGER.read_text())
    return {"cap_usd": 60.0, "spend_usd": 0.0, "prompt_tokens": 0, "completion_tokens": 0, "calls": 0, "batches": []}


def ledger_add(prompt_tokens: int, completion_tokens: int, discount: float = 1.0) -> None:
    with LEDGER_LOCK:
        d = ledger_read()
        d["prompt_tokens"] += prompt_tokens
        d["completion_tokens"] += completion_tokens
        d["calls"] += 1
        d["spend_usd"] = round(d["spend_usd"] + discount * (prompt_tokens * PRICE_IN + completion_tokens * PRICE_OUT), 6)
        LEDGER.write_text(json.dumps(d, indent=2) + "\n")


def install_judge_patch(judge_cache: Path, mode: str, collect_path: Path | None, system: str, questions: list[dict]) -> None:
    import evaluation.qa_eval_metrics as qm

    cache: dict[str, dict] = {}
    if judge_cache.exists():
        for line in judge_cache.read_text().splitlines():
            if line.strip():
                r = json.loads(line)
                cache[r["key"]] = r
    qtexts = [(q["id"], q["question"] if isinstance(q["question"], str) else q["question"]["text"]) for q in questions]
    lock = threading.Lock()

    def qid_of(messages: list[dict]) -> str:
        body = messages[-1]["content"]
        for qid, text in qtexts:
            if text in body:
                return qid
        return "unknown"

    def patched(*, client, model, messages, max_completion_tokens, reasoning_effort, temperature, top_p, timeout_seconds):
        request = {"model": model, "messages": messages, "max_completion_tokens": max_completion_tokens}
        if reasoning_effort is not None:
            request["reasoning_effort"] = reasoning_effort
        if temperature is not None:
            request["temperature"] = temperature
        if top_p is not None:
            request["top_p"] = top_p
        key = hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest()
        with lock:
            hit = cache.get(key)
        if hit:
            return hit["content"]
        if mode == "collect":
            custom_id = f"{qid_of(messages)}__{system}"
            with lock:
                with collect_path.open("a") as fp:
                    fp.write(json.dumps({"custom_id": custom_id, "method": "POST", "url": "/v1/chat/completions", "body": request, "key": key}) + "\n")
            return '{"label": 1, "reason": "collect"}'
        if ledger_read()["spend_usd"] >= ledger_read()["cap_usd"]:
            raise RuntimeError("OpenAI spend cap reached")
        request["timeout"] = timeout_seconds
        response = client.chat.completions.create(**request)
        content = (response.choices[0].message.content or "").strip()
        if not content:
            raise ValueError("Evaluator model returned empty response content.")
        u = response.usage
        ledger_add(int(u.prompt_tokens), int(u.completion_tokens))
        rec = {"key": key, "custom_id": f"{qid_of(messages)}__{system}", "content": content,
               "prompt_tokens": int(u.prompt_tokens), "completion_tokens": int(u.completion_tokens)}
        with lock:
            cache[key] = rec
            with judge_cache.open("a") as fp:
                fp.write(json.dumps(rec) + "\n")
        return content

    qm._call_chat_completion = patched


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("stage", choices=["prompts", "full"])
    ap.add_argument("--system", required=True)
    ap.add_argument("--domain", required=True, choices=["web", "enterprise"])
    ap.add_argument("--tier", default="small")
    ap.add_argument("--data-root", default=str(ROOT / "data"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--question-ids", nargs="*")
    ap.add_argument("--limit", type=int)
    ap.add_argument("--embed-cache", default=str(ROOT / "embed_cache"))
    ap.add_argument("--reader-cache")
    ap.add_argument("--judge-cache")
    ap.add_argument("--judge-mode", choices=["sync", "collect"], default="sync")
    ap.add_argument("--collect-path")
    ap.add_argument("--reader-base-url", default="http://localhost:8023/v1")
    args = ap.parse_args()

    data_root = Path(args.data_root).resolve()
    out = Path(args.out).resolve()
    rt = out / "runtime_inputs"
    rt.mkdir(parents=True, exist_ok=True)
    qids = None
    if args.question_ids:
        qids = [x for raw in args.question_ids for x in raw.split(",") if x]
    questions = materialize_runtime_questions(data_root=data_root, domain=args.domain, question_ids=qids, limit=args.limit, output_path=rt / "questions.json")
    materialize_runtime_haystack(data_root=data_root, tier=args.tier, selected_questions=questions, output_path=rt / "haystack.json")
    cfg = memory_config(args.system, Path(args.embed_cache))
    write_json(rt / "memory_config.json", cfg)

    import evaluation.harness as H

    if args.stage == "prompts":
        async def stop(a, rows):
            raise SystemExit(0)
        H.generate_all_reader_outputs = stop
    else:
        rc = json.loads(Path(args.reader_cache).read_text())

        async def replay(a, rows):
            outs = {}
            for row in rows:
                h = hashlib.sha256(json.dumps(row["messages"], sort_keys=True).encode()).hexdigest()
                rec = rc.get(row["question_id"])
                if rec is None or rec["messages_sha256"] != h:
                    raise RuntimeError(f"reader cache missing or context mismatch for {row['question_id']}")
                outs[row["question_id"]] = rec["output"]
            return outs

        H.generate_all_reader_outputs = replay
        install_judge_patch(Path(args.judge_cache), args.judge_mode, Path(args.collect_path) if args.collect_path else None, args.system, questions)

    argv = [
        "evaluation.harness", "--domain", args.domain,
        "--questions-path", str(rt / "questions.json"), "--haystack-path", str(rt / "haystack.json"),
        "--trajectories-path", str(data_root / "trajectories.jsonl"),
        "--memory-config-path", str(rt / "memory_config.json"), "--output-dir", str(out),
        "--model", "Qwen/Qwen3.5-9B", "--base-url", args.reader_base_url, "--api-key-env", "OPENAI_API_KEY",
        "--temperature", "0.6", "--top-p", "0.95", "--top-k", "20",
        "--max-completion-tokens", "20000", "--memory-context-max-tokens", "200000",
        "--reader-max-concurrent-requests", "16", "--prompt-build-max-workers", "1",
        "--evaluator-model", "gpt-5.2", "--evaluator-api-key-env", "OPENAI_API_KEY",
        "--evaluator-reasoning-effort", "medium", "--evaluator-max-completion-tokens", "4096",
    ]
    old = sys.argv
    sys.argv = argv
    try:
        H.main()
    finally:
        sys.argv = old


if __name__ == "__main__":
    main()
