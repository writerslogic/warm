"""L22 item 1 follow-up (STATE.md 00:40Z): SOAR spill restricted to the K nearest centres, NO fallback.

l22b's certificate failed on 97-99.5% of rows, so the l22b "exact" kernel is the full spill. Here the
spill is chosen only among the K-1 non-nearest of the K nearest centres (approximate; nearest exact).
Phases: timing on 1M insert rows (K 16/32/64/128; agreement with the l22a3 reference) -> paired
ceilings on the A3 stream (arms: full = l22b exact K32, restricted K) for the given centre files ->
paired ceilings on A2 1M/3M prefixes.
usage: l22b2.py --out DIR --cents C32.npy [C22.npy ...] | --synthetic
"""
import argparse, importlib.util, json, os, sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
s_ = importlib.util.spec_from_file_location("l22b", os.path.join(HERE, "l22b.py"))
b = importlib.util.module_from_spec(s_)
s_.loader.exec_module(b)
a3 = b.a3
KS = (16, 32, 64, 128)


def assign_restricted(X, ce, K, ch=4096, nw=b.W):
    n = len(X)
    a1 = np.empty(n, np.int32)
    a2 = np.empty(n, np.int32)

    def blk(s, e):
        x = X[s:e]
        xc = x @ ce.C.T
        d = ce.c2[None, :] - 2.0 * xc
        cand = np.argpartition(d, K - 1, axis=1)[:, :K]
        a1[s:e], a2[s:e], _, _ = b.soar_on_cands(x, ce, cand, np.take_along_axis(d, cand, 1), np.take_along_axis(xc, cand, 1))

    b.par_rows(blk, n, ch, nw)
    return a1, a2


def arms():
    return [("full", lambda Z, ce: b.assign_exact(Z, ce, 32))] + [(f"r{K}", (lambda K: lambda Z, ce: assign_restricted(Z, ce, K))(K)) for K in KS]


def run_timing(mm, ce, res):
    T = res.setdefault("timing", {})
    Xi = np.ascontiguousarray(mm[15_000_000:16_000_000])
    (a1r, a2r), T["ref_s"] = b.timed(lambda: (lambda a: (a, a3.soar_spill(Xi, ce.C, a, b.LAM)))(a3.nearest(Xi, ce.C)))
    _, T["gemm_w8_s"] = b.timed(lambda: b.par_rows(lambda s, e: Xi[s:e] @ ce.C.T, len(Xi), 4096))
    for K in KS:
        (a1, a2), t = b.timed(lambda: assign_restricted(Xi, ce, K))
        tc = b.tie_check(Xi, ce, a1r, a2r, a1, a2)
        T[f"r{K}"] = {"s_per_1M": t, "nearest_agree": float((a1 == a1r).mean()), "spill_agree": float((a2 == a2r).mean()), **tc}
        print(f"RESULT timing restricted K={K} 1M={t:.2f}s nearest_agree={T[f'r{K}']['nearest_agree']:.5f} "
              f"spill_agree={T[f'r{K}']['spill_agree']:.5f} non_tie={tc['non_tie']}", flush=True)
    print(f"RESULT timing ref={T['ref_s']:.1f}s gemm_w8={T['gemm_w8_s']:.2f}s", flush=True)


def run_stream(mm, cents, ops, n_tot, eval_ops, tiers, sample, res):
    Q = np.load(f"{a3.PISO}/queries1000.npy")
    ranks = {k: b.qrank(Q, ce.C) for k, ce in cents.items()}
    live = np.zeros(n_tot, bool)
    out = res.setdefault("stream", [])
    for o in ops:
        op = o.get("operation")
        if op == "insert":
            live[int(o["start"]):int(o["end"])] = True
        elif op == "delete":
            live[int(o["start"]):int(o["end"])] = False
        elif op == "search" and o["i"] in eval_ops:
            i = o["i"]
            ids = np.nonzero(live)[0]
            gt = a3.read_gt(f"{a3.GTD}/step{i}.gt100")[:len(Q)]
            g = np.unique(gt.ravel())
            pos = np.searchsorted(g, gt)
            s = np.sort(np.random.default_rng(1000 + i).choice(ids, min(sample, len(ids)), replace=False))
            Xg, Xs = np.asarray(mm[g]), np.asarray(mm[s])
            row = {"op": i, "live": int(len(ids))}
            for cname, ce in cents.items():
                for aname, f in arms():
                    a1g, a2g = f(Xg, ce)
                    a1s, a2s = f(Xs, ce)
                    counts = (np.bincount(a1s, minlength=len(ce.C)) + np.bincount(a2s, minlength=len(ce.C))) * (len(ids) / len(s))
                    e = b.ceil_from(ranks[cname], a1g[pos], a2g[pos], counts, tiers)
                    row[f"{cname}_{aname}"] = e
                    print(f"RESULT stream op={i} arm={cname}_{aname} " + " ".join(
                        f"np{t}={e[t]['recall']:.4f}/{e[t]['codes_scanned'] / len(ids):.3f}" for t in tiers), flush=True)
            out.append(row)


def run_prefix(n, tiers, res):
    mm = np.memmap(f"{b.D2}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8,
                   shape=(int(np.fromfile(f"{b.D2}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0]), a3.DIM))
    X = np.ascontiguousarray(mm[:n])
    Q = np.load(f"{a3.PISO}/queries1000.npy")
    gt = np.load(f"{a3.PISO}/gt_{n}.npy")[:len(Q)]
    rng = np.random.default_rng(7)
    ce = b.Cent(a3.kmeans(X[rng.choice(n, 262144, replace=False)], 5000, 10, rng))
    rank = b.qrank(Q, ce.C)
    r = res.setdefault("prefix", {}).setdefault(str(n), {})
    for name, f in arms():
        a1, a2 = f(X, ce)
        e = b.ceil_from(rank, a1[gt], a2[gt], np.bincount(a1, minlength=5000) + np.bincount(a2, minlength=5000), tiers)
        r[name] = e
        print(f"RESULT prefix n={n} arm={name} " + " ".join(f"np{t}={e[t]['recall']:.4f}/{e[t]['codes_scanned']:.0f}" for t in tiers), flush=True)


def synthetic():
    rng = np.random.default_rng(3)
    n, k, dim = 30000, 60, 16
    X = (rng.standard_normal((n, dim)) + 3 * rng.standard_normal((k, dim))[rng.integers(k, size=n)]).astype(np.float32)
    ce = b.Cent(a3.kmeans(X[:5000], k, 5, rng))
    a1r = a3.nearest(X, ce.C)
    a2r = a3.soar_spill(X, ce.C, a1r, b.LAM)
    a1, a2 = assign_restricted(X, ce, k, ch=1000)
    tc = b.tie_check(X, ce, a1r, a2r, a1, a2)
    a1s, a2s = assign_restricted(X, ce, 8, ch=1000)
    print("K=k", tc, "K=8 nearest", float((a1s == a1r).mean()), "spill", float((a2s == a2r).mean()))
    ok = tc["non_tie"] == 0 and (a1s == a1r).all()
    print("SELFCHECK", "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(a3.PISO, "l22a", "out8"))
    ap.add_argument("--cents", nargs="+", default=[])
    ap.add_argument("--synthetic", action="store_true")
    ap.add_argument("--eval-ops", type=int, nargs="+", default=[32, 100, 200, 400, 640, 900, 1120, 1280])
    ap.add_argument("--tiers", type=int, nargs="+", default=[350, 500, 700])
    ap.add_argument("--sample", type=int, default=500_000)
    ap.add_argument("--phases", nargs="+", default=["timing", "stream", "prefix"])
    args = ap.parse_args()
    if args.synthetic:
        synthetic()
    os.makedirs(args.out, exist_ok=True)
    res = {"args": vars(args)}
    n_tot = int(np.fromfile(f"{a3.D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
    mm = np.memmap(f"{a3.D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, a3.DIM))
    cents = {os.path.basename(p)[:-4]: b.Cent(np.load(p)) for p in args.cents}

    def dump():
        with open(os.path.join(args.out, "l22b2.json"), "w") as f:
            json.dump(res, f, indent=1, default=str)

    if "timing" in args.phases:
        run_timing(mm, cents["C32"], res)
        dump()
    if "stream" in args.phases:
        run_stream(mm, cents, a3.parse_runbook(a3.RUNBOOK), n_tot, set(args.eval_ops), args.tiers, args.sample, res)
        dump()
    if "prefix" in args.phases:
        for n in (1_000_000, 3_000_000):
            run_prefix(n, args.tiers, res)
            dump()
    a3.log("done")


if __name__ == "__main__":
    main()
