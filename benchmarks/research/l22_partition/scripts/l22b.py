"""L22 non-search kernels, items 1-3 of STATE.md 00:10Z (numpy, wl-03 cpus 0-7, timing lock).

Kernels run on W Python worker threads with BLAS pinned to one thread each (an 8-core proxy for a
fused Rust kernel; numpy matmul, ufuncs and argpartition release the GIL).
  exact  : nearest + SOAR spill from ONE matmul. Top-K of d = |c|^2 - 2 x.c; SOAR loss of the K-1
           non-nearest candidates from x.c and G = C.C^T in float64; certificate: SOAR loss >= L2, so
           a row whose best restricted loss <= |x|^2 + d_(K+1) has the full-SOAR answer; other rows
           fall back to l22a3.soar_spill.
  hier   : S super-centres (k-means of C); each row scores the members of its m nearest supers
           (one GEMM per super over the rows that probe it), then the same SOAR step. Approximate.
  bf     : exact brute-force top-10 per query block with a running-threshold filter.
Phases: train C22/C24/C32 (fresh seed-7 rng each, C32 = A3's centres) -> timing -> paired recall on
the A3 stream (fixed centres; exact vs hier; train op 22/24 vs 32) -> paired recall on A2 1M/3M.
usage: l22b.py --out DIR | --synthetic
"""
import argparse, importlib.util, json, os, sys, time
from concurrent.futures import ThreadPoolExecutor

import numpy as np
from threadpoolctl import threadpool_limits

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("a3", os.path.join(HERE, "l22a3.py"))
a3 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a3)
D2 = "/home/davidcondrey/bigann/s1c/bab/data/MSTuring-30M-clustered"
W = 8
LAM = 1.0
log = a3.log


def par_rows(fn, n, ch, nw=W):
    """fn(s, e) over row ranges, nw workers, BLAS single-threaded."""
    rs = [(s, min(s + ch, n)) for s in range(0, n, ch)]
    with threadpool_limits(1, user_api="blas"), ThreadPoolExecutor(nw) as ex:
        list(ex.map(lambda r: fn(*r), rs))


class Cent:
    def __init__(self, C):
        self.C = np.ascontiguousarray(C, np.float32)
        self.c2 = (self.C * self.C).sum(1)
        C64 = self.C.astype(np.float64)
        self.c2_64 = (C64 * C64).sum(1)
        self.G = C64 @ C64.T
        self.hier = {}

    def add_hier(self, S, rng):
        Sc = a3.kmeans(self.C, S, 10, rng)
        lab = a3.nearest(self.C, Sc)
        groups = [np.nonzero(lab == s)[0].astype(np.int64) for s in range(S)]
        self.hier[S] = (np.ascontiguousarray(Sc), (Sc * Sc).sum(1), groups)
        return [len(g) for g in groups]


def soar_on_cands(x, ce, cand, dc, xcc):
    """cand (b, K) centre ids (-1 = none), dc = |c|^2 - 2x.c (f32), xcc = x.c (f32).
    Returns a1, a2, best restricted SOAR loss, |x|^2 (float64)."""
    b = len(x)
    ar = np.arange(b)
    dc = np.where(cand < 0, np.inf, dc)
    j1 = dc.argmin(1)
    a1 = cand[ar, j1]
    x64 = x.astype(np.float64)
    x2 = (x64 * x64).sum(1)
    xcc = xcc.astype(np.float64)
    xc1 = xcc[ar, j1]
    cs = np.maximum(cand, 0)
    rr = np.maximum(x2 - 2.0 * xc1 + ce.c2_64[a1], 1e-20)
    dist = x2[:, None] - 2.0 * xcc + ce.c2_64[cs]
    proj = (x2 - xc1)[:, None] - (xcc - ce.G[a1[:, None], cs])
    loss = dist + LAM * proj * proj / rr[:, None]
    loss[cand < 0] = np.inf
    loss[ar, j1] = np.inf
    j2 = loss.argmin(1)
    return a1.astype(np.int32), cand[ar, j2].astype(np.int32), loss[ar, j2], x2


def assign_exact(X, ce, K, ch=4096, nw=W, stats=None):
    n = len(X)
    a1 = np.empty(n, np.int32)
    a2 = np.empty(n, np.int32)
    fb = np.zeros(n, bool)

    def blk(s, e):
        x = X[s:e]
        xc = x @ ce.C.T
        d = ce.c2[None, :] - 2.0 * xc
        part = np.argpartition(d, K, axis=1)
        cand = part[:, :K]
        dk1 = np.take_along_axis(d, part[:, K:K + 1], 1)[:, 0].astype(np.float64)
        b1, b2, best, x2 = soar_on_cands(x, ce, cand, np.take_along_axis(d, cand, 1), np.take_along_axis(xc, cand, 1))
        bad = ~(best <= x2 + dk1)
        if bad.any():
            b2[bad] = a3.soar_spill(x[bad], ce.C, b1[bad], LAM)
        a1[s:e], a2[s:e], fb[s:e] = b1, b2, bad

    par_rows(blk, n, ch, nw)
    if stats is not None:
        stats["fallback_rate"] = float(fb.mean())
    return a1, a2


def assign_hier(X, ce, S, m, K, ch=32768, nw=W):
    Sc, s2, groups = ce.hier[S]
    n = len(X)
    a1 = np.empty(n, np.int32)
    a2 = np.empty(n, np.int32)

    def blk(s, e):
        x = X[s:e]
        b = len(x)
        sp = np.argpartition(s2[None, :] - 2.0 * (x @ Sc.T), m - 1, axis=1)[:, :m].ravel()
        prow = np.repeat(np.arange(b), m)
        order = np.argsort(sp, kind="stable")
        bounds = np.searchsorted(sp[order], np.arange(S + 1))
        cid = np.full((b * m, K), -1, np.int64)
        cd = np.full((b * m, K), np.inf, np.float32)
        cxc = np.zeros((b * m, K), np.float32)
        for g in range(S):
            lo, hi = bounds[g], bounds[g + 1]
            M = groups[g]
            if hi == lo or len(M) == 0:
                continue
            idx = order[lo:hi]
            xcs = x[prow[idx]] @ ce.C[M].T
            ds = ce.c2[M][None, :] - 2.0 * xcs
            kk = min(K, len(M))
            p = np.argpartition(ds, kk - 1, axis=1)[:, :kk] if len(M) > kk else np.broadcast_to(np.arange(kk), (len(idx), kk))
            cid[idx, :kk] = M[p]
            cd[idx, :kk] = np.take_along_axis(ds, p, 1)
            cxc[idx, :kk] = np.take_along_axis(xcs, p, 1)
        cid, cd, cxc = cid.reshape(b, m * K), cd.reshape(b, m * K), cxc.reshape(b, m * K)
        q = np.argpartition(cd, K - 1, axis=1)[:, :K]
        b1, b2, _, _ = soar_on_cands(x, ce, np.take_along_axis(cid, q, 1), np.take_along_axis(cd, q, 1), np.take_along_axis(cxc, q, 1))
        a1[s:e], a2[s:e] = b1, b2

    par_rows(blk, n, ch, nw)
    return a1, a2


def bruteforce(Q, X, ids, k=10, qb=1250, ch=16384, nw=W):
    """Exact top-k ids per query (L2), query blocks over workers, running-threshold filter."""
    x2 = (X * X).sum(1)
    out = np.empty((len(Q), k), np.int64)

    def blk(qs, qe):
        Qb = Q[qs:qe]
        nb = len(Qb)
        bd = np.full((nb, k), np.inf, np.float32)
        bi = np.full((nb, k), -1, np.int64)
        for s in range(0, len(X), ch):
            e = min(s + ch, len(X))
            Dm = x2[None, s:e] - 2.0 * (Qb @ X[s:e].T)
            mask = Dm < bd.max(1)[:, None]
            qi, ci = np.nonzero(mask)
            if len(qi) == 0:
                continue
            if len(qi) > nb * (e - s) // 8:
                cd = np.concatenate([bd, Dm], 1)
                cidx = np.concatenate([bi, np.broadcast_to(ids[None, s:e], Dm.shape)], 1)
                sel = np.argpartition(cd, k - 1, axis=1)[:, :k]
                bd, bi = np.take_along_axis(cd, sel, 1), np.take_along_axis(cidx, sel, 1)
                continue
            cnt = np.bincount(qi, minlength=nb)
            hq = np.nonzero(cnt)[0]
            inv = np.empty(nb, np.int64)
            inv[hq] = np.arange(len(hq))
            starts = np.concatenate([[0], np.cumsum(cnt)[:-1]])
            pos = np.arange(len(qi)) - starts[qi]
            mc = int(cnt.max())
            pv = np.full((len(hq), mc), np.inf, np.float32)
            pi = np.full((len(hq), mc), -1, np.int64)
            pv[inv[qi], pos] = Dm[qi, ci]
            pi[inv[qi], pos] = ids[s + ci]
            cd = np.concatenate([bd[hq], pv], 1)
            cidx = np.concatenate([bi[hq], pi], 1)
            sel = np.argpartition(cd, k - 1, axis=1)[:, :k]
            bd[hq], bi[hq] = np.take_along_axis(cd, sel, 1), np.take_along_axis(cidx, sel, 1)
        out[qs:qe] = bi

    par_rows(blk, len(Q), qb, nw)
    return out


def tie_check(X, ce, a1r, a2r, a1, a2):
    """Rows where (a1, a2) differs from the reference: count, and how many are loss ties (rel 1e-5)."""
    dif = np.nonzero((a1 != a1r) | (a2 != a2r))[0]
    if len(dif) == 0:
        return {"differ": 0, "non_tie": 0}
    x = X[dif].astype(np.float64)
    C = ce.C.astype(np.float64)

    def loss(p, c):
        r = x - C[p]
        rr = np.maximum((r * r).sum(1), 1e-20)
        xc = x - C[c]
        return (xc * xc).sum(1) + LAM * (xc * r).sum(1) ** 2 / rr

    d1 = ((x - C[a1[dif]]) ** 2).sum(1) - ((x - C[a1r[dif]]) ** 2).sum(1)
    n1 = np.abs(d1) > 1e-5 * ((x - C[a1r[dif]]) ** 2).sum(1)
    l, lr = loss(a1[dif], a2[dif]), loss(a1r[dif], a2r[dif])
    n2 = np.abs(l - lr) > 1e-5 * lr
    return {"differ": int(len(dif)), "non_tie": int((n1 | n2).sum()), "worse_loss": int(((l - lr) > 1e-5 * lr).sum())}


def ceil_from(rank, l1, l2, counts, tiers):
    cum = np.cumsum(counts[np.argsort(rank, axis=1)], axis=1)
    out = {}
    for t in tiers:
        hit = (np.take_along_axis(rank, l1, 1) < t) | (np.take_along_axis(rank, l2, 1) < t)
        out[t] = {"recall": float(hit.mean()), "codes_scanned": float(cum[:, t - 1].mean())}
    out["lists_over_4x_mean"] = int((counts > 4 * counts.mean()).sum())
    return out


def qrank(Q, C):
    return np.argsort(np.argsort((C * C).sum(1)[None, :] - 2.0 * (Q @ C.T), axis=1, kind="stable"), axis=1)


def live_at(ops, n_tot, stop_op):
    live = np.zeros(n_tot, bool)
    for o in ops:
        op = o.get("operation")
        if op == "insert":
            live[int(o["start"]):int(o["end"])] = True
        elif op == "delete":
            live[int(o["start"]):int(o["end"])] = False
        elif op == "search" and o["i"] == stop_op:
            return live
    raise SystemExit(f"op {stop_op} not found")


def timed(fn):
    t = time.time()
    r = fn()
    return r, time.time() - t


def run_timing(mm, ce, ops, n_tot, res):
    T = res.setdefault("timing", {})
    Xi = np.ascontiguousarray(mm[15_000_000:16_000_000])
    # GEMM alone: BLAS 8 threads chunk 20000 (reference layout) and 8 workers x BLAS 1 chunk 4096
    def g_ref():
        for s in range(0, len(Xi), 20000):
            Xi[s:s + 20000] @ ce.C.T
    _, T["gemm_1M_blas8_s"] = timed(g_ref)
    _, T["gemm_1M_w8_s"] = timed(lambda: par_rows(lambda s, e: Xi[s:e] @ ce.C.T, len(Xi), 4096))
    # reference assignment (l22a3 functions, BLAS 8 threads), the agreement baseline
    (a1r, a2r), T["ref_nearest_spill_1M_s"] = timed(lambda: (lambda a: (a, a3.soar_spill(Xi, ce.C, a, LAM)))(a3.nearest(Xi, ce.C)))
    log(f"timing gemm blas8 {T['gemm_1M_blas8_s']:.1f}s w8 {T['gemm_1M_w8_s']:.1f}s ref {T['ref_nearest_spill_1M_s']:.1f}s")
    # worker scaling on 131072 rows, exact K=16
    Xs = Xi[:131072]
    _, t1 = timed(lambda: assign_exact(Xs, ce, 16, nw=1))
    _, t8 = timed(lambda: assign_exact(Xs, ce, 16, nw=8))
    T["exact16_131k_w1_s"], T["exact16_131k_w8_s"], T["exact16_speedup_w8"] = t1, t8, t1 / t8
    for K in (8, 16, 32, 64):
        st = {}
        (a1, a2), t = timed(lambda: assign_exact(Xi, ce, K, stats=st))
        T[f"exact_K{K}_1M_s"] = t
        T[f"exact_K{K}_fallback_rate"] = st["fallback_rate"]
        T[f"exact_K{K}_agree"] = tie_check(Xi, ce, a1r, a2r, a1, a2)
        print(f"RESULT timing exact K={K} 1M={t:.2f}s fallback={st['fallback_rate']:.5f} agree={T[f'exact_K{K}_agree']}", flush=True)
    for S, m in ((71, 4), (71, 8)):
        (a1, a2), t = timed(lambda: assign_hier(Xi, ce, S, m, 16))
        T[f"hier_S{S}_m{m}_1M_s"] = t
        T[f"hier_S{S}_m{m}_nearest_agree"] = float((a1 == a1r).mean())
        T[f"hier_S{S}_m{m}_both_agree"] = float(((a1 == a1r) & (a2 == a2r)).mean())
        print(f"RESULT timing hier S={S} m={m} K=16 1M={t:.2f}s nearest_agree={T[f'hier_S{S}_m{m}_nearest_agree']:.5f} both_agree={T[f'hier_S{S}_m{m}_both_agree']:.5f}", flush=True)
    del Xi
    # brute force at op-32 live, all 10k queries, vs official GT
    live = live_at(ops, n_tot, 32)
    ids = np.nonzero(live)[0]
    X = np.ascontiguousarray(mm[ids])
    Qf = np.fromfile(f"{a3.D}/testQuery10K.fbin", dtype=np.float32, offset=8).reshape(-1, a3.DIM)
    bi, t = timed(lambda: bruteforce(Qf, X, ids))
    gt = a3.read_gt(f"{a3.GTD}/step32.gt100")
    rec = float(np.mean([len(set(bi[i]) & set(gt[i])) / 10 for i in range(len(gt))]))
    T["bf_step_op32_s"], T["bf_step_op32_recall"] = t, rec
    print(f"RESULT timing bruteforce op32 live={len(ids)} q={len(Qf)} {t:.1f}s recall_vs_gt={rec:.5f}", flush=True)


def run_stream(mm, cents, ops, n_tot, eval_ops, tiers, sample, res):
    """Paired ceilings on the A3 stream: per op the same GT rows and live sample for every arm."""
    Q = np.load(f"{a3.PISO}/queries1000.npy")
    arms = [("c32_exact", 32, "exact"), ("c32_hier_m4", 32, ("hier", 71, 4)), ("c32_hier_m8", 32, ("hier", 71, 8)),
            ("c22_exact", 22, "exact"), ("c24_exact", 24, "exact")]
    ranks = {op: qrank(Q, cents[op].C) for op in cents}
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
            t = time.time()
            ids = np.nonzero(live)[0]
            gt = a3.read_gt(f"{a3.GTD}/step{i}.gt100")[:len(Q)]
            assert live[gt.ravel()].all()
            g = np.unique(gt.ravel())
            pos = np.searchsorted(g, gt)
            s = np.sort(np.random.default_rng(1000 + i).choice(ids, min(sample, len(ids)), replace=False))
            Xg, Xs = np.asarray(mm[g]), np.asarray(mm[s])
            row = {"op": i, "live": int(len(ids))}
            for name, top, mode in arms:
                ce = cents[top]
                f = (lambda Z: assign_exact(Z, ce, 32)) if mode == "exact" else (lambda Z: assign_hier(Z, ce, mode[1], mode[2], 16))
                a1g, a2g = f(Xg)
                a1s, a2s = f(Xs)
                counts = (np.bincount(a1s, minlength=len(ce.C)) + np.bincount(a2s, minlength=len(ce.C))) * (len(ids) / len(s))
                e = ceil_from(ranks[top], a1g[pos], a2g[pos], counts, tiers)
                row[name] = e
                print(f"RESULT stream op={i} live={len(ids)} arm={name} " + " ".join(
                    f"np{t_}={e[t_]['recall']:.4f}/{e[t_]['codes_scanned'] / len(ids):.3f}" for t_ in tiers)
                    + f" lists_over_4x={e['lists_over_4x_mean']}", flush=True)
            out.append(row)
            log(f"stream op {i} {time.time() - t:.0f}s")


def run_prefix(n, tiers, res):
    mm = np.memmap(f"{D2}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8,
                   shape=(int(np.fromfile(f"{D2}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0]), a3.DIM))
    X = np.ascontiguousarray(mm[:n])
    Q = np.load(f"{a3.PISO}/queries1000.npy")
    gt = np.load(f"{a3.PISO}/gt_{n}.npy")[:len(Q)]
    rng = np.random.default_rng(7)
    ce = Cent(a3.kmeans(X[rng.choice(n, 262144, replace=False)], 5000, 10, rng))
    ce.add_hier(71, np.random.default_rng(71))
    rank = qrank(Q, ce.C)
    r = res.setdefault("prefix", {}).setdefault(str(n), {})
    for name, f in (("exact", lambda: assign_exact(X, ce, 32)), ("hier_m4", lambda: assign_hier(X, ce, 71, 4, 16)),
                    ("hier_m8", lambda: assign_hier(X, ce, 71, 8, 16))):
        a1, a2 = f()
        e = ceil_from(rank, a1[gt], a2[gt], np.bincount(a1, minlength=5000) + np.bincount(a2, minlength=5000), tiers)
        r[name] = e
        print(f"RESULT prefix n={n} arm={name} " + " ".join(f"np{t}={e[t]['recall']:.4f}/{e[t]['codes_scanned']:.0f}" for t in tiers), flush=True)


def synthetic():
    global W
    rng = np.random.default_rng(3)
    n, k, dim = 30000, 60, 16
    X = (rng.standard_normal((n, dim)) + 3 * rng.standard_normal((k, dim))[rng.integers(k, size=n)]).astype(np.float32)
    ce = Cent(a3.kmeans(X[:5000], k, 5, rng))
    S = 6
    ce.add_hier(S, rng)
    a1r = a3.nearest(X, ce.C)
    a2r = a3.soar_spill(X, ce.C, a1r, LAM)
    ok = True
    for K in (4, 8, 16):
        st = {}
        a1, a2 = assign_exact(X, ce, K, ch=1000, stats=st)
        tc = tie_check(X, ce, a1r, a2r, a1, a2)
        print("exact K", K, "fallback", st["fallback_rate"], tc)
        ok &= tc["non_tie"] == 0
    a1, a2 = assign_hier(X, ce, S, S, k, ch=4000)
    tc = tie_check(X, ce, a1r, a2r, a1, a2)
    print("hier m=S", tc)
    ok &= tc["non_tie"] == 0
    a1, a2 = assign_hier(X, ce, S, 2, 8, ch=4000)
    print("hier m=2 nearest agree", float((a1 == a1r).mean()))
    Q = X[rng.choice(n, 300, replace=False)] + 0.05 * rng.standard_normal((300, dim)).astype(np.float32)
    ids = np.arange(n, dtype=np.int64) * 3 + 1
    bi = bruteforce(Q, X, ids, qb=37, ch=1000)
    d = ((Q.astype(np.float64)[:, None, :] - X.astype(np.float64)[None]) ** 2).sum(2)
    ex = ids[np.argsort(d, axis=1)[:, :10]]
    rec = np.mean([len(set(bi[i]) & set(ex[i])) / 10 for i in range(len(Q))])
    print("bruteforce recall", rec)
    ok &= rec >= 0.999
    print("SELFCHECK", "PASS" if ok else "FAIL", flush=True)
    sys.exit(0 if ok else 1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(a3.PISO, "l22a", "out6"))
    ap.add_argument("--synthetic", action="store_true")
    ap.add_argument("--eval-ops", type=int, nargs="+", default=[32, 100, 200, 400, 640, 900, 1120, 1280])
    ap.add_argument("--tiers", type=int, nargs="+", default=[350, 500, 700])
    ap.add_argument("--sample", type=int, default=500_000)
    ap.add_argument("--phases", nargs="+", default=["timing", "stream", "prefix"])
    args = ap.parse_args()
    if args.synthetic:
        synthetic()
    res = {"args": vars(args)}
    os.makedirs(args.out, exist_ok=True)
    n_tot = int(np.fromfile(f"{a3.D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
    mm = np.memmap(f"{a3.D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, a3.DIM))
    ops = a3.parse_runbook(a3.RUNBOOK)
    cents = {}
    for top in (32, 22, 24):
        ids = np.nonzero(live_at(ops, n_tot, top))[0]
        rng = np.random.default_rng(7)
        (C), t = timed(lambda: a3.kmeans(np.asarray(mm[np.sort(rng.choice(ids, min(262144, len(ids)), replace=False))]), 5000, 10, rng))
        cents[top] = Cent(C)
        np.save(os.path.join(args.out, f"C{top}.npy"), C)
        log(f"trained C{top} live {len(ids)} in {t:.0f}s")
    sizes = cents[32].add_hier(71, np.random.default_rng(71))
    res["hier_group_sizes_c32"] = {"min": min(sizes), "max": max(sizes), "mean": float(np.mean(sizes))}
    print(f"RESULT hier S=71 group sizes min={min(sizes)} max={max(sizes)} mean={np.mean(sizes):.1f}", flush=True)

    def dump():
        with open(os.path.join(args.out, "l22b.json"), "w") as f:
            json.dump(res, f, indent=1, default=str)

    if "timing" in args.phases:
        run_timing(mm, cents[32], ops, n_tot, res)
        dump()
    if "stream" in args.phases:
        run_stream(mm, cents, ops, n_tot, set(args.eval_ops), args.tiers, args.sample, res)
        dump()
    if "prefix" in args.phases:
        for n in (1_000_000, 3_000_000):
            run_prefix(n, args.tiers, res)
            dump()
    log("done")


if __name__ == "__main__":
    main()
