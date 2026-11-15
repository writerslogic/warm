"""Apply the PREREG kill rule to the result JSONs and print markdown tables."""
import json
import sys
from pathlib import Path

d = Path(sys.argv[1])
out = []


def k1(name, eff=0.01):
    p = d / name
    if not p.exists():
        return
    r = json.load(open(p))
    out.append(f"### K1 near-key membership, {r['source']} keys (AUC; VSA = best of w, kappa tuned on tuning seeds)\n")
    out.append("| N | B | level | strongest baseline | baseline AUC | VSA AUC | VSA - baseline [95% CI] | VSA TPR@1% - baseline [CI] | win cell |")
    out.append("|---|---|---|---|---|---|---|---|---|")
    wins = {}
    for N, cells in r["results"].items():
        for key, c in cells.items():
            B, lv = key.split("|")
            sb = c["strongest_baseline"]
            diff = c.get("vsa_minus_strongest_auc")
            tp = c.get("vsa_minus_strongest_tpr1")
            if not diff:
                continue
            win = diff["ci95"][0] > eff
            if win:
                wins.setdefault((N, B), set()).add(lv)
            out.append(
                f"| {N} | {B[2:]} | {lv[6:]} | {sb} | {c['arms'][sb]['auc']:.3f} | {c['arms']['vsa']['auc']:.3f} | "
                f"{diff['mean']:+.3f} [{diff['ci95'][0]:+.3f}, {diff['ci95'][1]:+.3f}] | "
                f"{tp['mean']:+.3f} [{tp['ci95'][0]:+.3f}, {tp['ci95'][1]:+.3f}] | {'WIN' if win else '-'} |"
            )
    out.append(f"\nCells with CI lower bound > +{eff}: {len(wins)} (cell = (N,B) with any level)\n")


def k2(name):
    p = d / name
    if not p.exists():
        return
    r = json.load(open(p))
    out.append(f"### K2 near-key lookup, {r['source']} keys (top-1 accuracy)\n")
    out.append("| N | B | level | strongest baseline | baseline acc | VSA acc | VSA - baseline [95% CI] |")
    out.append("|---|---|---|---|---|---|---|")
    for key, c in r["results"].items():
        sb = c["strongest_baseline"]
        if not sb or "vsa" not in c["arms"]:
            continue
        df = c["vsa_minus_strongest"]
        out.append(f"| {key.replace('|', ' ')} | {sb} | {c['arms'][sb]['acc']:.3f} | {c['arms']['vsa']['acc']:.3f} | "
                   f"{df['mean']:+.3f} [{df['ci95'][0]:+.3f}, {df['ci95'][1]:+.3f}] |")
    out.append("")


def k3(name):
    p = d / name
    if not p.exists():
        return
    r = json.load(open(p))["results"]
    out.append("### K3 set similarity (RMSE of Jaccard estimate; ratio = VSA / best b-bit MinHash)\n")
    out.append("| n | B | J | best MinHash | RMSE | best VSA | RMSE | ratio |")
    out.append("|---|---|---|---|---|---|---|---|")
    n_better = 0
    for key, c in r.items():
        n, B, J = [x.split("=")[1] for x in key.split("|")]
        n_better += c["vsa_over_minhash"] < 0.9
        out.append(f"| {n} | {B} | {J} | {c['best_minhash']} | {c['rmse_minhash']:.4f} | {c['best_vsa']} | {c['rmse_vsa']:.4f} | {c['vsa_over_minhash']:.2f} |")
    out.append(f"\nCells where VSA RMSE is >10% lower: {n_better} of {len(r)}\n")


def k4(name, title, cols):
    p = d / name
    if not p.exists():
        return
    r = json.load(open(p))
    out.append(f"### {title}\n")
    out.append("| cell | " + " | ".join(cols) + " | strongest baseline | VSA - baseline [95% CI] |")
    out.append("|---|" + "---|" * (len(cols) + 2))
    for key, c in r.items():
        if "vsa_minus_strongest" not in c:
            continue
        df = c["vsa_minus_strongest"]
        out.append(f"| {key.replace('|', ' ')} | " + " | ".join(f"{c[x]['acc']:.3f}" if x in c else "" for x in cols)
                   + f" | {c['strongest_baseline']} | {df['mean']:+.3f} [{df['ci95'][0]:+.3f}, {df['ci95'][1]:+.3f}] |")
    out.append("")


k1("k1_synth.json")
k1("k1_locomo.json")
k2("k2_synth.json")
k2("k2_locomo.json")
k3("k3.json")
k4("k4b.json", "K4b: single missing role, bits per record below and above the information content (R=6, V=256)",
   ["table_subset", "bloomtriples", "vsa"])
k4("k4c.json", "K4c: u unknown roles, strongest conventional = parity-address tables (pat) or exact table",
   ["table_subset", "pat", "vsa"])
print("\n".join(out))
