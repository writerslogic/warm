"""summ.py <gen> [--dump]: per (arm,dataset,round) QPS at recall >= 0.95/0.99 (harness convention: best qps among
points with k-nn >= thr) and the log-qps linear interpolation between the bracketing frontier points; medians over rounds."""
import csv, glob, math, os, sys, statistics as st, collections
gen = sys.argv[1]; RUNS = os.path.expanduser("~/ab-a/runs")
def frontier(rows):
    pts = sorted(((float(r["k-nn"]), float(r["qps"])) for r in rows), key=lambda x: -x[1])
    out = []; best = -1
    for rec, q in pts:
        if rec > best: out.append((rec, q)); best = rec
    return sorted(out)                      # ascending recall, descending qps
def at(rows, thr):
    ok = [float(r["qps"]) for r in rows if float(r["k-nn"]) >= thr]
    hc = max(ok) if ok else None
    f = frontier(rows); interp = None
    for (r0, q0), (r1, q1) in zip(f, f[1:]):
        if r0 < thr <= r1:
            t = (thr - r0) / (r1 - r0); interp = math.exp(math.log(q0) + t * (math.log(q1) - math.log(q0)))
    if interp is None and f and f[-1][0] >= thr and f[0][0] >= thr: interp = f[0][1]
    return hc, interp
res = collections.defaultdict(list)
for p in sorted(glob.glob(f"{RUNS}/{gen}/*/export.csv")):
    arm, ds, r = os.path.basename(os.path.dirname(p)).split("__")
    rows = list(csv.DictReader(open(p)))
    if not rows: continue
    res[(ds, arm)].append((r, {t: at(rows, t) for t in (0.95, 0.99)}, max(float(x["k-nn"]) for x in rows)))
f = lambda v: "-" if v is None else f"{v:,.0f}"
print(f"{'dataset':30s} {'arm':22s} {'n':>2s} | {'0.95 hc med':>11s} {'interp':>8s} {'rounds(hc)':>26s} | {'0.99 hc med':>11s} {'interp':>8s} {'rounds(hc)':>26s} | maxrec")
for (ds, arm), L in sorted(res.items()):
    cols = []
    for t in (0.95, 0.99):
        hc = [x[1][t][0] for x in L if x[1][t][0] is not None]; ip = [x[1][t][1] for x in L if x[1][t][1] is not None]
        cols.append((st.median(hc) if hc else None, st.median(ip) if ip else None, "/".join(f(x[1][t][0]) for x in L)))
    print(f"{ds:30s} {arm:22s} {len(L):2d} | {f(cols[0][0]):>11s} {f(cols[0][1]):>8s} {cols[0][2]:>26s} | {f(cols[1][0]):>11s} {f(cols[1][1]):>8s} {cols[1][2]:>26s} | {max(x[2] for x in L):.4f}")
