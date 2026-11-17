#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Single-call Gemini checks over the states a controller selected. No labels are read.
  verify    VERIFY: presupposition checks, premise verdict, grounded analysis
  crux      CRUX: false-premise target with verbatim quotes, grounded against the cited state
  optcheck  OPTCHECK: per-option verdicts with grounded quotes (choice questions)
  anscheck  ANSWER-CHECK: the reader pass-1 answer against verbatim state text (needs --answers)
usage: check_stages.py <stage> --spans ctl.jsonl... --out out.jsonl [--ids ids.json] [--model gemini-3.6-flash] [--workers 8] [--cap USD]"""
import argparse, json, threading, time
import fileio  # noqa: E402
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import common, ctl_g
from prompts import (VERIFY_SYSTEM, VERIFY_DECL, CRUX_SYSTEM, CRUX_DECL, OPTCHECK_SYSTEM, OPTCHECK_DECL, ANSCHECK_SYSTEM, ANSCHECK_DECL,
                     stage_body, verify_message, states_message, anscheck_message)

ap = argparse.ArgumentParser(); ap.add_argument("stage", choices=["verify", "crux", "optcheck", "anscheck"])
ap.add_argument("--spans", nargs="+", required=True); ap.add_argument("--out", required=True); ap.add_argument("--model", default="gemini-3.6-flash")
ap.add_argument("--workers", type=int, default=8); ap.add_argument("--cap", type=float, default=None); ap.add_argument("--ids"); ap.add_argument("--answers")
a = ap.parse_args()
if a.stage == "anscheck" and not a.answers: ap.error("anscheck needs --answers")
Q = common.questions(); common.init(); render, ground = common.render, common.ground
ctl = common.load_jsonl_by_id(a.spans)
ids = fileio.read_json(a.ids) if a.ids else sorted(ctl)
ANS = {json.loads(l)["id"]: json.loads(l)["answer"] for l in fileio.lines(a.answers)} if a.answers else {}
out = Path(a.out); done = common.done_ids(out); lock = threading.Lock(); spent = [0.0]


def one(i):
    if a.cap is not None and spent[0] >= a.cap: return
    r = ctl[i]; q = Q[i]["question"]
    if a.stage == "verify": body = stage_body(VERIFY_SYSTEM, verify_message(q, r, render(r)), VERIFY_DECL, "verify")
    elif a.stage == "crux": body = stage_body(CRUX_SYSTEM, states_message(q, render(r)), CRUX_DECL, "cite")
    elif a.stage == "optcheck": body = stage_body(OPTCHECK_SYSTEM, states_message(q, render(r)), OPTCHECK_DECL, "cite")
    else: body = stage_body(ANSCHECK_SYSTEM, anscheck_message(q, ANS.get(i, ''), render(r)), ANSCHECK_DECL, "cite")
    t0 = time.perf_counter()
    res, err, cost, u = ctl_g.call_once(a.model, body)
    if a.stage == "verify":
        # the reference VERIFY stage prices only the last response
        pin, pout = ctl_g.PRICES[a.model]; cost = u.get("promptTokenCount", 0) * pin + (u.get("candidatesTokenCount", 0) + u.get("thoughtsTokenCount", 0)) * pout
        rec = {"id": i, "v": res, "err": err, "seconds": time.perf_counter() - t0, "cost_usd": cost, "ptok": u.get("promptTokenCount", 0)}
        msg = (i, (res or {}).get("premise"), (res or {}).get("support"), round(rec["seconds"]), round(cost, 3), err)
    elif a.stage == "optcheck":
        g = [dict(o, quotes=ground(r, o.get("quotes"))) for o in (res or {}).get("options") or []]
        rec = {"id": i, "c": res, "grounded": g, "n_quotes": sum(len(o.get("quotes") or []) for o in (res or {}).get("options") or []), "err": err, "seconds": time.perf_counter() - t0, "cost_usd": cost}
        msg = (i, rec["n_quotes"], sum(len(o["quotes"]) for o in g), round(rec["seconds"]), round(cost, 3), err)
    else:
        g = ground(r, (res or {}).get("quotes")) if res else []
        rec = {"id": i, "c": res, "grounded": g, "n_quotes": len((res or {}).get("quotes") or []), "err": err, "seconds": time.perf_counter() - t0, "cost_usd": cost}
        msg = (i, (res or {}).get("verdict"), rec["n_quotes"], len(g), round(rec["seconds"]), round(cost, 3), err)
    with lock: spent[0] += cost; fileio.append_line(out, json.dumps(rec) + "\n")
    print(*msg, flush=True)


with ThreadPoolExecutor(a.workers) as ex: list(ex.map(one, [i for i in ids if i in ctl and i not in done]))
