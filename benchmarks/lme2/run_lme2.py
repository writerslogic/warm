#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Stage driver around the upstream LongMemEval-V2 harness for the G1 memory module.

stages:
  prompts  build memory + queries, write <out>/prompt_rows.jsonl, stop before the reader
  full     rerun the harness; reader outputs come from --reader-cache (keyed by question id, checked by messages hash),
           judge calls (gpt-5.2) go through a resumable cache with a spend ledger
Environment: LME2_DATA, LME2_UPSTREAM; OPENAI_API_KEY for the judge (full stage only).
"""
from __future__ import annotations

import argparse
import fileio  # noqa: E402
import hashlib
import json
import os
import sys
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import common  # noqa: E402

common.add_upstream_path()
import memory_modules  # noqa: E402,F401
import g1_memory  # noqa: E402,F401
from data.public_data import materialize_runtime_haystack, materialize_runtime_questions, write_json  # noqa: E402

PRICE_IN, PRICE_OUT = 1.75 / 1e6, 14.0 / 1e6
LEDGER_LOCK = threading.Lock()


def ledger_read(ledger: Path, cap: float) -> dict:
    if ledger.exists():
        return json.loads(ledger.read_text())
    return {"cap_usd": cap, "spend_usd": 0.0, "prompt_tokens": 0, "completion_tokens": 0, "calls": 0, "batches": []}


def ledger_add(ledger: Path, cap: float, prompt_tokens: int, completion_tokens: int) -> None:
    with LEDGER_LOCK:
        d = ledger_read(ledger, cap)
        d["prompt_tokens"] += prompt_tokens
        d["completion_tokens"] += completion_tokens
        d["calls"] += 1
        d["spend_usd"] = round(d["spend_usd"] + (prompt_tokens * PRICE_IN + completion_tokens * PRICE_OUT), 6)
        ledger.write_text(json.dumps(d, indent=2) + "\n")


def install_judge_patch(judge_cache: Path, ledger: Path, cap: float, system: str, questions: list[dict]) -> None:
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
        d = ledger_read(ledger, cap)
        if d["spend_usd"] >= d["cap_usd"]:
            raise RuntimeError("OpenAI spend cap reached")
        request["timeout"] = timeout_seconds
        response = client.chat.completions.create(**request)
        content = (response.choices[0].message.content or "").strip()
        if not content:
            raise ValueError("Evaluator model returned empty response content.")
        u = response.usage
        ledger_add(ledger, cap, int(u.prompt_tokens), int(u.completion_tokens))
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
    ap.add_argument("--domain", required=True, choices=["web", "enterprise"])
    ap.add_argument("--tier", default="small")
    ap.add_argument("--out", required=True)
    ap.add_argument("--question-ids", nargs="*")
    ap.add_argument("--ids-file", help="json list of question ids; only those of --domain are used")
    ap.add_argument("--limit", type=int)
    ap.add_argument("--spans", required=True, help="final spans json (rules.py final-rule or mkp6 output)")
    ap.add_argument("--first", help="reread Evidence Check json (rules.py mkp6 --out-first); reader pass 2 only")
    ap.add_argument("--form-backend", choices=["engine", "reference"], default=os.environ.get("LME2_FORM_BACKEND", "engine"))
    ap.add_argument("--reader-cache")
    ap.add_argument("--judge-cache")
    ap.add_argument("--judge-cap-usd", type=float, default=60.0)
    ap.add_argument("--trust-cache", action="store_true", help="accept a reader output whose rebuilt context hash differs (the frozen judge run set ARCH_TRUST_CACHE=1)")
    args = ap.parse_args()

    data_root = common.data_dir()
    out = Path(args.out).resolve()
    rt = out / "runtime_inputs"
    rt.mkdir(parents=True, exist_ok=True)
    qids = None
    if args.question_ids:
        qids = [x for raw in args.question_ids for x in raw.split(",") if x]
    elif args.ids_file:
        Q = common.questions()
        qids = [i for i in fileio.read_json(args.ids_file) if Q[i]["domain"] == args.domain]
    questions = materialize_runtime_questions(data_root=data_root, domain=args.domain, question_ids=qids, limit=args.limit, output_path=rt / "questions.json")
    materialize_runtime_haystack(data_root=data_root, tier=args.tier, selected_questions=questions, output_path=rt / "haystack.json")
    store = out / "warm_store"
    store.mkdir(exist_ok=True)
    cfg = {"memory_type": "warm_g1_span", "memory_params": {
        "mode": "span_v", "spans": str(Path(args.spans).resolve()), "first": str(Path(args.first).resolve()) if args.first else None,
        "form_backend": args.form_backend, "screens_root": str(data_root / "screenshots"), "store_root": str(store)}}
    write_json(rt / "memory_config.json", cfg)

    import evaluation.harness as H

    if args.stage == "prompts":
        async def stop(a, rows):
            raise SystemExit(0)
        H.generate_all_reader_outputs = stop
    else:
        if not (args.reader_cache and args.judge_cache):
            ap.error("full needs --reader-cache and --judge-cache")
        rc = json.loads(Path(args.reader_cache).read_text())

        async def replay(a, rows):
            outs = {}
            for row in rows:
                h = hashlib.sha256(json.dumps(row["messages"], sort_keys=True).encode()).hexdigest()
                rec = rc.get(row["question_id"])
                if rec is not None and rec["messages_sha256"] != h and args.trust_cache:
                    print(f"trusting pinned reader output despite rebuilt-context hash mismatch for {row['question_id']}", flush=True)
                elif rec is None or rec["messages_sha256"] != h:
                    raise RuntimeError(f"reader cache missing or context mismatch for {row['question_id']}")
                outs[row["question_id"]] = rec["output"]
            return outs

        H.generate_all_reader_outputs = replay
        install_judge_patch(Path(args.judge_cache), out / "openai-spend.json", args.judge_cap_usd, "span_v", questions)

    argv = [
        "evaluation.harness", "--domain", args.domain,
        "--questions-path", str(rt / "questions.json"), "--haystack-path", str(rt / "haystack.json"),
        "--trajectories-path", str(data_root / "trajectories.jsonl"),
        "--memory-config-path", str(rt / "memory_config.json"), "--output-dir", str(out),
        "--model", "Qwen/Qwen3.5-9B", "--base-url", os.environ.get("READER_BASE_URL", "http://localhost:8023/v1"), "--api-key-env", "READER_API_KEY",
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
