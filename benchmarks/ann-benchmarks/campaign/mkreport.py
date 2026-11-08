"""mkreport.py <out.json> <gen>...: aggregate export.csv of generations into one results JSON + markdown on stdout.
Per (gen, dataset, arm): per-round QPS at recall>=0.95/0.99 (best QPS among points with recall >= thr, the harness
convention) and log-linear interpolation on the frontier; median over rounds; ratio to that gen's ctrl arm."""
import csv, glob, json, math, os, statistics as st, sys
RUNS = os.path.expanduser("~/ab-a/runs")
def frontier(rows):
    pts = sorted(((float(r["k-nn"]), float(r["qps"])) for r in rows), key=lambda x: -x[1]); out = []; best = -1
    for rec, q in pts:
        if rec > best: out.append((rec, q)); best = rec
    return sorted(out)
def at(rows, thr):
    ok = [float(r["qps"]) for r in rows if float(r["k-nn"]) >= thr]; hc = max(ok) if ok else None
    f = frontier(rows); ip = None
    for (r0, q0), (r1, q1) in zip(f, f[1:]):
        if r0 < thr <= r1: ip = math.exp(math.log(q0) + (thr - r0) / (r1 - r0) * (math.log(q1) - math.log(q0)))
    if ip is None and f and f[0][0] >= thr: ip = f[0][1]
    return hc, ip
out = {"generations": {}}; md = []
for gen in sys.argv[2:]:
    G = {}
    for p in sorted(glob.glob(f"{RUNS}/{gen}/*/export.csv")):
        arm, ds, r = os.path.basename(os.path.dirname(p)).split("__")
        rows = list(csv.DictReader(open(p)))
        if not rows: continue
        d = G.setdefault(ds, {}).setdefault(arm, {"rounds": {}, "params": sorted({x["parameters"] for x in rows})[:1]})
        d["rounds"][r] = {str(t): dict(zip(("hc", "interp"), at(rows, t))) for t in (0.95, 0.99)}
        d["rounds"][r]["max_recall"] = max(float(x["k-nn"]) for x in rows)
        d["rounds"][r]["points"] = [{"recall": float(x["k-nn"]), "qps": float(x["qps"]), "parameters": x["parameters"]} for x in rows]
    for ds, A in G.items():
        for arm, d in A.items():
            for t in ("0.95", "0.99"):
                for k in ("hc", "interp"):
                    v = [x[t][k] for x in d["rounds"].values() if x[t][k]]
                    d.setdefault("median", {}).setdefault(t, {})[k] = st.median(v) if v else None
                    d.setdefault("spread", {}).setdefault(t, {})[k] = (max(v) / min(v)) if len(v) > 1 else None
        ctrl = A.get("ctrl")
        md.append(f"\n### {gen} / {ds}\n\n| arm | n | 0.95 QPS (hc) | 0.95 interp | x ctrl | 0.99 QPS (hc) | 0.99 interp | x ctrl | max recall |\n|---|---|---|---|---|---|---|---|---|")
        for arm, d in sorted(A.items(), key=lambda kv: -(kv[1]["median"]["0.95"]["interp"] or 0)):
            m = d["median"]; c = (ctrl or {}).get("median", {})
            ratio = lambda t: (f"{m[t]['interp']/c[t]['interp']:.2f}" if m[t]["interp"] and c.get(t, {}).get("interp") else "-")
            f = lambda v: f"{v:,.0f}" if v else "-"
            md.append(f"| {arm} | {len(d['rounds'])} | {f(m['0.95']['hc'])} | {f(m['0.95']['interp'])} | {ratio('0.95')} | {f(m['0.99']['hc'])} | {f(m['0.99']['interp'])} | {ratio('0.99')} | {max(x['max_recall'] for x in d['rounds'].values()):.4f} |")
    out["generations"][gen] = G
json.dump(out, open(sys.argv[1], "w"), indent=1)
print("\n".join(md))
