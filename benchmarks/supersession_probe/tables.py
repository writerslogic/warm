import json,sys
from pathlib import Path
r=json.loads(Path(sys.argv[1]).read_text())
def pct(x): return "n/a" if x is None else f"{100*x:.1f}"
def row(split,arm,scope,metrics):
    d=r[split][arm][scope]
    return " | ".join(pct(d[m]["rate"]) for m in metrics)
out=[]
M=["cva1","stale1","stale5","new_in5","cva_session1","old_sibling1"]
H="| arm | CVA@1 | Stale@1 | Stale@5 | new in top5 | CVA-session@1 | old-sibling@1 |\n|---|---|---|---|---|---|---|"
names={"a":"(a) plain hybrid","b":"(b) recency among top5","c":"(c) WARM oracle links, follow","c_filter":"(c-filter) oracle, stock filter","d":"(d) WARM detected links, follow"}
for scope,title in (("pooled","Pooled"),("T1","Type I"),("T2","Type II")):
    out.append(f"\n**TEST, {title}** (n queries = {r['test']['a'][scope]['cva1']['n']})\n\n"+H)
    for arm in ("a","b","c","c_filter","d"):
        out.append(f"| {names[arm]} | "+row("test",arm,scope,M)+" |")
out.append("\n**TEST CVA@1 by dim (pooled)**\n\n| arm | dim1 | dim2 | dim3 |\n|---|---|---|---|")
for arm in ("a","b","c","c_filter","d"):
    out.append(f"| {names[arm]} | "+row("test",arm,"pooled",["cva1_dim1","cva1_dim2","cva1_dim3"])+" |")
out.append("\n**DEV (arms a, b only; d is the simulation sweep below)**\n\n"+H)
for arm in ("a","b"):
    out.append(f"| {names[arm]} | "+row("dev",arm,"pooled",M)+" |")
out.append("\n**tau sweep on DEV (simulated arm d, pooled CVA@1)**\n\n| tau | CVA@1 |\n|---|---|")
for t in r["tau_sweep_dev"]: out.append(f"| {t['tau']:.2f} | {100*t['dev_pooled_cva1']:.1f} |")
out.append(f"\nChosen tau = {r['tau_star']}.")
out.append("\n**Paired cluster bootstrap on TEST (points of CVA@1, 95% interval over instances)**\n\n| x - y | pooled | T1 | T2 |\n|---|---|---|---|")
for b in r["bootstrap_test_points_cva1"]:
    f=lambda v: "n/a" if v is None else f"{v['points']:+.1f} [{v['lo95']:+.1f}, {v['hi95']:+.1f}]"
    out.append(f"| {b['x']} - {b['y']} | {f(b['pooled'])} | {f(b['T1'])} | {f(b['T2'])} |")
dd=r["decision"]
out.append(f"\nDecision: **{dd['result']}** (a={100*dd['a']:.1f}, b={100*dd['b']:.1f}, c={100*dd['c']:.1f}, d={100*dd['d']:.1f}). Real-vs-simulated arm d list mismatches on TEST: {r['d_real_vs_simulated_list_mismatches_test']}.")
print("\n".join(out))
