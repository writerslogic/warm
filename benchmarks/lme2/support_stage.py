#!/usr/bin/env python3
"""Query-time Support Analysis (premise check) written by the local reader model for every
pending (question, evidence) pair left by warm_notes. Resumable; keys are sha256(question)."""
import argparse, asyncio, json, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "upstream")); sys.path.insert(0, str(Path(__file__).resolve().parent))
import memory_modules.memory  # noqa: F401
from warm_notes import SUPPORT_SYSTEM, SUPPORT_USER
from openai import AsyncOpenAI
ap = argparse.ArgumentParser()
ap.add_argument("--pending", required=True); ap.add_argument("--cache", required=True)
ap.add_argument("--base-url", default="http://localhost:8023/v1"); ap.add_argument("--concurrency", type=int, default=16)
ap.add_argument("--max-tokens", type=int, default=8000); ap.add_argument("--keys")
a = ap.parse_args()
cp = Path(a.cache); cache = json.loads(cp.read_text()) if cp.exists() else {}
pend = json.loads(Path(a.pending).read_text())
keys = set(json.load(open(a.keys))) if a.keys else None
todo = [k for k in pend if k not in cache and (keys is None or k in keys)]
print(len(todo), "to do", flush=True)
async def main():
    cl = AsyncOpenAI(base_url=a.base_url, api_key="EMPTY", timeout=7200); sem = asyncio.Semaphore(a.concurrency); done = 0
    async def one(k):
        nonlocal done
        async with sem:
            try:
                r = await cl.chat.completions.create(model="Qwen/Qwen3.5-9B", temperature=0.6, top_p=0.95, max_tokens=a.max_tokens, extra_body={"top_k": 20},
                    messages=[{"role": "system", "content": SUPPORT_SYSTEM}, {"role": "user", "content": SUPPORT_USER.format(question=pend[k]["question"], evidence=pend[k]["evidence"])}])
            except Exception as e:
                print("error", k, repr(e)[:200], flush=True); return
        txt = (r.choices[0].message.content or "").strip()
        cache[k] = txt  # empty text = thinking exhausted; context then omits the analysis
        cp.with_suffix(".tmp").write_text(json.dumps(cache)); cp.with_suffix(".tmp").replace(cp)
        done += 1; print("done", done, len(todo), r.usage.prompt_tokens, r.usage.completion_tokens, flush=True)
    await asyncio.gather(*(one(k) for k in todo))
asyncio.run(main())
