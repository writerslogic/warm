"""Lead A analysis: tables and the criterion A verdict from the k5a.py outputs.

usage: a_report.py OUTDIR
"""
import glob
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from common import boot_ci  # noqa: E402

ARMS = ["vsa", "col", "inv", "rowbf", "fpf", "pat"]
BASE = ["col", "inv", "rowbf", "fpf", "pat"]


def cell_table(d, rng):
    seeds = sorted(d["seeds"])
    bprs = d["bprs"]
    rows, summ = [], {"bits_at": {}, "wins": []}
    mean = {a: {} for a in ARMS}
    for b in bprs:
        per = {}
        for a in ARMS:
            vals = [d["seeds"][s][str(b)].get(a) for s in seeds if str(b) in d["seeds"][s]]
            vals = [v for v in vals if v is not None]
            if vals:
                per[a] = np.array(vals)
                mean[a][b] = float(per[a].mean())
        if "vsa" in per:
            n = len(per["vsa"])
            best = max(BASE, key=lambda a: per[a][:n].mean())
            diff = per["vsa"] - per[best][:n]
            m, lo, hi = boot_ci(diff, rng)
            win = bool(per["vsa"].mean() >= 0.9 and lo > 0.02)
            if win:
                summ["wins"].append(b)
            vs = f"{per['vsa'].mean():.3f}"
            ds = f"{best} {per[best][:n].mean():.3f}; d={m:+.3f} [{lo:+.3f},{hi:+.3f}]{' WIN' if win else ''}"
        else:
            vs = "dead(screen)" if b in d.get("screened_dead", []) or b in d.get("tune_dead", []) else "not run"
            if b in d.get("screened_saturated", []):
                vs = "saturated(screen)"
            best = max(BASE, key=lambda a: per[a].mean() if a in per else -1)
            ds = f"{best} {per[best].mean():.3f}"
        rows.append(f"| {b} | {vs} | " + " | ".join(f"{per[a].mean():.3f}" if a in per else "-" for a in BASE) + f" | {ds} |")
    for a in ARMS:
        for tgt in (0.99, 0.90):
            ok = [b for b in bprs if mean[a].get(b, -1) >= tgt]
            summ["bits_at"][f"{a}@{tgt}"] = min(ok) if ok else None
    return rows, summ


def main(outdir):
    rng = np.random.default_rng(5)
    md, js = [], {}
    for fn in sorted(glob.glob(f"{outdir}/a_R*_N*.json")) + sorted(glob.glob(f"{outdir}/p_R*_N*.json")):
        d = json.load(open(fn))
        name = os.path.basename(fn)[:-5]
        rows, summ = cell_table(d, rng)
        tuned = {b: (v["w"], v["kappa"], v["rho"], v["strong"], round(v["acc_tune"], 3)) for b, v in d["tune"].items()}
        js[name] = {"summary": summ, "tuned": tuned, "done": d.get("done", False),
                    "screened_dead": d.get("screened_dead", []), "tune_dead": d.get("tune_dead", []),
                    "mean_search_fail": float(np.mean([v["vsa_search_fail"] for s in d["seeds"].values() for v in s.values()
                                                       if "vsa_search_fail" in v] or [0]))}
        md.append(f"### {name}\n\n| bits/rec | VSA-res | COL | INV | ROWBF | FPF(model) | PAT(analytic) | best baseline |\n|---|---|---|---|---|---|---|---|\n"
                  + "\n".join(rows) + f"\n\nbits per record at accuracy target: {json.dumps(summ['bits_at'])}\n")
    for fn in sorted(glob.glob(f"{outdir}/o_R*_N*.json")):
        d = json.load(open(fn))
        keys = sorted({k for s in d["seeds"].values() for k in s}, key=lambda k: (int(k.split("|")[0]), k))
        md.append(f"### oracle {os.path.basename(fn)} (exhaustive decode of the same bundle, u=3)\n\n| bits/rec | w | acc | s per query |\n|---|---|---|---|\n"
                  + "\n".join(f"| {k.split('|')[0]} | {k.split('|')[1]} | {np.mean([s[k]['acc'] for s in d['seeds'].values() if k in s]):.3f} | "
                              f"{np.mean([s[k]['s_per_query'] for s in d['seeds'].values() if k in s]):.2f} |" for k in keys) + "\n")
    for fn in sorted(glob.glob(f"{outdir}/t_*.json")):
        d = json.load(open(fn))
        md.append(f"### timing {os.path.basename(fn)} (ms per query, single thread)\n\n| bits/rec | VSA-res | COL | INV | ROWBF | VSA acc in timed queries |\n|---|---|---|---|---|---|\n"
                  + "\n".join(f"| {b} | {m['vsa']:.1f} | {m['col']:.4f} | {m['inv']:.3f} | {m['rowbf']:.3f} | {m['vsa_acc_timed']:.2f} |"
                              for b, m in d["ms"].items()) + "\n")
    if os.path.exists(f"{outdir}/fpf.jsonl"):
        md.append("### FPF real xorf measurements\n\n```\n" + open(f"{outdir}/fpf.jsonl").read() + "```\n")
    open(f"{outdir}/a_tables.md", "w").write("\n".join(md))
    json.dump(js, open(f"{outdir}/a_summary.json", "w"), indent=1)
    wins = {k: v["summary"]["wins"] for k, v in js.items() if v["summary"]["wins"]}
    print("cells", len(js), "bits-win cells:", wins)


if __name__ == "__main__":
    main(sys.argv[1])
