#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Simulate the STALE arms over the probe's dumped ranked lists (protocol: docs/research/cue-tags.md).

usage: stale_arms.py PROBE_JSONL DUMP_JSONL FLAGS_JSON OUT_JSON

DUMP_JSONL comes from `examples/supersession_probe.rs --dump`: per instance, arm a's top-50 lists
(history = All, independent of links) and the cosine link candidates (cos >= 0.60, at most 256,
sorted by cos then index). FLAGS_JSON maps uid -> cue-flagged turn ids (measure.py).
"""
import json, random, struct, sys

SHOW = 5
TAUS = [0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95, 1.01]
SEED = 20261103


def f32(x):
    return struct.unpack("f", struct.pack("f", x))[0]


def measure(inst, lst, sess):
    top = lst[:SHOW]
    first = top[0] if top else None
    return {"cv1": first == inst["new_id"], "new5": inst["new_id"] in top,
            "stale1": first == inst["old_id"], "stale5": inst["old_id"] in top,
            "sess1": first is not None and sess[first] == inst["new_session"],
            "oldsib1": first is not None and sess[first] == inst["old_session"]}


def follow(hits, succ, ts):
    """Port of `follow`/`resolve` in examples/supersession_probe.rs."""
    def resolve(i):
        if i not in succ:
            return i
        seen, stack, best = {i}, [i], None
        while stack:
            for nxt in succ.get(stack.pop(), []):
                if nxt in seen:
                    continue
                seen.add(nxt)
                if nxt not in succ:
                    key = (ts[nxt], nxt)
                    if best is None or key[0] > best[0] or (key[0] == best[0] and key[1] < best[1]):
                        best = key
                else:
                    stack.append(nxt)
        return best[1] if best else None

    out = []
    for h in hits:
        r = resolve(h)
        if r is not None and r not in out:
            out.append(r)
            if len(out) == SHOW:
                break
    return out


def succ_of(supersedes):
    succ = {}
    for x, olds in supersedes.items():
        for y in olds:
            succ.setdefault(y, []).append(x)
    return succ


probe_path, dump_path, flags_path, out_path = sys.argv[1:5]
probe = {i["uid"]: i for i in map(json.loads, open(probe_path))}
dump = [json.loads(l) for l in open(dump_path)]
flags = {u: set(v) for u, v in json.load(open(flags_path)).items()}


def links(d, tau, gate, top1=False):
    inst = probe[d["uid"]]
    t = f32(tau)
    sup = {}
    for turn, cand in zip(inst["turns"], d["cand"]):
        if gate is not None and turn["id"] not in gate:
            continue
        ys = [y for y, c in cand if f32(c) >= t]
        if top1:
            ys = ys[:1]
        if ys:
            sup[turn["id"]] = ys
    return sup


def run(d, arm, tau=None):
    inst = probe[d["uid"]]
    sess = {t["id"]: t["session"] for t in inst["turns"]}
    ts = {t["id"]: t["ts"] for t in inst["turns"]}
    if arm == "a":
        return [measure(inst, r, sess) for r in d["ranked"]]
    if arm == "b":
        return [measure(inst, sorted(r[:SHOW], key=lambda i: -ts[i]), sess) for r in d["ranked"]]
    gate = flags[d["uid"]]
    sup = {"c": lambda: {inst["new_id"]: [inst["old_id"]]},
           "e_o": lambda: {inst["new_id"]: [inst["old_id"]]} if inst["new_id"] in gate else {},
           "d": lambda: links(d, tau, None),
           "e_x": lambda: links(d, tau, gate),
           "e_x1": lambda: links(d, tau, gate, top1=True)}[arm]()
    succ = succ_of(sup)
    return [measure(inst, follow(r, succ, ts), sess) for r in d["ranked"]]


def pooled(items, arm, tau=None, metric="cv1", kind=None):
    ms = [m for d in items if kind is None or d["type"] == kind for m in run(d, arm, tau)]
    return 100.0 * sum(m[metric] for m in ms) / len(ms), len(ms)


def bootstrap(items, x, y):
    # Paired cluster bootstrap over instances, 10,000 resamples.
    diffs = [sum(m["cv1"] for m in run(d, x[0], x[1])) - sum(m["cv1"] for m in run(d, y[0], y[1])) for d in items]
    rng = random.Random(SEED)
    n = len(diffs)
    draws = sorted(sum(diffs[rng.randrange(n)] for _ in range(n)) / (3 * n) for _ in range(10_000))
    return {"points": 100 * sum(diffs) / (3 * n), "lo95": 100 * draws[249], "hi95": 100 * draws[9_749]}


dev = [d for d in dump if d["split"] == "dev"]
test = [d for d in dump if d["split"] == "test"]
res = {"instances": {"dev": len(dev), "test": len(test)}, "tau_sweep_dev": {}}
for arm in ("d", "e_x", "e_x1"):
    sweep = [(pooled(dev, arm, tau)[0], tau) for tau in TAUS]
    res["tau_sweep_dev"][arm] = sweep
    best = max(sweep, key=lambda st: (st[0], st[1]))
    res[f"tau_{arm}"] = best[1]
res["tau_d_reference"] = 0.75
arms = {"a": ("a", None), "b": ("b", None), "c": ("c", None), "d@0.75": ("d", 0.75),
        "d@dev": ("d", res["tau_d"]), "e_x": ("e_x", res["tau_e_x"]), "e_x1": ("e_x1", res["tau_e_x1"]),
        "e_o": ("e_o", None)}
res["test"] = {}
for name, (arm, tau) in arms.items():
    row = {}
    for metric in ("cv1", "stale1", "stale5", "new5", "sess1", "oldsib1"):
        row[metric], n = pooled(test, arm, tau, metric)
    row["n"] = n
    row["cv1_T1"] = pooled(test, arm, tau, "cv1", "T1")[0]
    row["cv1_T2"] = pooled(test, arm, tau, "cv1", "T2")[0]
    res["test"][name] = row
res["dev_cv1"] = {name: pooled(dev, arm, tau)[0] for name, (arm, tau) in arms.items() if arm != "c" or True}
ref = {"a": 3.75, "b": 23.75, "c": 74.583, "d@0.75": 9.583}
res["reproduction_gate"] = {k: {"sim": res["test"][k]["cv1"], "reference": v,
                                "ok": abs(res["test"][k]["cv1"] - v) < 0.05} for k, v in ref.items()}
res["bootstrap_test"] = {f"{x}-{y}": bootstrap(test, arms[x], arms[y])
                         for x, y in [("e_x", "b"), ("e_x", "d@dev"), ("e_x", "a"), ("e_o", "b"), ("e_x1", "b")]}
b = res["test"]["b"]["cv1"]
ex = res["test"]["e_x"]["cv1"]
gate_ok = all(v["ok"] for v in res["reproduction_gate"].values())
res["decision"] = {"rule": "KILL if e_x - b < 2.0; GO if e_x - b >= 10; else NO-GO (gate must pass)",
                   "b": b, "e_x": ex, "lift": ex - b, "e_o": res["test"]["e_o"]["cv1"],
                   "e_o_vs_b_plus_15": res["test"]["e_o"]["cv1"] - (b + 15),
                   "result": "NOT REPORTABLE" if not gate_ok else
                   ("KILL" if ex - b < 2.0 else "GO" if ex - b >= 10 else "NO-GO")}
json.dump(res, open(out_path, "w"), indent=1)
print(json.dumps({k: res[k] for k in ("reproduction_gate", "tau_e_x", "tau_e_x1", "tau_d", "decision")}, indent=1))
for name, row in res["test"].items():
    print(f'{name:8s} ' + " ".join(f"{k}={v:.1f}" for k, v in row.items() if k != "n"), row["n"])
print(json.dumps(res["bootstrap_test"], indent=1))
