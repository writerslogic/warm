"""Round 2, Lead A: many unknown roles. VSA resonator store vs conventional structures at equal bits.

Registered in benchmarks/results/factstore_kill_20261114/PREREG.md (Round 2, Lead A).
Per-role codebooks play the role atoms (binding a role atom to a filler atom is a fixed re-randomisation
of the filler codebook, so a separate role atom adds nothing).
"""
import argparse
import json
import math
import os
import sys
import time

import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
from common import mix64, quantise  # noqa: E402

V = 256
BPR = [12, 16, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192]
TUNE_SEEDS = [0, 1]
CFGS = [(1, 1.0), (2, 1.0), (2, 2.0), (4, 1.0), (4, 2.0)]  # (w, kappa); w=1 keeps the sign


# ----------------------------------------------------------------------------- VSA
def gen_books(R, Dmax, rng):
    return [rng.integers(0, 2, size=(V, Dmax), dtype=np.uint8).astype(np.float32) * 2 - 1 for _ in range(R)]


def bundle(books, rec, Dmax, chunk=48):
    R = rec.shape[1]
    S = np.zeros(Dmax, np.float32)
    for i0 in range(0, len(rec), chunk):
        blk = rec[i0:i0 + chunk]
        P = books[0][blk[:, 0]]
        for r in range(1, R):
            P = P * books[r][blk[:, r]]
        S += P.sum(0)
    return S


def make_targets(Sq, books, rec, qi, U, D):
    R = rec.shape[1]
    T = np.tile(Sq[None, :D], (len(qi), 1))
    for r in range(R):
        m = ~(U == r).any(1)
        if m.any():
            T[m] *= books[r][rec[qi[m], r], :D]
    return T


def tuple_score(T, U, tup, books, D):
    P = T.copy()
    for j in range(U.shape[1]):
        for r in np.unique(U[:, j]):
            m = U[:, j] == r
            P[m] *= books[r][tup[m, j], :D]
    return P.sum(1)


def decode(T, U, books, D, iters, act, rng, init_random):
    """Batched resonator. T (n,D); U (n,u) unknown roles; returns (tuple (n,u), score (n,))."""
    n, u = U.shape
    est = []
    for j in range(u):
        e = np.empty((n, D), np.float32)
        for r in np.unique(U[:, j]):
            m = U[:, j] == r
            if init_random:
                g = rng.standard_normal((int(m.sum()), V)).astype(np.float32)
                e[m] = np.sign(g @ books[r][:, :D])
            else:
                e[m] = np.sign(books[r][:, :D].sum(0))
        e[e == 0] = 1
        est.append(e)
    prev = None
    tup = np.zeros((n, u), np.int64)
    for _ in range(iters):
        for j in range(u):
            x = T.copy()
            for k in range(u):
                if k != j:
                    x *= est[k]
            for r in np.unique(U[:, j]):
                m = U[:, j] == r
                Cr = books[r][:, :D]
                a = x[m] @ Cr.T
                tup[m, j] = a.argmax(1)
                if act == "hard":
                    est[j][m] = Cr[tup[m, j]]
                elif act == "cube":
                    est[j][m] = np.where((a * np.abs(a) * np.abs(a)) @ Cr >= 0, 1.0, -1.0)
                else:
                    est[j][m] = np.where(a @ Cr >= 0, 1.0, -1.0)
        if prev is not None and (tup == prev).all():
            break
        prev = tup.copy()
    return tup, tuple_score(T, U, tup, books, D)


def vsa_answer(T, U, books, D, iters, rho, act, rng, chunk=16):
    """Tuples (nq,u) and scores for the best of rho restarts (restart 0: superposition init, others random)."""
    nq, u = U.shape
    Tr = np.repeat(T, rho, axis=0)
    Ur = np.repeat(U, rho, axis=0)
    rand = np.tile(np.arange(rho) > 0, nq)
    tup = np.zeros((nq * rho, u), np.int64)
    sc = np.zeros(nq * rho, np.float32)
    for kind in (False, True):
        idx = np.flatnonzero(rand == kind)
        for c0 in range(0, len(idx), chunk):
            ii = idx[c0:c0 + chunk]
            tup[ii], sc[ii] = decode(Tr[ii], Ur[ii], books, D, iters, act, rng, kind)
    sc = sc.reshape(nq, rho)
    tup = tup.reshape(nq, rho, u)
    best = sc.argmax(1)
    return tup[np.arange(nq), best], sc[np.arange(nq), best]


# ----------------------------------------------------------------------------- baselines
def col_answer(kept, cue_cols, cue, miss):
    ix = np.flatnonzero((kept[:, cue_cols] == cue).all(1))
    return kept[ix[0], miss] if len(ix) else None


class Inv:
    def __init__(self, rec, ids):
        R = rec.shape[1]
        self.lists = {}
        bits = 0
        for r in range(R):
            for v in range(V):
                l = ids[rec[ids, r] == v]  # sorted ascending
                self.lists[(r, v)] = l
                if len(l):
                    bits += len(l) * (2 + math.ceil(math.log2(max(1.0, len(rec) / len(l)))))
        self.bits_per_rec = bits / max(len(ids), 1)

    def answer(self, cue_roles, cue_vals, U):
        order = sorted(range(len(cue_roles)), key=lambda t: len(self.lists[(cue_roles[t], cue_vals[t])]))
        cand = None
        for t in order:
            l = self.lists[(cue_roles[t], cue_vals[t])]
            cand = l if cand is None else np.intersect1d(cand, l, assume_unique=True)
            if len(cand) == 0:
                return None
        rid = cand[0]
        out = []
        for r in U:
            f = None
            for v in range(V):
                l = self.lists[(r, v)]
                p = np.searchsorted(l, rid)
                if p < len(l) and l[p] == rid:
                    f = v
                    break
            if f is None:
                return None
            out.append(f)
        return out


class RowBF:
    def __init__(self, rec, m, k, seed):
        N, R = rec.shape
        tok = np.arange(R * V, dtype=np.uint64)
        h = np.stack([mix64(tok * np.uint64(16) + np.uint64(j) + np.uint64(seed * 7919)) for j in range(k)], 1)
        self.pos = (h % np.uint64(m)).astype(np.int64)  # (R*V, k)
        self.bf = np.zeros((N, m), dtype=bool)
        t = rec + (np.arange(R) * V)[None, :]
        self.bf[np.repeat(np.arange(N), R * k), self.pos[t].reshape(N, -1).ravel()] = True

    def answer(self, cue_roles, cue_vals, U, rng):
        toks = np.array(cue_roles) * V + np.array(cue_vals)
        cands = np.flatnonzero(self.bf[:, self.pos[toks]].all(axis=(1, 2)))
        if len(cands) == 0:
            return None
        sets = []
        for c in cands:
            sets.append([np.flatnonzero(self.bf[c][self.pos[r * V + np.arange(V)]].all(1)) for r in U])
        ok = [s for s in sets if all(len(p) for p in s)]
        if not ok:
            return None
        per = ok[rng.integers(len(ok))]
        return [int(p[rng.integers(len(p))]) for p in per]


def fpf_acc(bpr, u):
    lam = (float(V) ** u - 1) * 2.0 ** (-bpr / 1.13)
    return 1.0 if lam < 1e-12 else (1 - math.exp(-lam)) / lam


def pat_acc(bpr, u):
    return 1.0 if bpr >= (u + 1) * u * 9.84 else 0.0


def known_of(R, Uq):
    return [r for r in range(R) if r not in Uq]


def baseline_acc(rec, qi, U, bpr, rng, seed, ks=(1, 2, 3, 4, 6, 8)):
    N, R = rec.shape
    out = {}
    kept = rec[:int(min(1.0, bpr / (8 * R)) * N)]
    ok = 0
    for i, Uq in zip(qi, U):
        kn = known_of(R, Uq)
        a = col_answer(kept, kn, rec[i, kn], Uq)
        ok += int(a is not None and (a == rec[i, Uq]).all())
    out["col"] = ok / len(qi)
    c = Inv(rec, np.arange(N)).bits_per_rec
    inv = Inv(rec, np.arange(int(min(1.0, bpr / c) * N)))
    ok = 0
    for i, Uq in zip(qi, U):
        kn = known_of(R, Uq)
        a = inv.answer(kn, rec[i, kn].tolist(), Uq.tolist())
        ok += int(a is not None and list(a) == rec[i, Uq].tolist())
    out["inv"] = ok / len(qi)
    out["inv_bits_per_rec_full"] = c
    best = (-1.0, 0)
    for k in ks:
        if k > bpr:
            continue
        bf = RowBF(rec, bpr, k, seed)
        ok = 0
        for i, Uq in zip(qi, U):
            kn = known_of(R, Uq)
            a = bf.answer(kn, rec[i, kn].tolist(), Uq.tolist(), rng)
            ok += int(a is not None and list(a) == rec[i, Uq].tolist())
        if ok / len(qi) > best[0]:
            best = (ok / len(qi), k)
    out["rowbf"], out["rowbf_k"] = best
    u = U.shape[1]
    out["fpf"] = fpf_acc(bpr, u)
    out["pat"] = pat_acc(bpr, u)
    return out


# ----------------------------------------------------------------------------- driver
def gen_queries(rng, N, R, u, nq):
    qi = rng.integers(0, N, nq)
    U = np.sort(np.stack([rng.choice(R, u, replace=False) for _ in range(nq)]), 1)
    return qi, U


def vsa_eval(S, books, rec, qi, U, bpr, cfg, N, rng, act):
    w, kap, rho, iters = cfg
    D = int(N * bpr // w)
    Sq = quantise(S[:D], w, kap, N) if w > 1 else np.where(S[:D] > 0, 1.0, -1.0).astype(np.float32)
    T = make_targets(Sq, books, rec, qi, U, D)
    tup, sc = vsa_answer(T, U, books, D, iters, rho, act, rng)
    truth = np.stack([rec[i, Uq] for i, Uq in zip(qi, U)])
    okq = (tup == truth).all(1)
    ts = tuple_score(T, U, truth, books, D)
    fail = ~okq
    return float(okq.mean()), (float((ts[fail] > sc[fail]).mean()) if fail.any() else 0.0)


def setup(seed, N, R, u, Dmax):
    rng = np.random.default_rng(seed * 1000003 + N * 31 + R * 7 + u)
    rec = rng.integers(0, V, size=(N, R))
    books = gen_books(R, Dmax, rng)
    return rng, rec, books, bundle(books, rec, Dmax)


def run_cell(a):
    R, u, N = a.R, a.u, a.N
    bprs = [int(x) for x in a.bprs.split(",")] if a.bprs else [b for b in BPR if b <= a.maxbpr]
    Dmax = N * max(bprs)
    out = {"R": R, "u": u, "N": N, "bprs": bprs, "act": a.act, "tune": {}, "seeds": {}, "args": vars(a)}
    t0 = time.time()
    tr = {}
    for seed in TUNE_SEEDS:
        tr[seed] = setup(seed, N, R, u, Dmax)
        tr[seed] += (gen_queries(tr[seed][0], N, R, u, a.nq_tune),)
    cfgsel = {}
    dead = False
    for b in sorted(bprs, reverse=True):  # top-down: a point below a dead point is dead (monotone in bits)
        if dead:
            cfgsel[b] = {"w": 1, "kappa": 1.0, "rho": 1, "acc_tune": 0.0, "search_fail_tune": 0.0, "strong": False, "inferred_dead": True}
            continue
        sc = {}
        for (w, kap) in CFGS:
            vals = []
            for seed in TUNE_SEEDS:
                rng, rec, books, S, (qi, U) = tr[seed]
                vals.append(vsa_eval(S, books, rec, qi, U, b, (w, kap, 1, 40), N, rng, a.act)[0])
                if seed == 0 and max(vals) < 0.02 and w == 1:
                    pass
            sc[(w, kap)] = float(np.mean(vals))
        bw = max(sc, key=sc.get)
        if sc[bw] < 0.02:
            dead = True
            cfgsel[b] = {"w": bw[0], "kappa": bw[1], "rho": 1, "acc_tune": sc[bw], "search_fail_tune": 0.0, "strong": False}
            continue
        r4 = []
        for seed in TUNE_SEEDS:
            rng, rec, books, S, (qi, U) = tr[seed]
            r4.append(vsa_eval(S, books, rec, qi, U, b, (bw[0], bw[1], 4, 40), N, rng, a.act)[0])
        rho = 4 if np.mean(r4) > sc[bw] + 1e-9 else 1
        acc = max(float(np.mean(r4)), sc[bw])
        sfs = []
        for seed in TUNE_SEEDS:
            rng, rec, books, S, (qi, U) = tr[seed]
            sfs.append(vsa_eval(S, books, rec, qi, U, b, (bw[0], bw[1], rho, 40), N, rng, a.act)[1])
        sf = float(np.mean(sfs))
        strong = bool(acc < 0.99 and acc > 0.05 and sf > 0.2)
        cfgsel[b] = {"w": bw[0], "kappa": bw[1], "rho": rho, "acc_tune": acc, "search_fail_tune": sf, "strong": strong}
    del tr
    out["tune"] = {str(b): v for b, v in cfgsel.items()}
    print(f"tuned {time.time() - t0:.0f}s", {b: (v["w"], v["kappa"], v["rho"], round(v["acc_tune"], 2), v["strong"]) for b, v in cfgsel.items()}, flush=True)

    seeds = list(range(100, 100 + a.seeds))
    active = {b: cfgsel[b]["acc_tune"] >= 0.02 for b in bprs}
    out["tune_dead"] = [b for b in bprs if not active[b]]
    for si, seed in enumerate(seeds):
        rng, rec, books, S = setup(seed, N, R, u, Dmax)
        qi, U = gen_queries(rng, N, R, u, a.nq)
        row = {}
        for b in bprs:
            c = cfgsel[b]
            r = baseline_acc(rec, qi, U, b, rng, seed)
            if active[b]:
                cfg = (c["w"], c["kappa"], 16 if c["strong"] else c["rho"], 100 if c["strong"] else 40)
                r["vsa"], r["vsa_search_fail"] = vsa_eval(S, books, rec, qi, U, b, cfg, N, rng, a.act)
            row[str(b)] = r
        out["seeds"][str(seed)] = row
        del books, S
        if si == 2:  # 3-seed screen
            for b in bprs:
                if active[b] and np.mean([out["seeds"][str(s)][str(b)]["vsa"] for s in seeds[:3]]) < 0.5:
                    active[b] = False
                    out.setdefault("screened_dead", []).append(b)
            for b in bprs:
                if active[b] and np.mean([out["seeds"][str(s)][str(b)]["vsa"] for s in seeds[:3]]) >= 0.999:
                    for b2 in bprs:
                        if b2 > b and active[b2]:
                            active[b2] = False
                            out.setdefault("screened_saturated", []).append(b2)
                    break
        json.dump(out, open(a.out, "w"))
        print(f"seed {seed} done {time.time() - t0:.0f}s active={[b for b in bprs if active[b]]}", flush=True)
    out["done"] = True
    json.dump(out, open(a.out, "w"))


def run_time(a):
    """Single-thread query timing at seed 100, tuned config read from the accuracy run."""
    acc = json.load(open(a.tuned))
    R, u, N = acc["R"], acc["u"], acc["N"]
    bprs = [b for b in acc["bprs"] if b <= a.maxbpr]
    Dmax = N * max(bprs)
    rng, rec, books, S = setup(100, N, R, u, Dmax)
    qi, U = gen_queries(rng, N, R, u, a.ntime)
    res = {"R": R, "u": u, "N": N, "ms": {}}
    cinv = Inv(rec, np.arange(N)).bits_per_rec
    for b in bprs:
        c = acc["tune"][str(b)]
        w, kap = c["w"], c["kappa"]
        rho, iters = (16, 100) if c["strong"] else (c["rho"], 40)
        D = int(N * b // w)
        Sq = quantise(S[:D], w, kap, N) if w > 1 else np.where(S[:D] > 0, 1.0, -1.0).astype(np.float32)
        tv, okv = [], 0
        for i in range(a.ntime):
            t = time.perf_counter()
            T = make_targets(Sq, books, rec, qi[i:i + 1], U[i:i + 1], D)
            tup, _ = vsa_answer(T, U[i:i + 1], books, D, iters, rho, a.act, rng)
            tv.append(time.perf_counter() - t)
            okv += int((tup[0] == rec[qi[i], U[i]]).all())
        m = {"vsa": float(np.median(tv) * 1e3), "vsa_acc_timed": okv / a.ntime, "w": w, "rho": rho, "iters": iters}
        kept = rec[:int(min(1.0, b / (8 * R)) * N)]
        tv = []
        for rep in range(5):
            for i in range(a.ntime):
                kn = known_of(R, U[i])
                t = time.perf_counter()
                col_answer(kept, kn, rec[qi[i], kn], U[i])
                tv.append(time.perf_counter() - t)
        m["col"] = float(np.median(tv) * 1e3)
        inv = Inv(rec, np.arange(int(min(1.0, b / cinv) * N)))
        tv = []
        for i in range(a.ntime):
            kn = known_of(R, U[i])
            t = time.perf_counter()
            inv.answer(kn, rec[qi[i], kn].tolist(), U[i].tolist())
            tv.append(time.perf_counter() - t)
        m["inv"] = float(np.median(tv) * 1e3)
        bf = RowBF(rec, b, max(1, min(4, b // 8)), 100)
        tv = []
        for i in range(a.ntime):
            kn = known_of(R, U[i])
            t = time.perf_counter()
            bf.answer(kn, rec[qi[i], kn].tolist(), U[i].tolist(), rng)
            tv.append(time.perf_counter() - t)
        m["rowbf"] = float(np.median(tv) * 1e3)
        res["ms"][str(b)] = m
        print(b, m, flush=True)
        json.dump(res, open(a.out, "w"))
    res["done"] = True
    json.dump(res, open(a.out, "w"))


def run_oracle(a):
    """Exhaustive decode of the same bundle (u=3 only): scores all V^3 tuples. Information limit of the store."""
    R, u, N = a.R, 3, a.N
    bprs = [int(x) for x in a.bprs.split(",")]
    Dmax = N * max(bprs)
    out = {"R": R, "u": u, "N": N, "mode": "oracle", "seeds": {}}
    for seed in range(100, 100 + a.seeds):
        rng, rec, books, S = setup(seed, N, R, u, Dmax)
        qi, U = gen_queries(rng, N, R, u, a.nq)
        row = {}
        for b in bprs:
            for w in (1, 2):
                D = int(N * b // w)
                Sq = quantise(S[:D], w, 2.0, N) if w > 1 else np.where(S[:D] > 0, 1.0, -1.0).astype(np.float32)
                T = make_targets(Sq, books, rec, qi, U, D)
                ok = 0
                tv = []
                for q in range(len(qi)):
                    t0 = time.perf_counter()
                    ra, rb, rc = U[q]
                    A, B_, C_ = books[ra][:, :D], books[rb][:, :D], books[rc][:, :D]
                    best, arg = -1e30, None
                    for c in range(V):
                        M = T[q] * C_[c]
                        Z = (A * M[None, :]) @ B_.T
                        i = int(Z.argmax())
                        if Z.flat[i] > best:
                            best, arg = float(Z.flat[i]), (i // V, i % V, c)
                    tv.append(time.perf_counter() - t0)
                    ok += int(list(arg) == rec[qi[q], U[q]].tolist())
                row[f"{b}|w{w}"] = {"acc": ok / len(qi), "s_per_query": float(np.median(tv))}
        out["seeds"][str(seed)] = row
        json.dump(out, open(a.out, "w"))
        print("oracle seed", seed, {k: v["acc"] for k, v in row.items()}, flush=True)
    out["done"] = True
    json.dump(out, open(a.out, "w"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["acc", "time", "oracle"])
    ap.add_argument("--R", type=int, default=12)
    ap.add_argument("--u", type=int, default=3)
    ap.add_argument("--N", type=int, default=256)
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--nq", type=int, default=100)
    ap.add_argument("--nq-tune", dest="nq_tune", type=int, default=32)
    ap.add_argument("--maxbpr", type=int, default=192)
    ap.add_argument("--act", default="lin")
    ap.add_argument("--bprs", default="")
    ap.add_argument("--tuned")
    ap.add_argument("--ntime", type=int, default=10)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    {"acc": run_cell, "time": run_time, "oracle": run_oracle}[a.mode](a)


if __name__ == "__main__":
    main()
