#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Second, targeted SPAN_V controller pass for choice questions whose option check found no option decided by the selected states.
The controller gets the question plus a note listing the undecided options; its spans go first, then first-pass spans, up to 20 states. No labels read.
usage: ctl_pass2.py --ids undecided_ids.json --opt opt.jsonl --spans held_span_v.jsonl --out pass2.jsonl"""
import argparse, json, threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import common, ctl_g, tools3
from rules import merge_pass2
from prompts import CONTROLLER_MODEL, CONTROLLER_MAX_STEPS, CONTROLLER_EXTRA, PASS2_NO_OPTIONS, workflow, pass2_question
ap = argparse.ArgumentParser(); ap.add_argument("--ids", required=True); ap.add_argument("--opt", required=True); ap.add_argument("--spans", nargs="+", required=True); ap.add_argument("--out", required=True); a = ap.parse_args()
Q = common.questions()
hay = common.haystack(); T = common.trajectories(); notes = common.notes()
ids = json.load(open(a.ids)); O = {json.loads(l)["id"]: json.loads(l) for l in open(a.opt)}
P1 = common.load_jsonl_by_id(a.spans)
TL = {d: tools3.Tools3(next(hay[q] for q in Q if Q[q]["domain"] == d), T, notes) for d in {Q[i]["domain"] for i in ids}}
DECLS = [{"name": t["function"]["name"], "description": t["function"]["description"], "parameters": ctl_g._schema(t["function"]["parameters"])} for t in tools3.SPAN_SPECS]
lock = threading.Lock(); out = Path(a.out); done = common.done_ids(out)


def one(i):
    q = Q[i]; tl = TL[q["domain"]]; opts = "; ".join(o["option"][:80] for o in O[i]["grounded"]) or PASS2_NO_OPTIONS
    qq = pass2_question(q["question"], opts)
    b, st = ctl_g.run(CONTROLLER_MODEL, tl, qq, max_steps=CONTROLLER_MAX_STEPS, extra=dict(CONTROLLER_EXTRA), system=workflow(CONTROLLER_MAX_STEPS), tool_decls=DECLS, dispatch=tl.dispatch, final_name="submit_spans", final_check=lambda x: tl.check_spans(x.get("spans"))[2])
    spans, tot, err = tl.check_spans((b or {}).get("spans")) if b else (None, 0, "no brief")
    merged = merge_pass2(spans, P1[i]["spans"])
    rec = {"id": i, "analysis": (b or {}).get("analysis") or P1[i]["analysis"], "support": (b or {}).get("support") or P1[i]["support"], "spans": merged, "stats": st, "cost_usd": st["cost_usd"], "span_err": err}
    with lock: open(out, "a").write(json.dumps(rec) + "\n")
    print(i, tot, err, round(st["seconds"]), round(st["cost_usd"], 3), flush=True)


with ThreadPoolExecutor(8) as ex: list(ex.map(one, [i for i in ids if i not in done]))
