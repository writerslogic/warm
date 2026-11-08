"""summ.py RESULTS.jsonl [CONTROL_LABEL]: per-arm means over rounds with spread, % vs the control,
digest/recall agreement. Labels are `arm:rN`."""
import collections, json, statistics as st, sys

rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip().startswith("{")]
ctrl = sys.argv[2] if len(sys.argv) > 2 else rows[0]["label"].split(":")[0]
arms = collections.OrderedDict()
for r in rows:
    arms.setdefault(r["label"].split(":")[0], []).append(r)
fields = ["wall_s", "insert_s", "delete_s", "search_s", "read_s"]
ph = ["scan_s", "lut_mean_s", "reorder_s", "centres_s", "route_s"]
def m(rs, f):
    v = [x[f] if f in x else x["stats"][f] for x in rs]
    return st.mean(v), (max(v) - min(v)), v
base = arms[ctrl]
print(f"{'arm':8s} n | " + " ".join(f"{f[:-2]:>13s}" for f in fields + ph) + " | rss  recall   digest")
for a, rs in arms.items():
    cells = []
    for f in fields + ph:
        mu, sp, _ = m(rs, f)
        b = m(base, f)[0]
        cells.append(f"{mu:7.1f}{(mu / b - 1) * 100:+5.1f}%" if b else f"{mu:7.1f}")
    dg = {x["digest"] for x in rs}
    print(f"{a:8s} {len(rs)} | " + " ".join(f"{c:>13s}" for c in cells) +
          f" | {max(x['peak_rss_gib'] for x in rs):.2f} {st.mean(x['recall_mean'] for x in rs):.6f} {'same' if dg == {base[0]['digest']} else sorted(dg)}")
print("spread of control (max-min over rounds): " + ", ".join(f"{f[:-2]} {m(base, f)[1]:.1f}" for f in fields + ph))
