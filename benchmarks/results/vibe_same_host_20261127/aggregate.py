"""Aggregate raw_per_block.csv (one row per block/system/dataset/config) into full.csv, pareto.csv, targets.csv.

Per config: strict ID recall = minimum over blocks, QPS = median over blocks with min and max
(per-block QPS = 1 / mean stored per-query time of VIBE's last of 5 passes). Only configs
present in every block are kept.
"""
import csv, re, statistics, collections, sys
rows = list(csv.DictReader(open("raw_per_block.csv")))
SYS = {"new": "warm-541", "base": "warm-pre541", "roar": None, "mlann": None}
def method(r):
    u, n = r["unit"], r["name"]
    if u == "roar":
        return "roargraph%s" % re.search(r"M_pjbp=(\d+)", n).group(1)
    if u == "mlann":
        return "mlann_rf%s_depth%s" % (re.search(r"n_trees=(\d+)", n).group(1), re.search(r"depth=(\d+)", n).group(1))
    return SYS[u]
def config(r):
    n = r["name"]
    if r["unit"] == "roar": return "L=" + re.search(r"L_pq=(\d+)", n).group(1)
    if r["unit"] == "mlann": return "t=" + re.search(r"voting_thresh=([\d.e+-]+)", n).group(1).rstrip(")")
    m = re.search(r"ef=(\d+), rerank=(\d+)", n); ef, rr = m.groups()
    lists = re.search(r"query_lists=(\d+)", n); pr = re.search(r"probes=(\d+)", n); po = "probe_only=False" in n   # the adapter omits probe_only from the name when it is true
    s = "ef=%s,rr=%s" % (ef, rr)
    if pr and int(pr.group(1)) > 0:
        s += ",P=%s,%s,lists=%s" % (pr.group(1), "graph-seeded" if po else "probe-only", lists.group(1) if lists else "?")
    else: s += ",P=0"
    return s
g = collections.defaultdict(list)
for r in rows:
    # the first yi new block ran the full pre-registered grid; keep only configs seen in all 3 blocks
    g[(r["dataset"], method(r), r["file"], r["group"], r["name"])].append(r)
full = []
for (ds, m, f, grp, n), rs in g.items():
    if len({r["block"] for r in rs}) != 3: continue
    q = [float(r["qps_last"]) for r in rs]
    full.append(dict(phase="same-host", dataset=ds, method=m, configuration=config(rs[0]) if m else "",
                     recall_ids_min=min(float(r["recall_f64"]) for r in rs), qps_median=statistics.median(q), qps_min=min(q), qps_max=max(q)))
full.sort(key=lambda d: (d["dataset"], d["method"], -d["recall_ids_min"]))
F = ["phase", "dataset", "method", "configuration", "recall_ids_min", "qps_median", "qps_min", "qps_max"]
def dump(fn, rs):
    w = csv.DictWriter(open(fn, "w"), F); w.writeheader(); w.writerows(rs)
dump("full.csv", full)
front = []
for key in {(d["dataset"], d["method"]) for d in full}:
    pts = sorted((d for d in full if (d["dataset"], d["method"]) == key), key=lambda d: (-d["recall_ids_min"], -d["qps_median"]))
    best = -1
    for d in pts:   # descending recall: keep a point only if faster than all higher-recall points
        if d["qps_median"] > best: front.append(d); best = d["qps_median"]
front.sort(key=lambda d: (d["dataset"], d["method"], -d["recall_ids_min"]))
dump("pareto.csv", front)
T = (0.90, 0.95, 0.99)
out = []
for ds in sorted({d["dataset"] for d in full}):
    for m in sorted({d["method"] for d in full if d["dataset"] == ds}):
        for t in T:
            c = [d for d in full if d["dataset"] == ds and d["method"] == m and d["recall_ids_min"] >= t]
            b = max(c, key=lambda d: d["qps_median"]) if c else None
            out.append(dict(dataset=ds, method=m, target=t, recall_ids_min=b and b["recall_ids_min"], qps_median=b and b["qps_median"],
                            qps_min=b and b["qps_min"], qps_max=b and b["qps_max"], configuration=b and b["configuration"]))
w = csv.DictWriter(open("targets.csv", "w"), list(out[0])); w.writeheader(); w.writerows(out)
