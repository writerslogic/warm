"""Price item 4 (STATE.md 01:30Z) from l22c.json and the bench5 sustained/means.
For each arm: P* = smallest evaluated nprobe whose ceiling >= fixed's nprobe-350 ceiling at every eval op;
search(P*) = sum over searches >= train op of [table(P*) + live * f_arm(P*, op) * Q / rate(P*)] (table and
rate linear in nprobe between bench points, mean components); cost = maint_s + (assign_s - fixed) +
31 s x (mean lists / 5,000 - 1). usage: l22c_price.py RUNBOOK L22C_JSON BENCH_JSON [BENCH_JSON ...]"""
import importlib.util, json, os, sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
s_ = importlib.util.spec_from_file_location("pt", os.path.join(HERE, "project_total.py"))
pt = importlib.util.module_from_spec(s_)
s_.loader.exec_module(pt)
TRAIN_OP, Q, CENTRE_S = 32, 10_000, 31.0


def main(runbook, cj, benches):
    d = json.load(open(cj))
    rows = d["eval"]
    tiers = d["args"]["tiers"]
    pts = {}
    for p in benches:
        b = json.load(open(p))
        if "scan_admit_stats" in b:
            pts[b["nprobe"]] = (b["lut_build_stats"]["mean"], b["codes_scanned"] / b["scan_admit_stats"]["mean"])
    ks = sorted(pts)
    table = lambda P: float(np.interp(P, ks, [pts[k][0] for k in ks]))
    rate = lambda P: float(np.interp(P, ks, [pts[k][1] for k in ks]))
    lives = []
    live = 0
    for o in pt.parse(runbook):
        op = o.get("operation")
        if op == "insert":
            live += int(o["end"]) - int(o["start"])
        elif op == "delete":
            live -= int(o["end"]) - int(o["start"])
        elif op == "search" and o["i"] >= TRAIN_OP:
            lives.append((o["i"], live))
    target = {r["op"]: r["fixed"]["350"]["recall"] for r in rows}
    fixed_assign = rows[-1]["fixed"]["assign_s"]
    arms = [k for k in rows[0] if k not in ("op", "live")]
    print(f"bench points (mean table s, rate): " + ", ".join(f"{k}: {pts[k][0]:.3f}/{pts[k][1]:.3e}" for k in ks))
    out = {}
    for arm in arms:
        ok = [P for P in tiers if all(r[arm][str(P)]["recall"] >= target[r["op"]] - 1e-12 for r in rows)]
        if not ok:
            print(f"{arm}: no evaluated nprobe reaches fixed@350 at every op")
            continue
        P = ok[0]
        f, _ = pt.f_curve(cj, arm, P)
        tab = len(lives) * table(P)
        scan = sum(l * f(i) * Q / rate(P) for i, l in lives)
        last = rows[-1][arm]
        mean_lists = float(np.mean([r[arm]["lists"] for r in rows]))
        extra = last["maint_s"] + (last["assign_s"] - fixed_assign) + CENTRE_S * (mean_lists / 5000 - 1)
        total = tab + scan + extra
        out[arm] = total
        worst = min(r[arm][str(P)]["recall"] - target[r["op"]] for r in rows)
        print(f"{arm}: P*={P} (worst margin vs fixed@350 {worst:+.4f}) tables {tab:.0f} + scan {scan:.0f} + maint {last['maint_s']:.0f}"
              f" + extra assign {last['assign_s'] - fixed_assign:.0f} + centre {CENTRE_S * (mean_lists / 5000 - 1):.0f} = {total:.0f} s"
              f"  [lists final {last['lists']}, splits {last['splits']}, moved {last['moved']}, unpriced check_s {last['check_s']:.0f}]")
    if "fixed" in out:
        for arm, v in out.items():
            if arm != "fixed":
                print(f"{arm} vs fixed: {v - out['fixed']:+.0f} s ({'ADOPT' if out['fixed'] - v > 50 else 'reject'} at the 50 s bar)")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3:])
