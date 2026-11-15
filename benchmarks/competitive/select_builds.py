"""Screening summary and build selection for the swept comparators.
Usage: select_builds.py <screening-results.json> [--md out.md]
Prints one line of `system:build` tokens (the builds that are best at any of recall 0.90/0.95/0.99 for that
system; hnswlib and hnswlib-cpp share the union because they serve the same index files) for `warm-bench run --builds`.
Score of a build at a recall level = mean QPS of the fastest sweep row whose recall (over the screening queries) reaches the level."""
import json, statistics, sys
T = (0.90, 0.95, 0.99)
r = json.load(open(sys.argv[1]))
md = sys.argv[sys.argv.index("--md") + 1] if "--md" in sys.argv else None
table, winners = {}, {}
for s, entry in r["systems"].items():
    table[s] = {}
    for b, bd in entry["builds"].items():
        cfgs = bd["serve_rounds"][0]["configs"]
        row = {"max_recall": max(c["recall_at_10"] for c in cfgs), "last_param": list(cfgs[-1]["params"].values())[-1], "rows": len(cfgs)}
        for t in T:
            ok = [statistics.mean(x["qps"] for x in c["repeats"]) for c in cfgs if c["recall_at_10"] >= t]
            row[t] = max(ok) if ok else None
        table[s][b] = row
    win = set()
    for t in T:
        reach = {b: v[t] for b, v in table[s].items() if v[t] is not None}
        if reach:
            win.add(max(reach, key=reach.get))
        else:
            win.add(max(table[s], key=lambda b: table[s][b]["max_recall"]))
    winners[s] = win
if "hnswlib" in winners and "hnswlib-cpp" in winners:
    winners["hnswlib"] = winners["hnswlib-cpp"] = winners["hnswlib"] | winners["hnswlib-cpp"]
lines = [f"screening: {r['dataset']['name']} n={r['dataset']['n_train']} queries={r['protocol']['queries']} (prefix), 1 round", "",
         "| system | build | QPS@0.90 | QPS@0.95 | QPS@0.99 | max recall | sweep rows | last param | selected |", "|---|---|---|---|---|---|---|---|---|"]
for s, bs in table.items():
    for b, v in bs.items():
        f = lambda x: "not reached" if x is None else f"{x:,.0f}"
        lines.append(f"| {s} | {b} | {f(v[0.90])} | {f(v[0.95])} | {f(v[0.99])} | {v['max_recall']:.4f} | {v['rows']} | {v['last_param']} | {'yes' if b in winners[s] else ''} |")
text = "\n".join(lines)
if md:
    open(md, "w").write(text + "\n")
print(text, file=sys.stderr)
print(" ".join(f"{s}:{b}" for s, bs in winners.items() for b in sorted(bs)))
