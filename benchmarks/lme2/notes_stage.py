#!/usr/bin/env python3
"""Ingest-time notes: one procedure note and one hint note per trajectory, written by the local
reader model (Qwen3.5-9B) with the upstream NOTE_GENERATION prompt (qwen_v6_retrieval_safe).
Text-only (URL, thought, action per state); screenshots are not sent. Resumable."""
import argparse, asyncio, json, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "upstream"))
import memory_modules.memory  # noqa: F401 (import order avoids a circular import in support)
from memory_modules.support import (NOTE_GENERATION_SYSTEM_PROMPT, NOTE_GENERATION_REPAIR_SYSTEM_PROMPT,
                                    _parse_generated_notes_json)
from openai import AsyncOpenAI

ap = argparse.ArgumentParser()
ap.add_argument("--traj", default="/Volumes/A/.hms-lb/lme2/data/trajectories.jsonl")
ap.add_argument("--ids", help="json list of trajectory ids to restrict to")
ap.add_argument("--out", required=True)
ap.add_argument("--base-url", default="http://localhost:8023/v1")
ap.add_argument("--concurrency", type=int, default=24)
ap.add_argument("--max-tokens", type=int, default=12000)
a = ap.parse_args()
out = Path(a.out)
notes = json.loads(out.read_text()) if out.exists() else {}
want = set(json.load(open(a.ids))) if a.ids else None
trajs = [t for t in map(json.loads, open(a.traj)) if (want is None or t["id"] in want) and t["id"] not in notes]
print(len(trajs), "to do", flush=True)

def messages(t):
    st = t["states"]
    head = "\n".join([
        "Extract two reusable notes from this UI task run.", f"Goal: {t['goal']}", f"Outcome: {t.get('outcome') or '<unknown>'}",
        f"Start URL: {t['start_url']}",
        "Actions in this dataset are attached to destination states, so the next-action line below describes the action taken from the current state to the next state.",
        "These notes should stay useful later under retrieval for unknown questions.", "Only preserve facts grounded in the observed run.",
        "Favor durable answer-bearing facts over generic page furniture.",
        'Return only the JSON object: {"procedure_note":{"title":"...","description":"...","content":"- ..."},"hint_note":{"title":"...","description":"...","content":"- ..."}}',
        "Do not think aloud in the final answer. First character must be {."])
    parts = [head]
    for i, s in enumerate(st):
        nxt = st[i + 1].get("action") if i + 1 < len(st) else None
        parts.append(f"State {i}\nURL: {s.get('url')}\nThought: {(s.get('thought') or '<none>').strip()}\nNext action from this state: {nxt if nxt not in (None,'None','') else ('<end of trajectory>' if i + 1 >= len(st) else '<none>')}")
    return [{"role": "system", "content": NOTE_GENERATION_SYSTEM_PROMPT}, {"role": "user", "content": "\n\n".join(parts)}]

def fallback(t, kind):
    acts = [s.get("action") for s in t["states"][1:] if s.get("action") not in (None, "None", "")]
    if kind == "procedure":
        return {"title": f"Procedure fallback for {t['goal'][:60]}", "description": "Fallback procedure note.",
                "content": "\n".join(f"- Step {i+1}: {x}" for i, x in enumerate(acts[:8]))}
    return {"title": f"Hint fallback for {t['goal'][:60]}", "description": "Fallback hint note.",
            "content": f"- Outcome: {t.get('outcome')}\n- Start URL: {t['start_url']}\n- Recorded actions: {len(acts)}"}

async def main():
    cl = AsyncOpenAI(base_url=a.base_url, api_key="EMPTY", timeout=3600)
    sem = asyncio.Semaphore(a.concurrency)
    async def call(msgs, think=True, mt=None):
        r = await cl.chat.completions.create(model="Qwen/Qwen3.5-9B", messages=msgs, temperature=0.6, top_p=0.95,
            max_tokens=mt or a.max_tokens, extra_body={"top_k": 20, **({} if think else {"chat_template_kwargs": {"enable_thinking": False}})})
        return (r.choices[0].message.content or "").strip()
    async def one(t):
        async with sem:
            try:
                txt = await call(messages(t))
                p, h, _, _ = _parse_generated_notes_json(txt)
                if (p is None or h is None) and txt:
                    p2, h2, _, _ = _parse_generated_notes_json(await call([{"role": "system", "content": NOTE_GENERATION_REPAIR_SYSTEM_PROMPT}, {"role": "user", "content": "Convert this draft into the required JSON object.\nReturn JSON only.\n\n" + txt}], think=False, mt=3000))
                    if p2 and h2: p, h = p2, h2
                fb = p is None or h is None
            except Exception as e:
                print("error", t["id"], repr(e), flush=True); return
            notes[t["id"]] = {"procedure": p or fallback(t, "procedure"), "hint": h or fallback(t, "hint"), "fallback": fb}
            tmp = out.with_suffix(".tmp"); tmp.write_text(json.dumps(notes)); tmp.replace(out)
            print("done", len(notes), "fb" if fb else "", flush=True)
    await asyncio.gather(*(one(t) for t in trajs))
asyncio.run(main())
