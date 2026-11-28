import json, statistics as st, sys
path = sys.argv[1]
W = "warm-0.7.5-bp"
with open(path) as f:
    data = json.load(f)
res = data["results"]
engines = list(res["TOP_10"].keys())
cats = ["ALL", "union", "intersection", "phrase", "intersection_union", "negated", "term", "global"]
cmds = [c for c in ["TOP_10", "TOP_100", "TOP_1000", "TOP_100_COUNT", "COUNT"] if c in res]
def cell(cmd, eng, cat):
    qs = [q for q in res[cmd][eng] if cat == "ALL" or cat in q["tags"]]
    if not qs:
        return None
    best = [min(q["duration"]) for q in qs]
    return len(qs), st.mean(best), st.median(best)
out = []
out.append("Columns: " + " | ".join(engines) + "  (mean / median microseconds, per-query best of 10 runs)\n")
for cmd in cmds:
    out.append(f"## {cmd}\n")
    out.append("| category | n | " + " | ".join(engines) + " |")
    out.append("|---|---|" + "---|" * len(engines))
    for cat in cats:
        row = [cell(cmd, e, cat) for e in engines]
        if row[0] is None:
            continue
        out.append(f"| {cat} | {row[0][0]} | " + " | ".join(f"{r[1]:.0f} / {r[2]:.0f}" for r in row) + " |")
    out.append("")
out.append("## WARM losses (WARM slower than the named engine)\n")
out.append("Format: command / category: engine (WARM mean vs engine mean; WARM median vs engine median), listed when WARM is slower on that statistic.\n")
for cmd in cmds:
    for cat in cats:
        w = cell(cmd, W, cat)
        if w is None:
            continue
        for e in engines:
            if e == W:
                continue
            o = cell(cmd, e, cat)
            ml, dl = w[1] > o[1], w[2] > o[2]
            if ml or dl:
                kind = "mean+median" if ml and dl else ("mean only" if ml else "median only")
                out.append(f"- {cmd} / {cat} (n={w[0]}): {e}: {kind} (WARM {w[1]:.0f}/{w[2]:.0f} vs {o[1]:.0f}/{o[2]:.0f})")
print("\n".join(out))
