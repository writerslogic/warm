"""L22 kill test A2: SOAR database spilling on msturing prefixes (numpy only).

Ceiling = |GT ∩ rows in the probed lists| / 10, exact (the global top-10 restricted to a subset is
the subset's top-10 among those ids). Rows live in their nearest list and, when spilling, in one
second list chosen by the SOAR loss |x-c'|^2 + lam * <x-c', r>^2 / |r|^2, r = x - c1.
PQ arms: PQ-50/16 residual codes per list membership, float LUT and INT8 LUT (de-quantized per
(query, list) before the cross-list merge), ids deduplicated before the exact reorder.
usage: l22a2.py --n N [--out DIR] | --synthetic
"""
import argparse, json, os, sys, time

import numpy as np

D = "/home/davidcondrey/bigann/s1c/bab/data/MSTuring-30M-clustered"
PISO = "/home/davidcondrey/bigann/piso"
DIM = 100


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def nearest(X, C, chunk=20000):
    c2 = (C * C).sum(1)
    out = np.empty(len(X), np.int32)
    for s in range(0, len(X), chunk):
        x = X[s:s + chunk]
        out[s:s + chunk] = (c2[None, :] - 2.0 * (x @ C.T)).argmin(1)
    return out


def kmeans(X, k, iters, rng):
    C = X[rng.choice(len(X), k, replace=False)].astype(np.float32)
    for it in range(iters):
        a = nearest(X, C)
        counts = np.bincount(a, minlength=k)
        sums = np.zeros((k, X.shape[1]), np.float64)
        np.add.at(sums, a, X)
        nz = counts > 0
        C[nz] = (sums[nz] / counts[nz, None]).astype(np.float32)
        empty = np.nonzero(~nz)[0]
        if len(empty):
            C[empty] = X[rng.choice(len(X), len(empty), replace=False)]
    log("kmeans done, max list", counts.max())
    return C


def soar_spill(X, C, a1, lam, chunk=20000):
    """Second list per row: argmin over c != c1 of |x-c|^2 + lam * <x-c, r>^2 / |r|^2."""
    c2 = (C * C).sum(1)
    out = np.empty(len(X), np.int32)
    for s in range(0, len(X), chunk):
        x = X[s:s + chunk]
        p = a1[s:s + chunk]
        r = x - C[p]
        rr = np.maximum((r * r).sum(1), 1e-20)
        xc = x @ C.T
        d = (x * x).sum(1)[:, None] - 2.0 * xc + c2[None, :]
        if lam:
            proj = (x * r).sum(1)[:, None] - r @ C.T  # <x - c, r>
            d = d + lam * proj * proj / rr[:, None]
        d[np.arange(len(x)), p] = np.inf
        out[s:s + chunk] = d.argmin(1)
    return out


def ceiling(rank, a1, a2, gt, counts, tiers):
    """recall and mean codes scanned per query for each nprobe tier."""
    res = {}
    g1 = a1[gt]  # nq x 10
    order = np.argsort(rank, axis=1)  # list ids by rank per query
    sizes = counts[order]  # nq x k, list sizes in rank order
    cum = np.cumsum(sizes, axis=1)
    for t in tiers:
        hit = np.take_along_axis(rank, g1, 1) < t
        if a2 is not None:
            hit |= np.take_along_axis(rank, a2[gt], 1) < t
        res[t] = (float(hit.mean()), float(cum[:, t - 1].mean()))
    return res


def pq_train(R, blocks, k, iters, rng):
    cb = np.empty((blocks, k, 2), np.float32)
    for b in range(blocks):
        X = R[:, 2 * b:2 * b + 2]
        Cb = X[rng.choice(len(X), k, replace=False)].copy()
        for _ in range(iters):
            asg = ((X[:, None, :] - Cb[None]) ** 2).sum(2).argmin(1)
            for j in range(k):
                m = asg == j
                Cb[j] = X[m].mean(0) if m.any() else X[rng.integers(len(X))]
        cb[b] = Cb
    return cb


def pq_encode(R, cb, chunk=200000):
    codes = np.empty((len(R), cb.shape[0]), np.uint8)
    for s in range(0, len(R), chunk):
        r = R[s:s + chunk]
        for b in range(cb.shape[0]):
            codes[s:s + chunk, b] = ((r[:, None, 2 * b:2 * b + 2] - cb[b][None]) ** 2).sum(2).argmin(1)
    return codes


def int8_dequant_scores(lut, oh):
    """INT8_LUT16 emulation, returned on the float scale: q_sum * scale + sum of block minimums."""
    mn = lut.min(2, keepdims=True)
    span = (lut.max(2, keepdims=True) - mn).max(1, keepdims=True)
    scale = np.maximum(span, 1e-12) / 255.0
    q = np.clip(np.rint((lut - mn) / scale), 0, 255).astype(np.float32)
    return (q.reshape(len(q), -1) @ oh.T) * scale[:, 0, 0][:, None] + mn.sum((1, 2))[:, None]


def recall(found, gt):
    return float(np.mean([len(set(f.tolist()) & set(g.tolist())) / gt.shape[1] for f, g in zip(found, gt)]))


def pq_arm(X, Q, gt, C, members, cb, codes, rank, nprobe, reorder, raw=False):
    """members: list -> (row ids, row positions into `codes`). Returns recall for float and int8 LUTs."""
    nq, blocks = len(Q), cb.shape[0]
    keep = 2 * reorder  # duplicates (spilled rows) are removed after the scan
    cand = {a: (np.full((nq, keep), np.inf, np.float32), np.full((nq, keep), -1, np.int64)) for a in ("float", "int8")}
    ar_b = np.arange(blocks)
    for l, (ids_l, pos_l) in enumerate(members):
        if len(ids_l) == 0:
            continue
        qs = np.nonzero(rank[:, l] < nprobe)[0]
        if len(qs) == 0:
            continue
        L = len(ids_l)
        qr = Q[qs] if raw else Q[qs] - C[l][None, :]
        lut = ((qr.reshape(len(qs), blocks, 1, 2) - cb[None]) ** 2).sum(3)
        oh = np.zeros((L, blocks * 16), np.float32)
        oh[np.arange(L)[:, None], ar_b[None, :] * 16 + codes[pos_l]] = 1.0
        scores = {"float": lut.reshape(len(qs), -1) @ oh.T, "int8": int8_dequant_scores(lut, oh)}
        ids = np.broadcast_to(ids_l[None, :], (len(qs), L))
        for a, s in scores.items():
            bd, bi = cand[a]
            cd = np.concatenate([bd[qs], s.astype(np.float32)], 1)
            ci = np.concatenate([bi[qs], ids], 1)
            if cd.shape[1] > keep:
                sel = np.argpartition(cd, keep - 1, axis=1)[:, :keep]
                cd, ci = np.take_along_axis(cd, sel, 1), np.take_along_axis(ci, sel, 1)
            else:
                pad = keep - cd.shape[1]
                cd = np.concatenate([cd, np.full((len(cd), pad), np.inf, np.float32)], 1)
                ci = np.concatenate([ci, np.full((len(ci), pad), -1, np.int64)], 1)
            bd[qs], bi[qs] = cd, ci
    out = {}
    for a, (bd, bi) in cand.items():
        found = np.full((nq, 10), -1, np.int64)
        for i in range(nq):
            o = np.argsort(bd[i], kind="stable")
            c = bi[i][o]
            c = c[c >= 0]
            _, first = np.unique(c, return_index=True)
            c = c[np.sort(first)][:reorder]  # best-scored copy of each id, then the reorder budget
            v = X[c].astype(np.float64) - Q[i].astype(np.float64)
            top = c[np.argsort((v * v).sum(1), kind="stable")[:10]]
            found[i, :len(top)] = top
        out[a] = recall(found, gt)
    return out


def build_members(a1, a2, k):
    """Per list: (row ids, positions into the stacked code array [primary codes; spill codes])."""
    n = len(a1)
    rows = np.arange(n)
    lists = [a1] if a2 is None else [a1, a2]
    allist = np.concatenate(lists)
    allrow = np.concatenate([rows] * len(lists))
    allpos = np.arange(len(allist))
    o = np.argsort(allist, kind="stable")
    counts = np.bincount(allist, minlength=k)
    offs = np.concatenate([[0], np.cumsum(counts)])
    return [(allrow[o[offs[l]:offs[l + 1]]], allpos[o[offs[l]:offs[l + 1]]]) for l in range(k)], counts


def run(X, Q, gt, args, tag, rng):
    n, k = len(X), args.lists
    res = {"tag": tag, "n": n, "lists": k}
    T = X[rng.choice(n, min(args.train_sample, n), replace=False)]
    C = kmeans(T, k, args.iters, rng)
    a1 = nearest(X, C)
    dq = (C * C).sum(1)[None, :] - 2.0 * (Q @ C.T)
    rank = np.argsort(np.argsort(dq, axis=1, kind="stable"), axis=1).astype(np.int32)
    spills = {}
    for lam in args.lams:
        t = time.time()
        spills[lam] = soar_spill(X, C, a1, lam)
        log(f"spill lam={lam} {time.time() - t:.1f}s")
    nq5 = min(args.nq_quant, len(Q))
    for label, a2 in [("nospill", None)] + [(f"spill_lam{lam}", spills[lam]) for lam in args.lams]:
        counts = np.bincount(a1, minlength=k) + (np.bincount(a2, minlength=k) if a2 is not None else 0)
        for nqs, sub in (("q1000", slice(None)), ("q500", slice(0, nq5))):
            c = ceiling(rank[sub], a1, a2, gt[sub], counts, args.tiers)
            for t, (r, scanned) in c.items():
                res[f"ceiling_{label}_{nqs}_nprobe{t}"] = {"recall": r, "codes_scanned": scanned}
                print(f"RESULT tag={tag} n={n} arm=ceiling_{label} nq={nqs} nprobe={t} recall={r:.4f} codes_scanned={scanned:.0f}", flush=True)
    # PQ arms on queries 0..nq5-1
    t = time.time()
    Qq, gtq, rq = Q[:nq5], gt[:nq5], rank[:nq5]
    a2 = spills[args.pq_lam]
    # --raw encodes the vectors themselves: one table per query serves every list.
    R1 = X if args.raw else X - C[a1]
    R2 = X if args.raw else X - C[a2]
    cb = pq_train(R1[rng.choice(n, min(32000, n), replace=False)], DIM // 2, 16, 10, rng)
    codes = np.concatenate([pq_encode(R1, cb), pq_encode(R2, cb)])
    log(f"pq encode {time.time() - t:.1f}s")
    mem_ns, cnt_ns = build_members(a1, None, k)
    mem_sp, cnt_sp = build_members(a1, a2, k)
    arms = [("nospill", mem_ns, cnt_ns, args.pq_nprobe[0]), ("nospill", mem_ns, cnt_ns, args.pq_nprobe[1])]
    arms += [(f"spill_lam{args.pq_lam}", mem_sp, cnt_sp, p) for p in args.spill_nprobe]
    if args.raw:
        arms = arms[2:]
    for label, mem, cnt, npb in arms:
        t = time.time()
        r = pq_arm(X, Qq, gtq, C, mem, cb, codes, rq, npb, args.reorder, args.raw)
        scanned = float(np.cumsum(cnt[np.argsort(rq, axis=1)], axis=1)[:, npb - 1].mean())
        for a, v in r.items():
            res[f"pq50_{a}_{label}_nprobe{npb}"] = {"recall": v, "codes_scanned": scanned}
            print(f"RESULT tag={tag} n={n} arm=pq50_{a}_{label} nq=q500 nprobe={npb} reorder={args.reorder} recall={v:.4f} codes_scanned={scanned:.0f}", flush=True)
        log(f"pq arm {label}@{npb} {time.time() - t:.1f}s")
    os.makedirs(args.out, exist_ok=True)
    with open(os.path.join(args.out, f"l22a2_{tag}.json"), "w") as f:
        json.dump(res, f, indent=1)
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=1_000_000)
    ap.add_argument("--train-sample", type=int, default=262144)
    ap.add_argument("--lists", type=int, default=5000)
    ap.add_argument("--iters", type=int, default=10)
    ap.add_argument("--tiers", type=int, nargs="+", default=[175, 350, 500, 700, 1000, 1400])
    ap.add_argument("--lams", type=float, nargs="+", default=[0.0, 1.0, 1.5])
    ap.add_argument("--pq-lam", type=float, default=1.0)
    ap.add_argument("--reorder", type=int, default=317)
    ap.add_argument("--pq-nprobe", type=int, nargs=2, default=[700, 1400], help="no-spill PQ arms")
    ap.add_argument("--spill-nprobe", type=int, nargs="+", default=[700], help="spilled PQ arms")
    ap.add_argument("--nq", type=int, default=1000)
    ap.add_argument("--nq-quant", type=int, default=500)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--out", default=os.path.join(PISO, "l22a", "out2"))
    ap.add_argument("--synthetic", action="store_true")
    ap.add_argument("--raw", action="store_true", help="PQ codes of the vectors, not of the residuals")
    args = ap.parse_args()
    rng = np.random.default_rng(args.seed)
    if args.synthetic:
        n, k = 20000, 50
        X = rng.standard_normal((n, DIM), dtype=np.float32) + rng.standard_normal((k, DIM), dtype=np.float32)[rng.integers(k, size=n)] * 3
        Q = X[rng.choice(n, 100, replace=False)] + 0.1 * rng.standard_normal((100, DIM), dtype=np.float32)
        gt = np.argsort(((Q[:, None, :].astype(np.float64) - X[None].astype(np.float64)) ** 2).sum(2), axis=1, kind="stable")[:, :10]
        args.lists, args.iters, args.tiers, args.train_sample, args.nq_quant = k, 5, [2, 5, k], n, 100
        args.pq_nprobe, args.spill_nprobe, args.reorder = [k, k], [k], 2000
        res = run(X, Q, gt, args, "synthetic", rng)
        full = [v["recall"] for key, v in res.items() if key.startswith("ceiling") and key.endswith(f"nprobe{k}")]
        mono = all(res[f"ceiling_spill_lam1.0_q1000_nprobe{t}"]["recall"] >= res[f"ceiling_nospill_q1000_nprobe{t}"]["recall"] for t in (2, 5))
        ok_pq = all(v["recall"] >= 0.99 for key, v in res.items() if key.startswith("pq50"))
        ok = all(r == 1.0 for r in full) and mono and ok_pq
        print("SELFCHECK", "PASS" if ok else "FAIL", "full-probe", full, "spill>=nospill", mono, "pq full-probe >= 0.99", ok_pq, flush=True)
        sys.exit(0 if ok else 1)
    tag = f"{args.n}_sample{args.train_sample}" + ("_raw" if args.raw else "")
    n_tot = int(np.fromfile(f"{D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
    mm = np.memmap(f"{D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, DIM))
    X = np.ascontiguousarray(mm[:args.n])
    Q = np.load(f"{PISO}/queries1000.npy")[:args.nq]
    gt = np.load(f"{PISO}/gt_{args.n}.npy")[:args.nq]
    assert gt.max() < args.n
    log("loaded", X.shape)
    run(X, Q, gt, args, tag, rng)
    log("done", tag)


if __name__ == "__main__":
    main()
