"""L22 kill test A: static partition-recall ceiling and 4-bit LUT16 recall on msturing prefixes (numpy only).

Arms per (N, training set): partition ceiling (exact L2 inside the probed lists) at nprobe 350/700/1400;
quantized recall at nprobe 700 with reorder 317 for PQ-50/16 residual codes (float LUT and INT8 LUT) and
per-dim 4-bit grid codes (INT8 LUT). Lists are k-means (10 Lloyd iterations) over a uniform sample or over the
first `--train-prefix` rows (imitates a quantizer trained early in the clustered stream).
usage: l22a.py --n N [--train-sample 262144 | --train-prefix 1000000] [--nq 1000 --nq-quant 500] [--out DIR]
       l22a.py --synthetic   (self-check on random data: full-probe ceiling and full-reorder recall must be 1.0)
Prints one `RESULT ...` line per arm; writes <out>/l22a_<n>_<train>.json.
"""
import argparse, json, os, sys, time

import numpy as np

D = "/home/davidcondrey/bigann/s1c/bab/data/MSTuring-30M-clustered"
PISO = "/home/davidcondrey/bigann/piso"
DIM = 100


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def sqdist_chunks(X, C, chunk):
    """argmin_c |x - c|^2 and the min value, rows in chunks (f32 matmul, |c|^2 added)."""
    c2 = (C * C).sum(1)
    out = np.empty(len(X), np.int32)
    best = np.empty(len(X), np.float32)
    for s in range(0, len(X), chunk):
        x = X[s:s + chunk]
        d = c2[None, :] - 2.0 * (x @ C.T)  # |x|^2 dropped: constant per row
        a = d.argmin(1)
        out[s:s + chunk] = a
        best[s:s + chunk] = d[np.arange(len(x)), a] + (x * x).sum(1)
    return out, best


def kmeans(X, k, iters, rng, chunk=20000):
    C = X[rng.choice(len(X), k, replace=False)].astype(np.float32)
    for it in range(iters):
        a, _ = sqdist_chunks(X, C, chunk)
        counts = np.bincount(a, minlength=k)
        sums = np.zeros((k, X.shape[1]), np.float64)
        np.add.at(sums, a, X)
        nz = counts > 0
        C[nz] = (sums[nz] / counts[nz, None]).astype(np.float32)
        empty = np.nonzero(~nz)[0]
        if len(empty):
            C[empty] = X[rng.choice(len(X), len(empty), replace=False)]
        log(f"  kmeans iter {it + 1}/{iters} empty={len(empty)} max_list={counts.max()}")
    return C


def recall(found, gt):
    return float(np.mean([len(set(f.tolist()) & set(g.tolist())) / gt.shape[1] for f, g in zip(found, gt)]))


def merge_top(best_d, best_i, d, i, k):
    """Row-wise: keep the k smallest of the concatenation (best_d, d). Returns new (best_d, best_i)."""
    cd = np.concatenate([best_d, d], 1)
    ci = np.concatenate([best_i, i], 1)
    if cd.shape[1] <= k:
        pad = k - cd.shape[1]
        return (np.concatenate([cd, np.full((len(cd), pad), np.inf, cd.dtype)], 1),
                np.concatenate([ci, np.full((len(ci), pad), -1, ci.dtype)], 1))
    sel = np.argpartition(cd, k - 1, axis=1)[:, :k]
    return np.take_along_axis(cd, sel, 1), np.take_along_axis(ci, sel, 1)


def pq_train(R, blocks, k, iters, rng):
    """Per-block k-means (dims 2b, 2b+1); returns codebook (blocks, k, 2)."""
    cb = np.empty((blocks, k, 2), np.float32)
    for b in range(blocks):
        cb[b] = kmeans_small(R[:, 2 * b:2 * b + 2], k, iters, rng)
    return cb


def kmeans_small(X, k, iters, rng):
    C = X[rng.choice(len(X), k, replace=False)].copy()
    for _ in range(iters):
        d = ((X[:, None, :] - C[None, :, :]) ** 2).sum(2)
        a = d.argmin(1)
        for j in range(k):
            m = a == j
            if m.any():
                C[j] = X[m].mean(0)
            else:
                C[j] = X[rng.integers(len(X))]
    return C


def pq_encode(R, cb, chunk=200000):
    n, blocks, k = len(R), cb.shape[0], cb.shape[1]
    codes = np.empty((n, blocks), np.uint8)
    for s in range(0, n, chunk):
        r = R[s:s + chunk]
        for b in range(blocks):
            d = ((r[:, None, 2 * b:2 * b + 2] - cb[b][None]) ** 2).sum(2)
            codes[s:s + chunk, b] = d.argmin(1)
    return codes


def int8_lut(lut):
    """INT8_LUT16 emulation: per query, subtract each block's minimum, one scale so every entry fits u8."""
    mn = lut.min(2, keepdims=True)
    span = (lut.max(2, keepdims=True) - mn).max(1, keepdims=True)
    scale = np.maximum(span, 1e-12) / 255.0
    return np.clip(np.rint((lut - mn) / scale), 0, 255).astype(np.float32)


def run(X, Q, gt, C, assign, args, tag, out):
    n, k = len(X), C.shape[0]
    nq = len(Q)
    res = {"tag": tag, "n": n, "lists": k, "nq": nq, "nq_quant": args.nq_quant, "reorder": args.reorder}
    t0 = time.time()
    order = np.argsort(assign, kind="stable")
    counts = np.bincount(assign, minlength=k)
    offs = np.concatenate([[0], np.cumsum(counts)])
    res["list_size"] = {"min": int(counts.min()), "median": int(np.median(counts)), "max": int(counts.max()), "empty": int((counts == 0).sum())}
    log("lists", res["list_size"])
    # query-to-center ranks
    dq = (C * C).sum(1)[None, :] - 2.0 * (Q @ C.T)
    rank = np.argsort(np.argsort(dq, axis=1, kind="stable"), axis=1).astype(np.int32)  # rank[q, l]
    tiers = [t for t in args.tiers if t <= k]
    maxt = max(tiers)
    best = {t: (np.full((nq, 10), np.inf, np.float32), np.full((nq, 10), -1, np.int64)) for t in tiers}
    q2 = (Q * Q).sum(1)
    log("ceiling pass")
    for l in range(k):
        if counts[l] == 0:
            continue
        qs = np.nonzero(rank[:, l] < maxt)[0]
        if len(qs) == 0:
            continue
        mem = order[offs[l]:offs[l + 1]]
        Xm = X[mem]
        d = q2[qs, None] - 2.0 * (Q[qs] @ Xm.T) + (Xm * Xm).sum(1)[None, :]
        ids = np.broadcast_to(mem[None, :], d.shape)
        for t in tiers:
            sub = rank[qs, l] < t
            if not sub.any():
                continue
            bd, bi = best[t]
            nd, ni = merge_top(bd[qs[sub]], bi[qs[sub]], d[sub], ids[sub], 10)
            bd[qs[sub]] = nd
            bi[qs[sub]] = ni
    for t in tiers:
        r = recall(best[t][1], gt)
        res[f"ceiling_nprobe{t}"] = r
        print(f"RESULT tag={tag} n={n} arm=ceiling nprobe={t} recall={r:.4f}", flush=True)
    res["t_ceiling_s"] = round(time.time() - t0, 1)

    # quantized arms: nprobe = args.nprobe, reorder = args.reorder, first nq_quant queries
    nqq = min(args.nq_quant, nq)
    Qq, gtq = Q[:nqq], gt[:nqq]
    rng = np.random.default_rng(args.seed + 1)
    t1 = time.time()
    R = X - C[assign]  # residuals (f32, n x dim)
    samp = rng.choice(n, min(32000, n), replace=False)
    blocks = DIM // 2
    cb = pq_train(R[samp], blocks, 16, 10, rng)
    codes_pq = pq_encode(R, cb)
    # per-list per-dim 4-bit grid over the members' residuals
    lo = np.zeros((k, DIM), np.float32)
    step = np.ones((k, DIM), np.float32)
    codes_grid = np.empty((n, DIM), np.uint8)
    for l in range(k):
        if counts[l] == 0:
            continue
        mem = order[offs[l]:offs[l + 1]]
        r = R[mem]
        lo[l] = r.min(0)
        step[l] = np.maximum((r.max(0) - lo[l]) / 15.0, 1e-9)
        codes_grid[mem] = np.clip(np.rint((r - lo[l]) / step[l]), 0, 15).astype(np.uint8)
    res["t_encode_s"] = round(time.time() - t1, 1)
    log("encoded pq and grid", res["t_encode_s"], "s")

    arms = {"pq50_float": None, "pq50_int8": None, "grid100_int8": None}
    cand = {a: (np.full((nqq, args.reorder), np.inf, np.float32), np.full((nqq, args.reorder), -1, np.int64)) for a in arms}
    ar_b = np.arange(blocks)
    ar_d = np.arange(DIM)
    t2 = time.time()
    for l in range(k):
        if counts[l] == 0:
            continue
        qs = np.nonzero(rank[:nqq, l] < args.nprobe)[0]
        if len(qs) == 0:
            continue
        mem = order[offs[l]:offs[l + 1]]
        L = len(mem)
        qr = Qq[qs] - C[l][None, :]  # query residual w.r.t. this list's center (m x dim)
        # PQ LUT: (m, blocks, 16) = |qr_b - cw_bv|^2
        lut = ((qr.reshape(len(qs), blocks, 1, 2) - cb[None]) ** 2).sum(3)
        oh = np.zeros((L, blocks * 16), np.float32)
        oh[np.arange(L)[:, None], ar_b[None, :] * 16 + codes_pq[mem]] = 1.0
        s_float = lut.reshape(len(qs), -1) @ oh.T
        s_int8 = int8_lut(lut).reshape(len(qs), -1) @ oh.T
        # grid LUT: (m, dim, 16) = (qr_j - lo_j - v*step_j)^2
        levels = lo[l][:, None] + step[l][:, None] * np.arange(16, dtype=np.float32)[None, :]  # dim x 16
        lutg = (qr[:, :, None] - levels[None]) ** 2
        ohg = np.zeros((L, DIM * 16), np.float32)
        ohg[np.arange(L)[:, None], ar_d[None, :] * 16 + codes_grid[mem]] = 1.0
        s_grid = int8_lut(lutg).reshape(len(qs), -1) @ ohg.T
        ids = np.broadcast_to(mem[None, :], (len(qs), L))
        for a, s in (("pq50_float", s_float), ("pq50_int8", s_int8), ("grid100_int8", s_grid)):
            bd, bi = cand[a]
            nd, ni = merge_top(bd[qs], bi[qs], s.astype(np.float32), ids, args.reorder)
            bd[qs] = nd
            bi[qs] = ni
    res["t_quant_scan_s"] = round(time.time() - t2, 1)
    q2q = (Qq * Qq).sum(1)
    for a in arms:
        found = np.empty((nqq, 10), np.int64)
        for i in range(nqq):
            c = cand[a][1][i]
            c = c[c >= 0]
            v = X[c].astype(np.float64) - Qq[i].astype(np.float64)
            d = (v * v).sum(1)
            top = c[np.argsort(d, kind="stable")[:10]]
            found[i, :len(top)] = top
            found[i, len(top):] = -1
        r = recall(found, gtq)
        res[f"{a}_nprobe{args.nprobe}_reorder{args.reorder}"] = r
        print(f"RESULT tag={tag} n={n} arm={a} nprobe={args.nprobe} reorder={args.reorder} nq={nqq} recall={r:.4f}", flush=True)
    res["t_total_s"] = round(time.time() - t0, 1)
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, f"l22a_{tag}.json"), "w") as f:
        json.dump(res, f, indent=1)
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=1_000_000)
    ap.add_argument("--train-sample", type=int, default=262144)
    ap.add_argument("--train-prefix", type=int, default=0, help="train on rows 0..P instead of a uniform sample")
    ap.add_argument("--lists", type=int, default=5000)
    ap.add_argument("--iters", type=int, default=10)
    ap.add_argument("--tiers", type=int, nargs="+", default=[350, 700, 1400])
    ap.add_argument("--nprobe", type=int, default=700)
    ap.add_argument("--reorder", type=int, default=317)
    ap.add_argument("--nq", type=int, default=1000)
    ap.add_argument("--nq-quant", type=int, default=500)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--out", default=os.path.join(PISO, "l22a", "out"))
    ap.add_argument("--synthetic", action="store_true")
    args = ap.parse_args()
    rng = np.random.default_rng(args.seed)
    if args.synthetic:
        n, k = 20000, 50
        X = rng.standard_normal((n, DIM), dtype=np.float32) + rng.standard_normal((k, DIM), dtype=np.float32)[rng.integers(k, size=n)] * 3
        Q = X[rng.choice(n, 100, replace=False)] + 0.1 * rng.standard_normal((100, DIM), dtype=np.float32)
        d = ((Q[:, None, :].astype(np.float64) - X[None].astype(np.float64)) ** 2).sum(2)
        gt = np.argsort(d, axis=1, kind="stable")[:, :10]
        args.lists, args.iters, args.tiers, args.nprobe = k, 5, [5, 10, k], k
        args.reorder, args.nq_quant = n, 100
        C = kmeans(X, k, args.iters, rng)
        assign, _ = sqdist_chunks(X, C, 5000)
        res = run(X, Q, gt, C, assign, args, "synthetic", args.out)
        ok = res[f"ceiling_nprobe{k}"] == 1.0 and all(res[f"{a}_nprobe{k}_reorder{n}"] == 1.0 for a in ("pq50_float", "pq50_int8", "grid100_int8"))
        print("SELFCHECK", "PASS" if ok else "FAIL", flush=True)
        sys.exit(0 if ok else 1)
    tag = f"{args.n}_{'prefix' + str(args.train_prefix) if args.train_prefix else 'sample' + str(args.train_sample)}"
    log("start", tag)
    n_tot = int(np.fromfile(f"{D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
    mm = np.memmap(f"{D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, DIM))
    X = np.ascontiguousarray(mm[:args.n])
    Q = np.load(f"{PISO}/queries1000.npy")[:args.nq]
    gt = np.load(f"{PISO}/gt_{args.n}.npy")[:args.nq]
    assert gt.max() < args.n
    log("loaded", X.shape, Q.shape, gt.shape)
    if args.train_prefix:
        T = X[:args.train_prefix]
        samp = rng.choice(len(T), min(args.train_sample, len(T)), replace=False)
        T = T[samp]
    else:
        T = X[rng.choice(args.n, min(args.train_sample, args.n), replace=False)]
    t = time.time()
    C = kmeans(T, args.lists, args.iters, rng)
    log("kmeans", round(time.time() - t, 1), "s on", len(T), "rows")
    t = time.time()
    assign, _ = sqdist_chunks(X, C, 20000)
    log("assign", round(time.time() - t, 1), "s")
    run(X, Q, gt, C, assign, args, tag, args.out)
    log("done", tag)


if __name__ == "__main__":
    main()
