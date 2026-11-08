"""L22 total-time projection on wl-03 (STATE.md 23:40Z rule: search + non-search + fixed <= 3,000 s).

search  = sum over searches at/after train_op of [table_s(P) + live * f_op(P) * Q / scan_rate(P)]
          f_op from --f-json (l22b stream / l22c maint eval rows, linear in op between eval ops,
          clamped at the ends) or a constant --f.
nonsrch = train k-means + assignment of the rows live at train_op + assignment of rows inserted after
          train_op + brute-force steps before train_op (scaled by live) + deletes + maintenance.
usage: project_total.py --runbook R --bench J [J ...] --nprobe P --train-op T (--f F | --f-json J --arm A)
       --kmeans-s S --assign-per-1m-s S --bf-step-s S --bf-ref-live N [--deletes-s 30] [--maint-s 0] [--fixed-s 150]
"""
import argparse, json, re

Q = 10_000
LIMIT = 3000


def parse(path):
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
    return ops


def f_curve(path, arm, P):
    with open(path) as fh:
        d = json.load(fh)
    rows = d.get("stream") or d.get("eval")
    pts = sorted((r["op"], r[arm][str(P)]["codes_scanned"] / r["live"]) for r in rows if arm in r)

    def f(op):
        if op <= pts[0][0]:
            return pts[0][1]
        for (o0, f0), (o1, f1) in zip(pts, pts[1:]):
            if op <= o1:
                return f0 + (f1 - f0) * (op - o0) / (o1 - o0)
        return pts[-1][1]
    return f, pts


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runbook", required=True)
    ap.add_argument("--bench", nargs="+", required=True)
    ap.add_argument("--nprobe", type=int, required=True)
    ap.add_argument("--train-op", type=int, required=True)
    ap.add_argument("--f", type=float)
    ap.add_argument("--f-json")
    ap.add_argument("--arm")
    ap.add_argument("--kmeans-s", type=float, required=True)
    ap.add_argument("--assign-per-1m-s", type=float, required=True)
    ap.add_argument("--bf-step-s", type=float, required=True)
    ap.add_argument("--bf-ref-live", type=float, required=True)
    ap.add_argument("--deletes-s", type=float, default=30.0)
    ap.add_argument("--maint-s", type=float, default=0.0)
    ap.add_argument("--fixed-s", type=float, default=150.0)
    ap.add_argument("--centre-extra-s", type=float, default=0.0, help="extra centre scoring when lists > 5,000")
    a = ap.parse_args()
    # Prefer a sustained run (most repetitions) and its MEAN components; else the best-of-3 min.
    best = None
    for p in a.bench:
        with open(p) as fh:
            b = json.load(fh)
        if b["nprobe"] == a.nprobe and (best is None or b.get("reps", 3) > best.get("reps", 3)
                                        or (b.get("reps", 3) == best.get("reps", 3) and b["step_s"] < best["step_s"])):
            best = b
    if "scan_admit_stats" in best:
        scan_s, table, basis = best["scan_admit_stats"]["mean"], best["lut_build_stats"]["mean"], f"mean of {best['reps']} reps"
    else:
        scan_s, table, basis = best["scan_admit_s"], best["lut_build_s"], "min of 3"
    rate = best["codes_scanned"] / scan_s
    f, pts = (f_curve(a.f_json, a.arm, a.nprobe) if a.f_json else ((lambda op: a.f), None))
    live = pre_live = live_at_train = ins_after = 0
    tables = scan = 0.0
    n_after = 0
    for o in parse(a.runbook):
        op = o.get("operation")
        if op == "insert":
            n = int(o["end"]) - int(o["start"])
            live += n
            if o["i"] > a.train_op:
                ins_after += n
        elif op == "delete":
            live -= int(o["end"]) - int(o["start"])
        elif op == "search":
            if o["i"] < a.train_op:
                pre_live += live
            else:
                if o["i"] == a.train_op:
                    live_at_train = live
                n_after += 1
                tables += table
                scan += live * f(o["i"]) * Q / rate
    search = tables + scan + a.fixed_s + a.centre_extra_s
    terms = {
        "train_kmeans": a.kmeans_s,
        "train_assign": live_at_train / 1e6 * a.assign_per_1m_s,
        "insert_assign": ins_after / 1e6 * a.assign_per_1m_s,
        "bruteforce_pretrain": a.bf_step_s * pre_live / a.bf_ref_live,
        "deletes": a.deletes_s,
        "maintenance": a.maint_s,
    }
    nonsearch = sum(terms.values())
    total = search + nonsearch
    print(f"nprobe {a.nprobe} train_op {a.train_op} f={'curve ' + a.arm if a.f_json else a.f}: searches after train {n_after}, "
          f"live at train {live_at_train}, inserted after {ins_after}, pre-train live sum {pre_live}")
    if pts:
        print("  f points:", " ".join(f"{o}:{v:.3f}" for o, v in pts))
    print(f"  search: tables {tables:.0f} + scan {scan:.0f} + fixed {a.fixed_s:.0f} = {search:.0f} s (centre extra {a.centre_extra_s:.0f})  [{basis}: scan rate {rate:.3e}/s, table {table:.3f} s/step]")
    print("  non-search: " + " + ".join(f"{k} {v:.0f}" for k, v in terms.items()) + f" = {nonsearch:.0f} s")
    print(f"  TOTAL {total:.0f} s (limit {LIMIT}: {'PASS' if total <= LIMIT else 'FAIL'}, margin {LIMIT - total:+.0f} s)")


if __name__ == "__main__":
    main()
