#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Offline check of the deterministic G1 pieces against the frozen reference run (no network, no engine build).
Needs LME2_DATA and LME2_UPSTREAM; --reference-dir holds the frozen run's files (arch3/, arch6/, warm_memory/).
  1. rules: final rule, choice ids, undecided ids, pass-1 answers, mkp6 reread rows and pass-2 application equal the frozen outputs
  2. pass-2 merge: boundary cases at 19, 20 and 21 states, and idempotence on the frozen pass-2 records
  3. renderer: G1SpanMemory(form_backend=reference) items equal the reference arch3_span renderer for the given questions,
     pass 1 (spans_held_s2) and pass 2 (spans_held_g1 + Evidence Check)
  4. engine input: accessibility-tree dicts stay inside the engine bounds, steps increase, and a form table computed from the
     tree alone equals formstate.table on the raw text for every state (the tree dicts lose nothing formstate reads;
     the engine itself parses controls from the accessibilityTree text)
  5. trajectory loader (--loader): records read from trajectories.jsonl equal a reference trajectory pickle
  6. engine (--engine, run where the writerslogic_warm wheel from this branch is installed): form_backend=engine renders
     exactly what form_backend=reference renders, both passes, every selected question
usage: smoke_test.py --reference-dir DIR [--questions all|N] [--loader REF_PKL] [--engine]"""
import argparse, json, os, pickle, sys
import fileio  # noqa: E402
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import common, formstate, formtable, rules  # noqa: E402

ap = argparse.ArgumentParser(); ap.add_argument("--reference-dir", required=True); ap.add_argument("--questions", default="all"); ap.add_argument("--loader"); ap.add_argument("--engine", action="store_true"); a = ap.parse_args()
R = Path(a.reference_dir); A6 = R / "arch6"
fails = []
def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + name + (f"  {detail}" if detail else ""), flush=True)
    if not ok: fails.append(name)
jl = lambda p: [json.loads(l) for l in fileio.lines(p)]

Q = common.questions(); QT = {i: q["question"] for i, q in Q.items()}
C = common.load_jsonl_by_id([R / "arch3/held_span_v.jsonl"])
V = {r["id"]: r["v"] for r in jl(A6 / "held_verify_v1.jsonl")}; X = {r["id"]: r["c"] for r in jl(A6 / "held_crux_v1.jsonl")}
f1 = rules.final_rule(C, V, X, QT)
check("final rule == spans_held_f1.json", f1 == fileio.read_json(A6 / "spans_held_f1.json"), f"absent {sum(o['support'] == 'absent' for o in f1.values())}/{len(f1)}")
held = fileio.read_json(HERE / "held_ids.json")
check("choice ids == held_choice_ids.json", rules.choice_ids(held, Q) == fileio.read_json(A6 / "held_choice_ids.json"))
check("undecided ids == held_undecided_ids.json", rules.undecided_ids(jl(A6 / "held_opt_v1.jsonl")) == fileio.read_json(A6 / "held_undecided_ids.json"))
ans = rules.pass1_answers(fileio.read_json(A6 / "gpu_out/out_held1.json"))
check("pass-1 answers == held_f1_answers.jsonl", ans == jl(A6 / "held_f1_answers.jsonl"))
base = {v["id"]: v for v in f1.values()}
first = rules.reread_first(jl(A6 / "held_anscheck_v1.jsonl"), base, QT)
check("reread Evidence Check rows == held_reread_first.json", first == fileio.read_json(A6 / "held_reread_first.json"), f"fires {len(first)}")
P2 = jl(A6 / "held_pass2_v1.jsonl")
check("pass-2 application == spans_held_s2.json", rules.apply_pass2(base, P2, QT) == fileio.read_json(A6 / "spans_held_s2.json"), f"{len(P2)} pass-2 records")

sp = lambda t, x, y: {"trajectory_id": t, "start_state_index": x, "end_state_index": y}
check("merge: pass-2 spans first, first-pass 19 states fits", rules.merge_pass2([sp("a", 0, 0)], [sp("b", 0, 18)]) == [sp("a", 0, 0), sp("b", 0, 18)])
check("merge: total of exactly 20 kept", rules.merge_pass2([sp("a", 0, 0)], [sp("b", 0, 18), sp("c", 0, 0)]) == [sp("a", 0, 0), sp("b", 0, 18)])
check("merge: 21 states drops the span that overflows, keeps a later one that fits", rules.merge_pass2([sp("a", 0, 1)], [sp("b", 0, 18), sp("c", 0, 0)]) == [sp("a", 0, 1), sp("c", 0, 0)])
check("merge: an exact duplicate is not repeated, an overlapping span is", rules.merge_pass2([sp("a", 0, 3)], [sp("a", 0, 3), sp("a", 2, 4)]) == [sp("a", 0, 3), sp("a", 2, 4)])
check("merge idempotent on frozen pass-2 records", all(rules.merge_pass2(r["spans"], C[r["id"]]["spans"]) == r["spans"] for r in P2))

T = common.init()
common.add_upstream_path()
import memory_modules  # noqa: E402,F401
import g1_memory  # noqa: E402
sys.path.insert(0, str(R / "warm_memory"))
import arch3_span as ref_span  # noqa: E402

ids = held if a.questions == "all" else held[:int(a.questions)]
fired = [i for i in held if rules.qkey(QT[i]) in first]
ids = list(dict.fromkeys(ids + fired[:3]))
screens = str(common.data_dir() / "screenshots")
for label, spans_file, first_file in (("pass 1 (s2)", A6 / "spans_held_s2.json", None), ("pass 2 (g1 + Evidence Check)", A6 / "spans_held_g1.json", A6 / "held_reread_first.json")):
    os.environ["ARCH3_SPANS"] = str(spans_file); os.environ["ARCH5_FORM"] = "table"; os.environ["ARCH5_BUDGET"] = "140000"
    if first_file: os.environ["ARCH6_FIRST"] = str(first_file)
    else: os.environ.pop("ARCH6_FIRST", None)
    ref = ref_span.Arch3SpanMemory({"mode": "span_v", "screens_root": screens})
    port = g1_memory.G1SpanMemory({"mode": "span_v", "spans": str(spans_file), "first": str(first_file) if first_file else None, "form_backend": "reference", "screens_root": screens})
    for t in T.values(): ref.insert(t); port.insert(t)
    same = 0; over = 0; bad = []
    for i in ids:
        x, y = ref.query(QT[i]), port.query(QT[i])
        same += x == y
        if x != y: bad.append(i)
        over += any("[... page text cut at the context budget]" in it["value"] for it in y if it["type"] == "text") or sum(0.42 * len(it["value"]) for it in y if it["type"] == "text") > 140000 * 0.9
    check(f"renderer {label}: identical items", not bad, f"{same}/{len(ids)} questions, {over} near or over the 140000-token budget, mismatches {bad[:5]}")
    if a.engine:
        import tempfile
        eng = g1_memory.G1SpanMemory({"mode": "span_v", "spans": str(spans_file), "first": str(first_file) if first_file else None, "form_backend": "engine", "screens_root": screens, "store_root": tempfile.mkdtemp(prefix="g1-smoke-")})
        for t in T.values(): eng.insert(t)
        ebad = [i for i in ids if eng.query(QT[i]) != port.query(QT[i])]
        if ebad:
            rec = eng._s[rules.qkey(QT[ebad[0]])]; valid = [(T[s["trajectory_id"]], s["start_state_index"], s["end_state_index"]) for s in rec["spans"]]
            for k, (x, y) in enumerate(zip(eng.form_blocks(valid), port.form_blocks(valid))):
                if x != y: print(f"first differing block, question {ebad[0]} state {k}:\nengine:\n{x}\nreference:\n{y}"); break
        check(f"engine {label}: identical items to the reference backend", not ebad, f"{len(ids) - len(ebad)}/{len(ids)} questions")

def tree_table(tree):
    """formstate.table driven by the tree only: preorder nodes with a handle, depth = nesting level."""
    nodes = []
    def walk(n, d):
        if n.get("handle"):
            rest = n.get("props", "")
            props = {k: v.strip("'") for k, v in formstate.PROP.findall(rest)}
            flags = {f.strip() for f in rest.split(",") if f.strip() and "=" not in f}
            nodes.append((d, n["handle"], n["role"], n["name"], props, flags))
        for c in n.get("children", []): walk(c, d + 1)
    walk(tree, 0)
    orig = formstate.parse
    formstate.parse = lambda _: nodes
    try: return formstate.table("")
    finally: formstate.parse = orig

stats = dict(states=0, max_axtree=0, max_nodes=0, max_depth=0, max_str=0, mismatch=[], steps_ok=True)
def depth_count(n, d=0):
    m, c, s = d, 1, max(len(n["role"].encode()), len(n["name"].encode()), len(n.get("props", "").encode()), len(n.get("handle", "").encode()))
    for ch in n.get("children", []):
        m2, c2, s2 = depth_count(ch, d + 1); m, c, s = max(m, m2), c + c2, max(s, s2)
    return m, c, s
sys.setrecursionlimit(10000)
for tid, t in T.items():
    prev = -1
    for k, st in enumerate(t["states"]):
        ev = formtable.state_event(t, k, st)
        if not (ev["step"] > prev and ev["step"] == k): stats["steps_ok"] = False
        prev = ev["step"]
        raw = st["accessibility_tree"] or ""
        md, nc, ms = depth_count(ev["tree"])
        stats["states"] += 1; stats["max_axtree"] = max(stats["max_axtree"], len(raw.encode())); stats["max_nodes"] = max(stats["max_nodes"], nc)
        stats["max_depth"] = max(stats["max_depth"], md); stats["max_str"] = max(stats["max_str"], ms, len(ev["url"].encode()), len(ev.get("action", {}).get("call", "").encode()))
        if tree_table(ev["tree"]) != formstate.table(raw): stats["mismatch"].append((tid, k))
check("engine input: steps strictly increasing and equal to state position", stats["steps_ok"])
check("engine input: within bounds (4 MiB text, 200000 nodes, depth < 128, strings <= 16384 bytes)", stats["max_axtree"] <= formtable.MAX_AXTREE and stats["max_nodes"] <= formtable.MAX_NODES and stats["max_depth"] < formtable.MAX_DEPTH and stats["max_str"] <= formtable.MAX_STR,
      f"{stats['states']} states; max text {stats['max_axtree']} B, nodes {stats['max_nodes']}, depth {stats['max_depth']}, string {stats['max_str']} B")
check("tree-only form table == formstate.table on raw text, every state", not stats["mismatch"], f"mismatches {len(stats['mismatch'])} {stats['mismatch'][:5]}")
wide = "\n".join("\t" * d + f"[{d}] generic 'n{d}'" for d in range(200))
md, nc, _ = depth_count(formtable.build_tree(wide))
check("build_tree clamps a 200-deep tree below depth 128", md < formtable.MAX_DEPTH and nc == 200, f"depth {md}, nodes {nc}")
check("build_tree fallback root", formtable.build_tree("") == {"role": "RootWebArea", "name": ""})

if a.loader:
    ref_t = fileio.read_pickle(a.loader)
    check("trajectory loader == reference pickle", T == ref_t, f"{len(T)} trajectories")

print(f"\n{'FAILED ' + str(len(fails)) if fails else 'all passed'}")
sys.exit(1 if fails else 0)
