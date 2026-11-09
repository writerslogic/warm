# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy==2.2.6", "tiktoken==0.14.0", "openai==2.8.1", "tqdm==4.67.1", "backoff==2.2.1"]
# ///
"""LongMemEval_S, all 500 questions, under the official generation and judge protocol.

Generalised copy of benchmarks/public/longmemeval_official_qa.py at WARM 425d30e4 (sha256 of the
original: 02e270118b8e6f2b501fcf8c448a9a60c6d39a95cacc096649da06b1177ee05d). The prompt builder,
call cache, spend ledger and Batch API stage are the original bodies byte for byte (extracted by
build/build_qa500.py); what changed:

  * all 500 ids of the frozen split (100 dev + 400 held-out) instead of the 100 dev ids;
  * rankings come from --rankings (JSON {question_id: [turn ids, best first]}), because the Qwen3
    reference pipeline scores the original used exist for the 100 dev questions only;
  * --dry-run counts questions and estimates prompt tokens and spend without any API call;
  * hypotheses are also written as hypotheses.jsonl for the OFFICIAL evaluate_qa.py (the headline
    judge pass); the in-script judges (--judges) use the same get_anscheck_prompt and are a cheap
    cross-check or batch path, not a replacement;
  * every summary is split into dev100 / heldout400 / all500, per judge model.

Frozen protocol: longmemeval_500_frozen_config.json beside this file; record its sha256 before the first API call.
Held-out questions are sealed by docs/evaluation.md until the pipeline is frozen: running this
script on them is the one-shot held-out evaluation, not a development run.
"""

import argparse
import copy
import hashlib
import importlib.util
import json
import os
import statistics
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import tiktoken

HERE = Path(__file__).resolve().parent
SPLIT = Path(os.environ.get("LME_SPLIT", HERE / "longmemeval_split.json"))
SPLIT_SHA256 = "ef9c00e7bc8d029a4f4cf2c177098c35e3ee347893680e1fe26ee98ed9d16ba7"
FROZEN_CONFIG = Path(os.environ.get("LME_FROZEN_CONFIG", HERE / "longmemeval_500_frozen_config.json"))
SOURCE = Path(os.environ.get("LME_SOURCE", Path.home() / ".cache/warm-bench/downloads/longmemeval_s_cleaned.json"))
SOURCE_SHA256 = "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442"
EVALUATE_QA = Path(os.environ.get(
    "LME_EVALUATE_QA", Path.home() / ".cache/warm-bench/downloads/longmemeval_evaluate_qa_9e0b455f4ef0.py"))
EVALUATE_QA_SHA256 = "ecce9c4c79dc89d99534ac17b383a5cbb5b9f0c69ee98adaf0684742e3d95251"
STATE = Path(os.environ.get("LME_STATE", Path.home() / ".cache/warm-bench/longmemeval_500_state"))
CACHE = STATE / "calls"
BATCH_STATE = STATE / "batches"
LEDGER = STATE / "spend_ledger.json"
SPEND_CAP_USD = 30.0  # matches frozen_config.json spend.cap_usd; checked in main()
JUDGE_MODEL = "gpt-4o-2024-08-06"
READERS = {"gpt-4o-2024-08-06", "gpt-4o-mini-2024-07-18"}
# USD per token: input, cached input, output.
PRICES = {"gpt-4o-2024-08-06": (2.5e-6, 1.25e-6, 10e-6), "gpt-4o-mini-2024-07-18": (0.15e-6, 0.075e-6, 0.6e-6)}
# Batch API: half the input and output rates. Cached input tokens are billed here at the batch input
# rate, an upper bound that does not depend on whether a batch discount stacks with prompt caching.
BATCH_PRICES = {model: (pin / 2, pin / 2, pout / 2) for model, (pin, _, pout) in PRICES.items()}
BATCH_STATE = Path.home() / ".cache/warm-bench/longmemeval_official_batches"
BATCH_DONE = {"completed", "failed", "expired", "cancelled"}
CHAT_URL = "/v1/chat/completions"
GEN_LENGTH = 800
MAX_RETRIEVAL_TOKENS = 128000 - GEN_LENGTH - 1000
TEMPLATE = ("I will give you several history chats between you and a user. Please answer the question based on the "
            "relevant chat history. Answer the question step by step: first extract all the relevant information, "
            "and then reason over the information to get the answer.\n\n\nHistory Chats:\n\n{}\n\nCurrent Date: {}\n"
            "Question: {}\nAnswer (step by step):")
LOCK = threading.Lock()
ENC = tiktoken.get_encoding("o200k_base")

BATCH_STATE = STATE / "batches"
JUDGES = {"gpt-4o": "gpt-4o-2024-08-06", "gpt-4o-mini": "gpt-4o-mini-2024-07-18"}  # official model_zoo
# Measured on the committed dev k=10 gpt-4o run (benchmarks/results/longmemeval_official_dev_gpt4o_k10.json
# and the call cache): used only by the dry-run spend estimate.
DEV_K10_READER_TOKENS = 572907
DEV_N = 100
READER_COMPLETION_TOKENS = 239  # mean over cached gpt-4o reader calls (pooled over k); cap is GEN_LENGTH
JUDGE_PROMPT_TOKENS = 390  # mean over 1502 cached gpt-4o judge calls
JUDGE_COMPLETION_TOKENS = 2


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def split_ids() -> tuple[set[str], set[str]]:
    data = SPLIT.read_bytes()
    if sha_bytes(data) != SPLIT_SHA256:
        raise ValueError("split file differs from the frozen split")
    split = json.loads(data)
    dev, heldout = set(split["dev"]), set(split["heldout"])
    if len(dev) != 100 or len(heldout) != 400 or dev & heldout:
        raise ValueError("invalid frozen dev100/heldout400 split")
    return dev, heldout


def load_all(source: Path = SOURCE) -> list[dict]:
    """All 500 entries of the pinned release, in source order."""
    data = source.read_bytes()
    if sha_bytes(data) != SOURCE_SHA256:
        raise ValueError("LongMemEval-S source differs from the pinned release")
    dev, heldout = split_ids()
    entries = json.loads(data)
    ids = [e["question_id"] for e in entries]
    if len(ids) != 500 or set(ids) != dev | heldout:
        raise ValueError("source question ids differ from the frozen split")
    return entries


def load_rankings(path: Path, entries: list[dict]) -> dict[str, list[str]]:
    """Validated {question_id: [turn id, ...]}; ids must exist in that question's haystack."""
    ranked = json.loads(path.read_text())
    wanted = {e["question_id"] for e in entries}
    if set(ranked) != wanted:
        raise ValueError(f"rankings cover {len(set(ranked) & wanted)} of {len(wanted)} questions "
                         f"({len(set(ranked) - wanted)} unknown ids)")
    for e in entries:
        sessions = dict(zip(e["haystack_session_ids"], e["haystack_sessions"]))
        ids = ranked[e["question_id"]]
        if len(ids) != len(set(ids)):
            raise ValueError(f"duplicate turn ids in the ranking for {e['question_id']}")
        for corpus_id in ids:
            converted = corpus_id.replace("noans_", "answer_")
            sid, _, num = converted.rpartition("_")
            if sid not in sessions or not num.isdigit() or not 1 <= int(num) <= len(sessions[sid]):
                raise ValueError(f"ranking for {e['question_id']} names a turn outside its haystack: {corpus_id}")
    return ranked


def official_prompt(entry: dict, ranked: list[str], topk: int) -> str:
    """run_generation.prepare_prompt with flat-turn, json, cot=true, merge none, useronly false."""
    by_id, dates = {}, {}
    for date, sid, session in zip(entry["haystack_dates"], entry["haystack_session_ids"], entry["haystack_sessions"]):
        by_id[sid], dates[sid] = session, date
    chunks = []
    for corpus_id in ranked:
        converted = corpus_id.replace("noans_", "answer_")
        sid, turn = "_".join(converted.split("_")[:-1]), int(converted.split("_")[-1]) - 1
        if sid not in by_id or turn >= len(by_id[sid]):
            continue
        round_ = [by_id[sid][turn]]
        if turn + 1 < len(by_id[sid]):
            round_.append(by_id[sid][turn + 1])
        chunks.append((dates[sid], copy.deepcopy(round_)))
    chunks = chunks[:topk]
    for _, session in chunks:
        for turn in session:
            turn.pop("has_answer", None)
    chunks.sort(key=lambda chunk: chunk[0])
    history = ""
    for i, (date, session) in enumerate(chunks):
        history += "\n### Session {}:\nSession Date: {}\nSession Content:\n{}\n".format(
            i + 1, date, "\n" + json.dumps(session))
    tokens = ENC.encode(history, allowed_special={"<|endoftext|>"})
    if len(tokens) > MAX_RETRIEVAL_TOKENS:
        history = ENC.decode(tokens[:MAX_RETRIEVAL_TOKENS])
    if "has_answer" in history:
        raise ValueError("evidence label reached the reader input")
    return TEMPLATE.format(history, entry["question_date"], entry["question"])


def load_judge_prompt():
    spec = importlib.util.spec_from_file_location("lme_evaluate_qa", EVALUATE_QA)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.get_anscheck_prompt


def _ledger() -> dict:
    if LEDGER.exists():
        return json.loads(LEDGER.read_text())
    return {"schema_version": 1, "cap_usd": SPEND_CAP_USD, "prices_usd_per_token": PRICES,
            "total_usd": 0.0, "calls": 0, "by_model_usd": {}, "by_run_usd": {}}


def request_body(model: str, prompt: str, max_tokens: int) -> dict:
    """The chat completion request both the synchronous and the batch path send."""
    return {"model": model, "messages": [{"role": "user", "content": prompt}], "n": 1,
            "temperature": 0, "max_tokens": max_tokens}


def cache_key(request: dict, repeat: int = 0) -> str:
    # A repeat index asks for a fresh sample of the same request (temperature 0 is not deterministic).
    return sha_bytes(json.dumps({**request, **({"repeat": repeat} if repeat else {})}, sort_keys=True).encode())


def _write_ledger(ledger: dict) -> None:
    STATE.mkdir(parents=True, exist_ok=True)
    LEDGER.write_text(json.dumps(ledger, indent=1, sort_keys=True) + "\n")


def _reserve(usd: float) -> None:
    with LOCK:
        ledger = _ledger()
        if ledger["total_usd"] + usd > SPEND_CAP_USD:
            raise RuntimeError("spend cap reached")
        ledger["total_usd"] += usd
        _write_ledger(ledger)


def _settle(run: str, model: str, reserve: float, cost: float, calls: int, batch: dict | None = None) -> None:
    with LOCK:
        ledger = _ledger()
        ledger["total_usd"] += cost - reserve
        ledger["calls"] += calls
        ledger["by_model_usd"][model] = ledger["by_model_usd"].get(model, 0.0) + cost
        ledger["by_run_usd"][run] = ledger["by_run_usd"].get(run, 0.0) + cost
        if batch is not None:
            ledger.setdefault("batches", []).append(batch)
        _write_ledger(ledger)


def _worst_case(prices: dict, model: str, prompt: str, max_tokens: int) -> float:
    price_in, _, price_out = prices[model]
    return (len(ENC.encode(prompt)) + 16) * price_in + max_tokens * price_out


def call(client, run: str, model: str, prompt: str, max_tokens: int, repeat: int = 0) -> dict:
    request = request_body(model, prompt, max_tokens)
    path = CACHE / f"{cache_key(request, repeat)}.json"
    if path.exists():
        return json.loads(path.read_text())
    price_in, price_cached, price_out = PRICES[model]
    reserve = _worst_case(PRICES, model, prompt, max_tokens)
    _reserve(reserve)
    cost = reserve
    try:
        for attempt in range(4):
            try:
                completion = client.chat.completions.create(**request)
                break
            except Exception:
                if attempt == 3:
                    raise
                threading.Event().wait(2 ** attempt * 5)
        usage = completion.usage
        cached = getattr(usage.prompt_tokens_details, "cached_tokens", 0) or 0
        cost = (usage.prompt_tokens - cached) * price_in + cached * price_cached + usage.completion_tokens * price_out
        out = {"text": completion.choices[0].message.content.strip(), "model": completion.model,
               "prompt_tokens": usage.prompt_tokens, "cached_tokens": cached,
               "completion_tokens": usage.completion_tokens, "cost_usd": cost}
    finally:
        _settle(run, model, reserve, cost, 1)
    CACHE.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(out))
    return out


def batch_lines(model: str, prompts: dict[str, str], max_tokens: int, repeat: int) -> tuple[list[str], dict[str, str]]:
    """JSONL request lines for the uncached requests and their custom-id to cache-key map."""
    pending = {}
    for prompt in prompts.values():
        key = cache_key(request_body(model, prompt, max_tokens), repeat)
        if not (CACHE / f"{key}.json").exists():
            pending[key] = prompt
    ids = {f"r{i:04d}": key for i, key in enumerate(sorted(pending))}
    lines = [json.dumps({"custom_id": cid, "method": "POST", "url": CHAT_URL,
                         "body": request_body(model, pending[key], max_tokens)}) for cid, key in ids.items()]
    return lines, ids


def _write_state(path: Path, state: dict) -> None:
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(state, indent=1) + "\n")
    tmp.replace(path)


def _line_result(model: str, record: dict) -> tuple[dict | None, object]:
    """(cache entry, None) for a successful output line, else (None, its error)."""
    response = record.get("response") or {}
    if record.get("error") or response.get("status_code") != 200:
        return None, record.get("error") or (response.get("body") or {}).get("error") or response.get("status_code")
    body = response["body"]
    usage = body["usage"]
    cached = (usage.get("prompt_tokens_details") or {}).get("cached_tokens", 0) or 0
    price_in, price_cached, price_out = BATCH_PRICES[model]
    cost = (usage["prompt_tokens"] - cached) * price_in + cached * price_cached + usage["completion_tokens"] * price_out
    return {"text": body["choices"][0]["message"]["content"].strip(), "model": body["model"],
            "prompt_tokens": usage["prompt_tokens"], "cached_tokens": cached,
            "completion_tokens": usage["completion_tokens"], "cost_usd": cost, "batch": True}, None


def _submit(client, state_path: Path, state: dict, data: bytes) -> dict:
    """Upload the input file and create the batch, persisting each id as soon as it exists."""
    if not state.get("input_file_id"):
        upload = client.files.create(file=(state_path.stem + ".jsonl", data, "application/jsonl"), purpose="batch")
        state["input_file_id"] = upload.id
        _write_state(state_path, state)
    batch = client.batches.create(input_file_id=state["input_file_id"], endpoint=CHAT_URL, completion_window="24h",
                                  metadata={"run": state["run"], "stage": state["stage"]})
    state["batch_id"] = batch.id
    _write_state(state_path, state)
    return state


def _poll(client, batch_id: str, max_wait: float):
    """Retrieve the batch with bounded backoff until it reaches a final status or the wait runs out."""
    deadline, delay = time.monotonic() + max_wait, 15.0
    while True:
        batch = client.batches.retrieve(batch_id)
        counts = batch.request_counts
        print(json.dumps({"batch_id": batch_id, "status": batch.status,
                          "completed": counts.completed if counts else None,
                          "failed": counts.failed if counts else None,
                          "total": counts.total if counts else None}), flush=True)
        if batch.status in BATCH_DONE:
            return batch
        if time.monotonic() + delay > deadline:
            raise SystemExit(f"batch {batch_id} is still {batch.status} after {max_wait:.0f}s; "
                             "rerun the same command to resume it")
        time.sleep(delay)
        delay = min(delay * 2, 300.0)


def batch_stage(client, run: str, stage: str, model: str, prompts: dict[str, str], max_tokens: int,
                repeat: int, max_wait: float, sync_retry: bool) -> dict[str, dict]:
    """Answer every prompt through one batch and the call cache; returns cache entries keyed like `prompts`."""
    lines, ids = batch_lines(model, prompts, max_tokens, repeat)
    if lines:
        data = ("\n".join(lines) + "\n").encode()
        BATCH_STATE.mkdir(parents=True, exist_ok=True)
        state_path = BATCH_STATE / f"{stage}_{sha_bytes(data)[:20]}.json"
        state = json.loads(state_path.read_text()) if state_path.exists() else None
        if state is None:
            state_path.with_suffix(".jsonl").write_bytes(data)
            reserve = sum(_worst_case(BATCH_PRICES, model, json.loads(line)["body"]["messages"][0]["content"],
                                      max_tokens) for line in lines)
            _reserve(reserve)
            state = {"run": run, "stage": stage, "model": model, "repeat": repeat, "custom_ids": ids,
                     "reserve_usd": reserve, "input_sha256": sha_bytes(data), "settled": False}
            _write_state(state_path, state)
        if not state.get("batch_id"):
            try:
                state = _submit(client, state_path, state, data)
            except Exception:
                _settle(run, model, state["reserve_usd"], 0.0, 0)
                state_path.unlink()
                raise
        print(json.dumps({"stage": stage, "batch_id": state["batch_id"], "requests": len(ids),
                          "state": str(state_path)}), flush=True)
        batch = _poll(client, state["batch_id"], max_wait)
        results, failures = {}, {}
        for file_id in (batch.output_file_id, batch.error_file_id):
            if not file_id:
                continue
            for raw in client.files.content(file_id).text.splitlines():
                if raw.strip():
                    record = json.loads(raw)
                    out, error = _line_result(model, record)
                    if out is None:
                        failures[record["custom_id"]] = error
                    else:
                        results[record["custom_id"]] = out
        for cid in ids:
            if cid not in results and cid not in failures:
                failures[cid] = f"no output line (batch {batch.status})"
        if batch.status == "failed" and batch.errors:
            print(json.dumps({"batch_errors": [e.model_dump() for e in batch.errors.data or []]}), flush=True)
        if not state["settled"]:
            cost = sum(out["cost_usd"] for out in results.values())
            _settle(run, model, state["reserve_usd"], cost, len(results),
                    {"batch": True, "batch_id": state["batch_id"], "run": run, "stage": stage, "model": model,
                     "status": batch.status, "requests": len(ids), "succeeded": len(results),
                     "failed": len(failures), "usd": cost, "rates_usd_per_token": BATCH_PRICES[model]})
            state["settled"] = True
            _write_state(state_path, state)
        CACHE.mkdir(parents=True, exist_ok=True)
        for cid, out in results.items():
            (CACHE / f"{ids[cid]}.json").write_text(json.dumps(out))
        if failures:
            print(json.dumps({"stage": stage, "batch_id": state["batch_id"], "failed": len(failures),
                              "errors": {cid: str(err) for cid, err in sorted(failures.items())}}), flush=True)
            if not sync_retry:
                raise SystemExit(f"{len(failures)} {stage} requests failed in batch {state['batch_id']}; rerun to "
                                 "resubmit them as a new batch, or add --sync-retry to send them synchronously")
            by_key = {cache_key(request_body(model, p, max_tokens), repeat): p for p in prompts.values()}
            for cid in sorted(failures):
                call(client, run, model, by_key[ids[cid]], max_tokens, repeat)
    return {qid: json.loads((CACHE / f"{cache_key(request_body(model, p, max_tokens), repeat)}.json").read_text())
            for qid, p in prompts.items()}


def summarize(rows: list[dict], judge: str) -> dict:
    """Accuracy per split (all500, dev100, heldout400) for one judge; failures stay in the denominator."""
    out = {}
    for split in ("all500", "dev100", "heldout400"):
        sel = [r for r in rows if split == "all500" or r["split"] == split[:-3]]
        by_type = {}
        for r in sel:
            g = by_type.setdefault(r["qtype"], {"n": 0, "correct": 0})
            g["n"] += 1
            g["correct"] += r["judges"][judge]["correct"]
        abst = [r for r in sel if "_abs" in r["qid"]]
        n, c = len(sel), sum(r["judges"][judge]["correct"] for r in sel)
        out[split] = {"n": n, "correct": c, "accuracy": c / n if n else None, "by_type": dict(sorted(by_type.items())),
                      "abstention": {"n": len(abst), "correct": sum(r["judges"][judge]["correct"] for r in abst)}}
    return out


def round_chunks(entry: dict):
    """(date, round) for every user turn: the flat-turn units the official retriever indexes."""
    for date, session in zip(entry["haystack_dates"], entry["haystack_sessions"]):
        for t, turn in enumerate(session):
            if turn["role"] != "user":
                continue
            round_ = [{k: v for k, v in x.items() if k != "has_answer"} for x in session[t:t + 2]]
            yield date, round_


def chunk_text(i: int, date: str, round_: list) -> str:
    return "\n### Session {}:\nSession Date: {}\nSession Content:\n{}\n".format(i, date, "\n" + json.dumps(round_))


def proxy_tokens(entries: list[dict], topk: int) -> dict:
    """Dataset-only estimate of reader input tokens when no ranking exists yet.

    Per question: tokens of the fixed template, question and date, plus topk chunks. Point estimate uses
    the mean chunk length of that haystack; the upper bound uses its topk longest chunks. Both ignore
    that a real ranker prefers relevant rounds and that the history is clipped at MAX_RETRIEVAL_TOKENS.
    """
    base = len(ENC.encode(TEMPLATE.format("", "", "")))
    per = {}
    for e in entries:
        lengths = sorted(len(t) for t in ENC.encode_ordinary_batch(
            [chunk_text(1, d, r) for d, r in round_chunks(e)], num_threads=8))
        fixed = base + len(ENC.encode(e["question"])) + len(ENC.encode(e["question_date"]))
        mean = statistics.fmean(lengths)
        per[e["question_id"]] = {"mean": fixed + topk * mean,
                                 "upper": fixed + min(MAX_RETRIEVAL_TOKENS, sum(lengths[-topk:])),
                                 "n_chunks": len(lengths)}
    return per


def spend_table(reader_tokens: float, reader_upper: float, n: int) -> dict:
    """Spend arithmetic, USD, from PRICES (input, cached input, output per token); cached input is not assumed."""
    def reader(model, tokens, completion, prices):
        p_in, _, p_out = prices[model]
        return round(tokens * p_in + n * completion * p_out, 2)

    def judge(model, prices):
        p_in, _, p_out = prices[model]
        return round(n * (JUDGE_PROMPT_TOKENS * p_in + JUDGE_COMPLETION_TOKENS * p_out), 2)

    out = {"inputs": {"questions": n, "reader_input_tokens_estimate": round(reader_tokens),
                      "reader_input_tokens_upper": round(reader_upper), "reader_completion_tokens_each": READER_COMPLETION_TOKENS,
                      "reader_completion_tokens_cap": GEN_LENGTH, "judge_prompt_tokens_each": JUDGE_PROMPT_TOKENS,
                      "judge_completion_tokens_each": JUDGE_COMPLETION_TOKENS,
                      "usd_per_token": {m: {"in": v[0], "out": v[2]} for m, v in PRICES.items()},
                      "batch_usd_per_token": "half of the above (BATCH_PRICES)"}}
    for mode, prices in (("sync", PRICES), ("batch", BATCH_PRICES)):
        rd = "gpt-4o-2024-08-06"
        out[mode] = {
            "reader_gpt4o_expected": reader(rd, reader_tokens, READER_COMPLETION_TOKENS, prices),
            "reader_gpt4o_upper": reader(rd, reader_upper, GEN_LENGTH, prices),
            "judge_gpt4o": judge("gpt-4o-2024-08-06", prices),
            "judge_gpt4o_mini": judge("gpt-4o-mini-2024-07-18", prices)}
        out[mode]["total_expected"] = round(out[mode]["reader_gpt4o_expected"] + out[mode]["judge_gpt4o"]
                                            + out[mode]["judge_gpt4o_mini"], 2)
        out[mode]["total_upper"] = round(out[mode]["reader_gpt4o_upper"] + out[mode]["judge_gpt4o"]
                                         + out[mode]["judge_gpt4o_mini"], 2)
    return out


def dry_run(entries: list[dict], args, ranked: dict | None) -> dict:
    dev, heldout = split_ids()
    types = {}
    for e in entries:
        types[e["question_type"]] = types.get(e["question_type"], 0) + 1
    report = {"questions": len(entries), "dev": sum(e["question_id"] in dev for e in entries),
              "heldout": sum(e["question_id"] in heldout for e in entries), "by_type": dict(sorted(types.items())),
              "abstention_questions": sum("_abs" in e["question_id"] for e in entries), "topk": args.topk,
              "reader": args.reader, "api_calls_made": 0}
    if ranked is not None:
        tokens = sum(len(ENC.encode(official_prompt(e, ranked[e["question_id"]], args.topk))) for e in entries)
        report["reader_input_tokens"] = {"kind": "exact, from --rankings", "total": tokens}
        reader_tokens = reader_upper = tokens
    else:
        per = proxy_tokens(entries, args.topk)
        proxy = sum(v["mean"] for v in per.values())
        upper = sum(v["upper"] for v in per.values())
        dev_proxy = sum(per[q]["mean"] for q in dev if q in per)
        ratio = DEV_K10_READER_TOKENS / dev_proxy if args.topk == 10 else None
        report["reader_input_tokens"] = {
            "kind": "estimate from the dataset file (no ranking supplied)", "proxy_mean_chunk": round(proxy),
            "proxy_upper_topk_longest": round(upper), "dev100_proxy": round(dev_proxy),
            "dev100_measured_k10": DEV_K10_READER_TOKENS if args.topk == 10 else None,
            "calibration_ratio_measured_over_proxy_on_dev": round(ratio, 3) if ratio else None,
            "calibrated_estimate": round(proxy * ratio) if ratio else None,
            "measured_dev_mean_per_question": DEV_K10_READER_TOKENS / DEV_N,
            "note": "proxy ignores ranking quality and the 126200-token clip; calibrated_estimate assumes the 400 "
                    "held-out questions behave like the 100 dev ones; measured numbers exist only for dev"}
        reader_tokens = proxy * ratio if ratio else proxy
        reader_upper = upper
    report["spend_usd"] = spend_table(reader_tokens, reader_upper, len(entries))
    report["spend_cap_usd"] = SPEND_CAP_USD
    return report


def main() -> None:
    cfg = json.loads(FROZEN_CONFIG.read_text())
    frozen_k, frozen_reader = cfg["retrieval"]["k"], cfg["reader"]["model"]
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--reader", choices=sorted(READERS), default=frozen_reader)
    parser.add_argument("--topk", type=int, default=frozen_k)
    parser.add_argument("--rankings", type=Path, help="JSON {question_id: [turn id '<session_id>_<n>', ...]}, all 500")
    parser.add_argument("--judges", default="gpt-4o,gpt-4o-mini", help="comma list from: " + ", ".join(JUDGES))
    parser.add_argument("--tag")
    parser.add_argument("--out-dir", type=Path, default=Path.home() / ".cache/warm-bench/longmemeval_500_out")
    parser.add_argument("--dry-run", action="store_true", help="count questions, estimate tokens and spend; no API")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--repeat", type=int, default=0)
    parser.add_argument("--batch", action="store_true", help="reader and judge stages via the Batch API")
    parser.add_argument("--max-wait", type=float, default=24 * 3600.0)
    parser.add_argument("--sync-retry", action="store_true")
    parser.add_argument("--qids", help="comma-separated ids for a smoke test (the report is then marked partial)")
    parser.add_argument("--allow-nonfrozen", action="store_true", help="permit k or reader different from frozen_config.json")
    args = parser.parse_args()
    if not args.allow_nonfrozen and (args.topk != frozen_k or args.reader != frozen_reader):
        parser.error("k and reader are frozen in frozen_config.json; --allow-nonfrozen makes the run exploratory")
    judges = args.judges.split(",")
    if not set(judges) <= set(JUDGES):
        parser.error("unknown judge")
    if cfg["spend"]["cap_usd"] != SPEND_CAP_USD:
        parser.error("SPEND_CAP_USD differs from frozen_config.json")
    entries = load_all()
    ranked = load_rankings(args.rankings, entries) if args.rankings else None
    if args.qids:
        wanted = set(args.qids.split(","))
        entries = [e for e in entries if e["question_id"] in wanted]
        if len(entries) != len(wanted):
            parser.error("--qids names an unknown question")
        ranked = {q: ranked[q] for q in wanted} if ranked else None
    args.out_dir.mkdir(parents=True, exist_ok=True)
    if args.dry_run:
        report = dry_run(entries, args, ranked)
        (args.out_dir / "dry_run.json").write_text(json.dumps(report, indent=1) + "\n")
        print(json.dumps(report, indent=1))
        return
    if ranked is None or not args.tag:
        parser.error("--rankings and --tag are required for a run (use --dry-run to estimate)")
    if sha_bytes(EVALUATE_QA.read_bytes()) != EVALUATE_QA_SHA256:
        raise ValueError("evaluate_qa.py differs from the pinned official file")
    dev, _ = split_ids()
    prompts = {e["question_id"]: official_prompt(e, ranked[e["question_id"]], args.topk) for e in entries}
    tokens = sum(len(ENC.encode(p)) for p in prompts.values())
    rates = BATCH_PRICES if args.batch else PRICES
    ceiling = tokens * rates[args.reader][0] + len(entries) * GEN_LENGTH * rates[args.reader][2]
    print(json.dumps({"reader_input_tokens": tokens, "max_reader_cost_usd": round(ceiling, 2)}), flush=True)
    from openai import OpenAI

    client = OpenAI()
    judge_prompt = load_judge_prompt()
    run = f"{args.tag}:{args.reader}"

    def judge_input(entry, hyp):
        return judge_prompt(entry["question_type"], entry["question"], entry["answer"], hyp["text"],
                            abstention="_abs" in entry["question_id"])

    if args.batch:
        hyps = batch_stage(client, run, "reader", args.reader, prompts, GEN_LENGTH, args.repeat, args.max_wait,
                           args.sync_retry)
        verdicts = {}
        for j in judges:
            jp = {e["question_id"]: judge_input(e, hyps[e["question_id"]]) for e in entries}
            verdicts[j] = batch_stage(client, run, f"judge_{j}", JUDGES[j], jp, 10, args.repeat, args.max_wait,
                                      args.sync_retry)
    else:
        def read(entry):
            return call(client, run, args.reader, prompts[entry["question_id"]], GEN_LENGTH, args.repeat)

        with ThreadPoolExecutor(args.workers) as pool:
            hyps = dict(zip(prompts, pool.map(read, entries)))
            verdicts = {}
            for j in judges:
                def judge(entry, j=j):
                    return call(client, run, JUDGES[j], judge_input(entry, hyps[entry["question_id"]]), 10, args.repeat)

                verdicts[j] = dict(zip(prompts, pool.map(judge, entries)))
    rows = sorted(({"qid": e["question_id"], "qtype": e["question_type"],
                    "split": "dev" if e["question_id"] in dev else "heldout",
                    "hypothesis": hyps[e["question_id"]]["text"], "reader_model": hyps[e["question_id"]]["model"],
                    "reader_prompt_tokens": hyps[e["question_id"]]["prompt_tokens"],
                    "judges": {j: {"model": verdicts[j][e["question_id"]]["model"],
                                   "reply": verdicts[j][e["question_id"]]["text"],
                                   "correct": "yes" in verdicts[j][e["question_id"]]["text"].lower()}
                               for j in judges}} for e in entries), key=lambda r: r["qid"])
    with open(args.out_dir / f"{args.tag}.hypotheses.jsonl", "w") as f:
        for r in rows:
            f.write(json.dumps({"question_id": r["qid"], "hypothesis": r["hypothesis"]}) + "\n")
    report = {"schema_version": 1, "tag": args.tag, "partial": bool(args.qids), "n_questions": len(rows),
              "frozen_config_sha256": sha_bytes(FROZEN_CONFIG.read_bytes()),
              "pinned_warm_revision": cfg["pinned_warm_revision"], "reader": args.reader,
              "reader_snapshots_observed": sorted({r["reader_model"] for r in rows}), "topk": args.topk,
              "rankings_sha256": sha_bytes(args.rankings.read_bytes()), "api": "batch" if args.batch else "sync",
              "repeat": args.repeat, "official_commit": "9e0b455f4ef0", "evaluate_qa_sha256": EVALUATE_QA_SHA256,
              "source_sha256": SOURCE_SHA256, "split_sha256": SPLIT_SHA256,
              "judges": {j: {"model": JUDGES[j], "summary": summarize(rows, j)} for j in judges},
              "rows": rows,
              "note": "In-script judges use the official get_anscheck_prompt; the headline judge pass is the official "
                      "evaluate_qa.py run on the .hypotheses.jsonl file. Report the two judge models separately."}
    (args.out_dir / f"{args.tag}.json").write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps({j: report["judges"][j]["summary"]["all500"]["accuracy"] for j in judges}), flush=True)


if __name__ == "__main__":
    main()
