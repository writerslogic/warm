"""Project L22 search seconds over final_runbook from per-step bench measurements (STATE.md 23:00Z gate).

search_s(P) = sum over searches after train_at of [table_s(P) + live_op * f(P) * Q / scan_rate(P)] + fixed_s
table_s(P) = the bench's LUT-build seconds per step; scan_rate(P) = codes scanned / scan+admission seconds.
f(P) = scanned fraction of live: A3 fixed-centre means (0.23 / 0.31 / 0.43) and retrained (0.14 / 0.20 / 0.28).
usage: project_time.py RUNBOOK BENCH_JSON [BENCH_JSON ...]   (best step per nprobe is used)
"""
import json, re, sys

Q = 10_000
TRAIN_OP = 32
FIXED_S = 150  # centre scoring + exact reorder over the run [I]
GATE_S = 2300
F_FIXED = {350: 0.23, 500: 0.31, 700: 0.43}
F_BAL = {350: 0.14, 500: 0.20, 700: 0.28}


def live_after_train(path):
    ops, cur = [], None
    with open(path) as fh:
        lines = fh.read().splitlines()
    for line in lines:
        s = line.strip()
        m = re.match(r"^(\d+):$", s)
        if m:
            cur = {"i": int(m.group(1))}
            ops.append(cur)
            continue
        m = re.match(r"^(\w+): '?([\w]+)'?$", s)
        if m and cur is not None:
            cur[m.group(1)] = m.group(2)
    live, out = 0, []
    for o in ops:
        op = o.get("operation")
        if op == "insert":
            live += int(o["end"]) - int(o["start"])
        elif op == "delete":
            live -= int(o["end"]) - int(o["start"])
        elif op == "search" and o["i"] >= TRAIN_OP:
            out.append(live)
    return out


def main(runbook, paths):
    live = live_after_train(runbook)
    best = {}
    for p in paths:
        with open(p) as f:
            b = json.load(f)
        k = b["nprobe"]
        if k not in best or b["step_s"] < best[k]["step_s"]:
            best[k] = b
    print(f"searches after train_at: {len(live)}, sum live {sum(live):.4e}")
    for k in sorted(best):
        b = best[k]
        rate = b["codes_scanned"] / b["scan_admit_s"]
        table = b["lut_build_s"]
        for name, f in (("fixed", F_FIXED), ("balanced", F_BAL)):
            scan = sum(live) * f[k] * Q / rate
            total = len(live) * table + scan + FIXED_S
            print(f"nprobe {k} {name:8s}: tables {len(live) * table:6.0f} s + scan {scan:6.0f} s + fixed {FIXED_S} = {total:6.0f} s"
                  f"  (gate {GATE_S}: {'PASS' if total <= GATE_S else 'FAIL'})  [scan rate {rate:.3e}/s, table {table:.3f} s/step]")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:])
