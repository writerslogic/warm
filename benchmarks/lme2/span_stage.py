#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""SPAN_V controller (pass 1): gemini-3.6-flash, workflow_doc4, at most 14 rounds, MEDIUM thinking, submit_spans with <=20 states.
No labels are read (the reference ran with NOLABELS=1, so cov is always null).
usage: span_stage.py --ids held_ids.json --out held_span_v.jsonl [--workers 8] [--cap USD]"""
import argparse, json, threading
import fileio  # noqa: E402
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import common, ctl_g, tools3
from prompts import CONTROLLER_MODEL, CONTROLLER_MAX_STEPS, CONTROLLER_EXTRA, _schema, workflow
ap = argparse.ArgumentParser()
ap.add_argument("--model", default=CONTROLLER_MODEL); ap.add_argument("--ids", required=True); ap.add_argument("--n", type=int, default=999); ap.add_argument("--out", required=True)
ap.add_argument("--workers", type=int, default=8); ap.add_argument("--max-steps", type=int, default=CONTROLLER_MAX_STEPS); ap.add_argument("--extra", default=json.dumps(CONTROLLER_EXTRA)); ap.add_argument("--cap", type=float, default=None)
a = ap.parse_args()
Q = common.questions(); hay = common.haystack()
ids = fileio.read_json(a.ids)[:a.n]
T = common.trajectories(); notes = common.notes()
TL = {d: tools3.Tools3(next(hay[q] for q in Q if Q[q]["domain"] == d), T, notes) for d in {Q[i]["domain"] for i in ids}}
DECLS = [{"name": t["function"]["name"], "description": t["function"]["description"], "parameters": _schema(t["function"]["parameters"])} for t in tools3.SPAN_SPECS]
lock = threading.Lock(); extra = json.loads(a.extra); out = Path(a.out); spent = [0.0]
done = common.done_ids(out)
def one(i):
    if a.cap is not None and spent[0] >= a.cap: return
    q = Q[i]; tl = TL[q["domain"]]
    b, st = ctl_g.run(a.model, tl, q["question"], max_steps=a.max_steps, extra=extra, system=workflow(a.max_steps), tool_decls=DECLS, dispatch=tl.dispatch, final_name="submit_spans", final_check=lambda x: tl.check_spans(x.get("spans"))[2])
    spans, tot, err = tl.check_spans((b or {}).get("spans")) if b else (None, 0, "no brief")
    rec = {"id": i, "domain": q["domain"], "type": q["question_type"], "is_abs": q["question_type"].endswith("-abs"), "analysis": (b or {}).get("analysis"), "support": (b or {}).get("support"), "spans": spans, "span_err": err, "n_states": tot,
           "stats": st, "cost_usd": st["cost_usd"], "cov": None}
    with lock: spent[0] += st["cost_usd"]; fileio.append_line(out, json.dumps(rec) + "\n")
    print(i, rec["cov"], tot, err, st["tool_calls"], round(st["seconds"]), round(st["cost_usd"], 3), st["err"], flush=True)
with ThreadPoolExecutor(a.workers) as ex: list(ex.map(one, [i for i in ids if i not in done]))
