# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Generates form_parity.json: windows of four consecutive real states from the public
LongMemEval-V2 trajectories (Apache-2.0), each with the form-control blocks the reference
renderer of the benchmark adapter prepends to them (formstate.py and the block construction of
its memory module), chosen greedily to cover the control roles and properties.

Usage: python generate_form_parity_fixture.py DIR_WITH_formstate.py trajectories.jsonl OUTPUT.json
"""
import json, sys
sys.path.insert(0, sys.argv[1])
import formstate

SRC = sys.argv[2]
OUT = sys.argv[3]
WIN = 4
MAX_WINDOW_BYTES = 26000


def pre_blocks(texts):
    shown = set(); out = []
    for raw in texts:
        rows = formstate.table(raw); new = [r for r in rows if r not in shown]; shown.update(rows)
        pre = ""
        if rows:
            pre = "- Form controls (structured from the page text; * = selected option):\n" + "\n".join("  " + r for r in new) + (f"\n  [{len(rows) - len(new)} controls unchanged from an earlier state omitted]" if len(new) < len(rows) else "") + "\n"
        out.append(pre)
    return out


FEATURES = ["textbox", "searchbox", "combobox", "listbox", "checkbox", "radio", "switch", "spinbutton", "button", "tab", "menuitemcheckbox", "menuitemradio",
            "value=", "checked=", "selected=", "expanded=", "pressed=", "required", "disabled", "readonly", "options:", "*", "more)", "not ", "unchanged", "\\x", "\""]


def feats(blocks):
    s = "\n".join(blocks); return {f for f in FEATURES if f in s}


cands = []
with open(SRC) as fh:
    for line in fh:
        t = json.loads(line)
        st = t["states"]
        for a in range(0, max(0, len(st) - WIN + 1)):
            w = st[a:a + WIN]
            texts = [s["accessibility_tree"] or "" for s in w]
            size = sum(len(x.encode()) for x in texts)
            if size > MAX_WINDOW_BYTES:
                continue
            blocks = pre_blocks(texts)
            if sum(1 for b in blocks if b) < 3:
                continue
            cands.append((t["id"], t["domain"], a, w, blocks, size, feats(blocks)))
print("candidates", len(cands), file=sys.stderr)

chosen = []; covered = set(); used = set(); total = 0; domains = {"web": 0, "enterprise": 0}
while len(chosen) < 7:
    best = None
    for c in cands:
        if c[0] in used:
            continue
        gain = len(c[6] - covered) * 10 + (3 if domains[c[1]] < 3 else 0) - c[5] / 10000
        if best is None or gain > best[0]:
            best = (gain, c)
    if best is None:
        break
    c = best[1]; chosen.append(c); used.add(c[0]); covered |= c[6]; total += c[5]; domains[c[1]] += 1
print("covered", sorted(covered), "missing", sorted(set(FEATURES) - covered), "bytes", total, domains, file=sys.stderr)

fixture = {
    "source": "LongMemEval-V2 trajectories.jsonl (Apache-2.0), https://github.com/xiaowu0162/LongMemEval-V2",
    "reference": "warm_memory/formstate.py sha256 561732f9b2532a6aeb24fc2b1dcbc2e927d8e1c6cd2c37ce6dea414a346c1906 with the block construction of warm_memory/arch3_span.py",
    "groups": [
        {"trajectory": c[0], "domain": c[1],
         "states": [{"step": s["state_index"], "url": s["url"], "action": s.get("action"), "accessibility_tree": s["accessibility_tree"] or ""} for s in c[3]],
         "expected": c[4]}
        for c in chosen
    ],
}
with open(OUT, "w") as f:
    json.dump(fixture, f, ensure_ascii=False, indent=1)
print("states", sum(len(g["states"]) for g in fixture["groups"]), file=sys.stderr)
