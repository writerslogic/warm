#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Run the fixed reader (Qwen3.5-9B on a vLLM OpenAI-compatible endpoint) over prompt_rows.jsonl files; resumable, writes a reader cache
{question_id: {messages_sha256, output}}. Environment: LME2_UPSTREAM, READER_BASE_URL (default http://localhost:8023/v1), READER_API_KEY (default EMPTY)."""
import argparse, asyncio, hashlib, json, os, sys
import fileio  # noqa: E402
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

common.add_upstream_path()
import evaluation.harness as H  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--rows", nargs="+", required=True)
ap.add_argument("--cache", required=True)
ap.add_argument("--base-url", default=os.environ.get("READER_BASE_URL", "http://localhost:8023/v1"))
ap.add_argument("--concurrency", type=int, default=32)
a = ap.parse_args()

args = SimpleNamespace(
    model="Qwen/Qwen3.5-9B", base_url=a.base_url, api_key_env="READER_API_KEY", api_key_file=None,
    max_completion_tokens=20000, timeout_seconds=43200.0, reasoning_effort=None, temperature=0.6,
    top_p=0.95, presence_penalty=None, top_k=20, repetition_penalty=None, reader_enable_thinking=True,
    reader_max_concurrent_requests=a.concurrency,
)
cache_path = Path(a.cache)
cache = json.loads(cache_path.read_text()) if cache_path.exists() else {}
rows = []
for p in a.rows:
    rows += [json.loads(l) for l in fileio.lines(p) if l.strip()]
todo = [r for r in rows if r["question_id"] not in cache]
print(f"{len(rows)} rows, {len(todo)} to run", flush=True)


async def rolling():
    client = H.create_async_client(args.base_url, args.api_key_env, args.api_key_file)
    sem = asyncio.Semaphore(a.concurrency)
    done = 0

    async def one(r):
        nonlocal done
        async with sem:
            try:
                text, usage = await H.call_reader_model_async(client, args, r["messages"])
            except H.BadRequestError as exc:
                print(f"bad request {r['question_id']}: {exc}", file=sys.stderr, flush=True)
                text, usage = "", {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
            except Exception as exc:  # transient: leave uncached, a rerun retries
                print(f"error {r['question_id']}: {exc!r}", file=sys.stderr, flush=True)
                return
        parsed = H.extract_boxed_answer(text)
        cache[r["question_id"]] = {
            "messages_sha256": hashlib.sha256(json.dumps(r["messages"], sort_keys=True).encode()).hexdigest(),
            "output": {"response_raw": text, "response_parsed_boxed": parsed, "is_unknown": H.is_unknown(parsed), "usage": usage},
        }
        tmp = cache_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(cache))
        tmp.replace(cache_path)
        done += 1
        print(f"done {done}/{len(todo)}", flush=True)

    await asyncio.gather(*(one(r) for r in todo))
    await client.close()

asyncio.run(rolling())
