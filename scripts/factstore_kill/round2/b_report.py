"""Lead B analysis: select configurations on the tuning seeds, then summarise the report seeds.

usage: b_report.py select OUTDIR SRC   -> writes OUTDIR/wanted_SRC.json from OUTDIR/b_tune_SRC_s*.json
       b_report.py stats  OUTDIR SRC   -> writes OUTDIR/b_summary_SRC.json and b_tables_SRC.md
"""
import glob
import json
import sys

import numpy as np

sys.path.insert(0, "/Volumes/A/.hms-lb/factstore-20261114/kill")
from common import boot_ci  # noqa: E402

FLAG = {("locomo", 100, 16384, "sub", 0.3): "C1", ("locomo", 300, 16384, "sub", 0.2): "C2",
        ("locomo", 300, 65536, "sub", 0.3): "C3"}


def fam_B(cfg):
    f = cfg[0]
    return (f, cfg[3]) if f in ("vsa", "sig") else (f, cfg[1])


def load(outdir, src, pat):
    runs = {}
    for fn in sorted(glob.glob(f"{outdir}/{pat}")):
        d = json.load(open(fn))
        for seed, res in d.items():
            runs[int(seed)] = res
    return runs


def parse(k):
    cfg, N, typ, lvl = json.loads(k)
    return tuple(cfg), N, typ, lvl


def select(outdir, src):
    runs = load(outdir, src, f"b_tune_{src}_s*.json")
    acc = {}
    for seed, res in runs.items():
        for k, v in res.items():
            cfg, N, typ, lvl = parse(k)
            acc.setdefault((N, typ, lvl, fam_B(cfg)), {}).setdefault(cfg, []).append(v[0])
    sel = {}
    for cell, d in acc.items():
        best = max(d, key=lambda c: np.mean(d[c]))
        sel[cell] = best
    wanted = sorted({json.dumps(list(c)) for c in sel.values()})
    json.dump([json.loads(w) for w in wanted], open(f"{outdir}/wanted_{src}.json", "w"))
    json.dump({json.dumps([c[0], c[1], c[2], list(c[3])]): list(v) for c, v in sel.items()}, open(f"{outdir}/selected_{src}.json", "w"))
    print(src, "cells", len(sel), "distinct configs", len(wanted))


def stats(outdir, src):
    sel = {tuple([c[0], c[1], c[2], tuple(c[3])]): tuple(v) for c, v in
           ((json.loads(k), v) for k, v in json.load(open(f"{outdir}/selected_{src}.json")).items())}
    runs = load(outdir, src, f"b_rep_{src}_s*.json")
    seeds = sorted(runs)
    rng = np.random.default_rng(11)
    out, lines = {}, []
    cells = sorted({(c[0], c[1], c[2]) for c in sel})
    Bs = sorted({c[3][1] for c in sel})
    for N, typ, lvl in cells:
        for B in Bs:
            fams = {c[3][0]: v for c, v in sel.items() if c[:3] == (N, typ, lvl) and c[3][1] == B}
            if "vsa" not in fams:
                continue
            arr = {}
            for f, cfg in fams.items():
                key = json.dumps([list(cfg), N, typ, lvl])
                try:
                    arr[f] = np.array([runs[s][key] for s in seeds])  # (seeds, 9)
                except KeyError:
                    continue
            if "vsa" not in arr:
                continue
            base = [f for f in arr if f != "vsa"]
            if not base:
                continue
            ba = max(base, key=lambda f: arr[f][:, 0].mean())
            bt = max(base, key=lambda f: arr[f][:, 1].mean())
            cell = {"vsa_cfg": list(fams["vsa"]), "best_auc_family": ba, "best_tpr1_family": bt, "n_seeds": len(seeds)}
            for proto, off in (("M", 0), ("C", 3), ("K", 6)):
                cell[proto] = {f: {"auc": float(arr[f][:, off].mean()), "tpr1": float(arr[f][:, off + 1].mean()),
                                   "tpr01": float(arr[f][:, off + 2].mean())} for f in arr}
            for proto, off in (("M", 0), ("C", 3)):
                da = arr["vsa"][:, off] - arr[ba][:, off]
                dt = arr["vsa"][:, off + 1] - arr[bt][:, off + 1]
                cell[f"d_auc_{proto}"] = boot_ci(da, rng)
                cell[f"d_tpr1_{proto}"] = boot_ci(dt, rng)
                cell[f"d_auc_{proto}_999"] = boot_ci(da, rng, n=20000)
                cell[f"d_tpr1_{proto}_999"] = boot_ci(dt, rng, n=20000)
            cell["win_M"] = bool(cell["d_auc_M"][1] > 0.01 and cell["d_tpr1_M"][1] > 0)
            cell["win_C"] = bool(cell["d_auc_C"][1] > 0.01 and cell["d_tpr1_C"][1] > 0)
            cell["win_M_999"] = bool(cell["d_auc_M_999"][1] > 0.01 and cell["d_tpr1_M_999"][1] > 0)
            cell["flag"] = FLAG.get((src, N, B, typ, lvl), "")
            out[f"N={N}|B={B}|{typ}|{lvl}"] = cell
            m, c, k = cell["M"], cell["C"], cell["K"]
            lines.append(
                f"| {N} | {B} | {typ} {lvl} | {cell['flag']} | {ba} {m[ba]['auc']:.3f} / {bt} {m[bt]['tpr1']:.3f} | "
                f"{m['vsa']['auc']:.3f} / {m['vsa']['tpr1']:.3f} / {m['vsa']['tpr01']:.3f} | "
                f"{cell['d_auc_M'][0]:+.3f} [{cell['d_auc_M'][1]:+.3f},{cell['d_auc_M'][2]:+.3f}] | "
                f"{cell['d_tpr1_M'][0]:+.3f} [{cell['d_tpr1_M'][1]:+.3f},{cell['d_tpr1_M'][2]:+.3f}] | "
                f"{'WIN' if cell['win_M'] else '-'} | {c['vsa']['auc']:.3f} vs {c[ba]['auc']:.3f} ({'WIN' if cell['win_C'] else '-'}) | "
                f"K: {k['vsa']['auc']:.3f} / {k[ba]['auc']:.3f} |")
    hdr = ("| N | B | noise | flag | best baseline AUC(M) / best TPR1(M) | VSA AUC / TPR1 / TPR0.1 (M) | dAUC (M) [95% CI] | "
           "dTPR1 (M) [95% CI] | win M | protocol C VSA vs best | control K AUC VSA / best |\n|" + "---|" * 11 + "\n")
    open(f"{outdir}/b_tables_{src}.md", "w").write(hdr + "\n".join(lines) + "\n")
    json.dump(out, open(f"{outdir}/b_summary_{src}.json", "w"), indent=1)
    wins = [k for k, v in out.items() if v["win_M"]]
    print(src, "cells", len(out), "wins M", len(wins), "wins C", sum(v["win_C"] for v in out.values()),
          "wins M 99.9%", sum(v["win_M_999"] for v in out.values()))


if __name__ == "__main__":
    {"select": select, "stats": stats}[sys.argv[1]](sys.argv[2], sys.argv[3])
