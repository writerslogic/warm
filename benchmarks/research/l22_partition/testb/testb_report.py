"""L22 test B report (STATE.md 2026-11-08 registration): validity, gates and the time split of a
WARM_MODE=partition final_runbook run against the stage-0 pin run.
usage: testb_report.py RUNBOOK PIN_OUT S2_OUT"""
import ast, collections, re, sys


def lines(path):
    with open(path) as f:
        return f.read().splitlines()


def ops(path, name="msturing-30M-clustered"):
    out, cur, inside = {}, None, False
    for l in lines(path):
        if re.match(r"^\S", l):
            inside = l.startswith(name + ":")
            continue
        if not inside:
            continue
        m = re.match(r"\s+(\d+):", l)
        if m:
            cur = int(m[1])
            continue
        m = re.match(r"\s+operation:\s*['\"]?(\w+)", l)
        if m and cur is not None:
            out[cur] = m[1]
    return out


def steps(p):
    return {int(m[1]): float(m[2]) for l in lines(p) for m in [re.match(r"Step (\d+) took ([\d.e-]+)s", l)] if m}


def recall(p):
    for l in lines(p):
        if l.startswith("Recall:"):
            return ast.literal_eval(l.split(":", 1)[1].strip())


runbook, pin, s2 = sys.argv[1:4]
op = ops(runbook)
st = steps(s2 + "/harness-stdout.log")
r = recall(s2 + "/data_export.log") or []
pr = recall(pin + "/data_export.log") or []
rc = "\n".join(lines(s2 + "/rc.txt")).strip()
line = [l for l in lines(s2 + "/harness-stdout.log") if l.startswith("WARM_MODE partition")]
kv = dict(t.split("=", 1) for t in line[0].split()[2:]) if line else {}
rss = [int(l.split(":")[1]) for l in lines(s2 + "/time.txt") if "Maximum resident" in l]
el = [l.split("):")[1].strip() for l in lines(s2 + "/time.txt") if "Elapsed (wall" in l]
g = collections.defaultdict(float)
for k, v in st.items():
    g[op[k]] += v
wall = sum(st.values())
valid = len(st) == 1280 and rc == "run_exit=0" and bool(line) and len(r) == 640
tr = int(kv.get("trained_at_search", "0") or 0) if kv.get("trained_at_search") not in (None, "None") else 0
post = r[tr - 1:] if tr else []
mean = sum(r) / len(r) if r else float("nan")
lo = min(post) if post else float("nan")
print(f"valid {valid} steps {len(st)} {rc} searches {len(r)} warm_line {bool(line)}")
print(f"wall(sum of steps) {wall:.0f} s; process elapsed {el[0] if el else '?'}; per op {dict((k, round(v)) for k, v in g.items())}")
print(f"mean recall {mean:.6f} (pin {sum(pr) / len(pr):.6f}); trained at search {tr} live {kv.get('trained_at_live')}; "
      f"min post-train {lo:.4f}; pre-train mean {sum(r[:tr - 1]) / max(tr - 1, 1):.4f}")
bins = [sum(r[i:i + 80]) / 80 for i in range(0, 640, 80)] if len(r) == 640 else []
print("recall by 80-search bins", [round(b, 4) for b in bins])
print("peak RSS %.2f GiB (6 GB projection, 8 GiB scope)" % (rss[0] / 2**20 if rss else float("nan")))
print("WARM_MODE terms", {k: kv[k] for k in kv})
gates = {"mean>=0.90": mean >= 0.90, "post-train min>=0.80": lo >= 0.80, "wall<3000": wall < 3000,
         "rss<8GiB": bool(rss) and rss[0] < 8 * 2**20}
print("GATES", gates)
print("TEST B:", "PASS" if valid and all(gates.values()) else ("INVALID" if not valid else "FAIL"))
