"""K2 near-key lookup, K3 set similarity, K4 partial-cue role-filler retrieval (see PREREG.md)."""
import argparse
import json
import time

import numpy as np

from common import Synth, LoCoMoText, TUNE_SEEDS, REPORT_SEEDS, boot_ci, mix64, proj, proj_sum, quantise

V = 1024


def atoms(ids, D, salt):
    """Deterministic +-1 atoms for integer ids: (len(ids), D) float32."""
    j = np.arange(D, dtype=np.uint64)
    x = mix64(np.asarray(ids, dtype=np.uint64)[:, None] * np.uint64(0x2545F4914F6CDD1D) + j[None, :] + np.uint64(salt))
    return np.where((x & np.uint64(1)) == 1, 1.0, -1.0).astype(np.float32)


# ---------------------------------------------------------------- K2
def k2_seed(src, seed, N, B, nq, level):
    rng = np.random.default_rng(seed * 7919 + N)
    X, mi, nm, mem = src.sample(rng, N, nq)
    vals = rng.integers(0, V, size=N)
    q = mem[level]
    truth = vals[mi]
    out = {}
    P = rng.standard_normal((src.d, max(B, 1024))).astype(np.float32)
    # stored signature + 10 bit id, scan; fraction f of items kept
    for f in (1.0, 0.5, 0.25):
        nk = max(1, int(N * f))
        m = B // nk - 10
        if m < 4:
            continue
        sig = proj(X[:nk], P[:, :m])
        qc = proj(q, P[:, :m])
        best = (qc @ sig.T).argmax(axis=1)
        out[f"sig|f{f}"] = float((vals[best] == truth).mean())
    # LSH static functions (idealised Bloomier: 1.23*10 bits per item per table), majority vote
    for r in (8, 12, 16, 20):
        T = int(B // (12.3 * N))
        if T < 1:
            break
        votes = np.full((len(mi), T), -1)
        for t in range(T):
            cols = slice(t * r % 1000, t * r % 1000 + r) if t * r % 1000 + r <= 1024 else slice(0, r)
            Pt = P[:, cols]
            w = (1 << np.arange(Pt.shape[1])).astype(np.int64)
            ik = ((proj(X, Pt) > 0).astype(np.int64) * w).sum(axis=1)
            iq = ((proj(q, Pt) > 0).astype(np.int64) * w).sum(axis=1)
            tab = dict(zip(ik.tolist(), vals.tolist()))
            votes[:, t] = [tab.get(k, int(rng.integers(0, V))) for k in iq.tolist()]
        pred = np.array([np.bincount(v, minlength=V).argmax() for v in votes])
        out[f"lshfn|r{r}|T{T}"] = float((pred == truth).mean())
    # VSA dense: S = sum sign(Xp) * atom(val); query unbinds with sign(q) and cleans up
    for w in (1, 2, 4, 8):
        D = B // w
        if D < 64:
            continue
        A = atoms(np.arange(V), D, 11)
        S = np.zeros(D, dtype=np.float32)
        for i0 in range(0, N, 256):
            c = proj(X[i0:i0 + 256], P[:, :D]) if D <= P.shape[1] else None
            if c is None:
                break
            S += (c * A[vals[i0:i0 + 256]]).sum(axis=0)
        if D > P.shape[1]:
            continue
        for kap in ([1.0] if w in (1, 8) else (1.0, 2.0, 3.0)):
            Sq = quantise(S, w, kap, N)
            qc = proj(q, P[:, :D])
            pred = ((qc * Sq) @ A.T).argmax(axis=1)
            out[f"vsa|w{w}|k{kap}"] = float((pred == truth).mean())
    return out


def run_k2(a):
    src = Synth() if a.source == "synth" else LoCoMoText(a.locomo)
    res = {}
    rng = np.random.default_rng(3)
    for N in [int(x) for x in a.Ns.split(",")]:
        if N > src.max_n():
            continue
        for B in [int(x) for x in a.Bs.split(",")]:
            if B / N < 12:
                continue
            for lv in src.levels:
                allr = {s: k2_seed(src, s, N, B, a.nq, lv) for s in TUNE_SEEDS + REPORT_SEEDS[: a.seeds]}
                rep = [s for s in allr if s not in TUNE_SEEDS]
                fams = {}
                for arm in allr[rep[0]]:
                    fams.setdefault(arm.split("|")[0], []).append(arm)
                best = {}
                for fam, arms in fams.items():
                    sc = {x: np.mean([allr[s].get(x, 0) for s in TUNE_SEEDS]) for x in arms}
                    best[fam] = max(sc, key=sc.get)
                per = {fam: np.array([allr[s].get(x, 0) for s in rep]) for fam, x in best.items()}
                base = [f for f in per if f != "vsa"]
                strongest = max(base, key=lambda f: per[f].mean()) if base else None
                cell = {"arms": {f: {"arm": best[f], "acc": float(per[f].mean())} for f in per}, "strongest_baseline": strongest}
                if strongest and "vsa" in per:
                    m, lo, hi = boot_ci(per["vsa"] - per[strongest], rng)
                    cell["vsa_minus_strongest"] = {"mean": m, "ci95": [lo, hi]}
                res[f"N={N}|B={B}|level={lv}"] = cell
                print("k2", N, B, lv, {f: round(c["acc"], 3) for f, c in cell["arms"].items()}, flush=True)
                json.dump({"source": src.name, "results": res}, open(a.out, "w"), indent=1)


# ---------------------------------------------------------------- K3
def minhash_sig(elems, k, bits):
    seeds = mix64(np.arange(k, dtype=np.uint64) + np.uint64(99))
    m = np.empty(k, dtype=np.uint64)
    for t in range(0, k, 256):
        h = mix64(elems[:, None] ^ seeds[None, t:t + 256])
        m[t:t + 256] = h.min(axis=0)
    return (m & np.uint64((1 << bits) - 1)).astype(np.int64)


def k3_cell(n, J, B, rng, pairs):
    # |A|=|B|=n, |A&B| = c with J = c/(2n-c)
    c = int(round(2 * n * J / (1 + J)))
    Jt = c / (2 * n - c)
    out = {"mh": {b: [] for b in (1, 2, 4, 8)}, "vsa": {}}
    for _ in range(pairs):
        base = rng.integers(1, 2 ** 62, size=2 * n - c, dtype=np.int64).astype(np.uint64)
        A = base[:n]
        Bs = np.concatenate([base[:c], base[n:]])
        for b in out["mh"]:
            k = B // b
            sa, sb = minhash_sig(A, k, b), minhash_sig(Bs, k, b)
            pm = (sa == sb).mean()
            est = (pm - 2.0 ** -b) / (1 - 2.0 ** -b)
            out["mh"][b].append(est - Jt)
        for w in (1, 2, 4, 8):
            D = B // w
            Sa = atoms_hash(A, D).sum(axis=0)
            Sb = atoms_hash(Bs, D).sum(axis=0)
            for kap in ([1.0] if w in (1, 8) else (1.0, 2.0, 3.0)):
                qa, qb = quantise(Sa, w, kap, n), quantise(Sb, w, kap, n)
                cs = float(qa @ qb / (np.linalg.norm(qa) * np.linalg.norm(qb) + 1e-9))
                if w == 1:  # sign quantisation shrinks the correlation: arcsine law
                    cs = float(np.sin(np.pi / 2 * cs))
                cs = min(max(cs, 0.0), 1.0)
                est = cs / (2 - cs)
                out["vsa"].setdefault((w, kap), []).append(est - Jt)
    return out


def atoms_hash(elems, D):
    return atoms(elems.astype(np.uint64) if elems.dtype != np.uint64 else elems, D, 5)


def run_k3(a):
    rng = np.random.default_rng(5)
    res = {}
    for n in (100, 1000):
        for B in (256, 1024, 4096):
            for J in (0.1, 0.3, 0.5, 0.7, 0.9):
                o = k3_cell(n, J, B, rng, a.k3pairs)
                rm = {f"mh{b}": float(np.sqrt(np.mean(np.square(v)))) for b, v in o["mh"].items()}
                rv = {f"vsa_w{w}_k{k}": float(np.sqrt(np.mean(np.square(v)))) for (w, k), v in o["vsa"].items()}
                bm = min(rm, key=rm.get)
                bv = min(rv, key=rv.get)
                res[f"n={n}|B={B}|J={J}"] = {"best_minhash": bm, "rmse_minhash": rm[bm], "best_vsa": bv,
                                              "rmse_vsa": rv[bv], "vsa_over_minhash": rv[bv] / rm[bm]}
                print("k3", n, B, J, round(rm[bm], 4), round(rv[bv], 4), flush=True)
                json.dump({"results": res, "note": "tuning on the same pairs (selects min RMSE per arm: favours both arms)"},
                          open(a.out, "w"), indent=1)


# ---------------------------------------------------------------- K4
def k4_cell(N, Bmul, rng, R=6, Vf=256, ncue=5, corrupt=False, nq=200):
    rec = rng.integers(0, Vf, size=(N, R))
    qi = rng.integers(0, N, size=min(nq, N))
    miss = rng.integers(0, R, size=len(qi))
    out = {}
    # exact table, scan with tolerance: pick the record with most matching cue roles
    pred_t = []
    for i, m in zip(qi, miss):
        cue = rec[i].copy()
        known = [r for r in range(R) if r != m]
        if corrupt:
            cue[rng.choice(known)] = rng.integers(0, Vf)
        match = (rec[:, known] == cue[known]).sum(axis=1)
        best = np.flatnonzero(match == match.max())
        pred_t.append(rec[best[0], m])
    out["table"] = float((np.array(pred_t) == rec[qi, miss]).mean())
    B = int(N * R * 8 * Bmul)
    for w in (1, 2, 4):
        D = B // w
        if D < 64:
            continue
        RA = atoms(np.arange(R), D, 21)
        FA = [atoms(np.arange(Vf), D, 100 + r) for r in range(R)]
        S = np.zeros(D, dtype=np.float32)
        for i in range(N):
            v = np.ones(D, dtype=np.float32)
            for r in range(R):
                v = v * RA[r] * FA[r][rec[i, r]]
            S += v
        for kap in ([1.0] if w == 1 else (1.0, 2.0)):
            Sq = quantise(S, w, kap, N)
            ok = 0
            for i, m in zip(qi, miss):
                v = Sq.copy()
                for r in range(R):
                    if r == m:
                        continue
                    f = rec[i, r]
                    if corrupt and r == (m + 1) % R:
                        f = (f + 1) % Vf
                    v = v * RA[r] * FA[r][f]
                v = v * RA[m]
                ok += int((FA[m] @ v).argmax() == rec[i, m])
            out[f"vsa|w{w}|k{kap}"] = ok / len(qi)
    return out


def run_k4(a):
    rng = np.random.default_rng(9)
    res = {}
    for corrupt in (False, True):
        for N in (50, 100, 200, 400, 800):
            for mul in (1, 2, 4, 8):
                o = [k4_cell(N, mul, rng, corrupt=corrupt) for _ in range(3)]
                keys = o[0].keys()
                mean = {k: float(np.mean([x[k] for x in o])) for k in keys}
                bv = max((k for k in mean if k.startswith("vsa")), key=mean.get, default=None)
                res[f"corrupt={corrupt}|N={N}|bits_mul={mul}"] = {"table": mean["table"], "best_vsa": bv, "vsa": mean.get(bv)}
                print("k4", corrupt, N, mul, mean["table"], bv, mean.get(bv), flush=True)
                json.dump(res, open(a.out, "w"), indent=1)


# ---------------------------------------------------------------- K4b: sub-information budgets
def bloom_triples_acc(rec, qi, miss, B, k, Vf, rng):
    N, R = rec.shape
    bl = np.zeros(B, dtype=bool)

    def keyhash(row_others, m, f):
        h = np.uint64(m + 1) * np.uint64(0x9E3779B97F4A7C15)
        for v in row_others:
            h = mix64(np.asarray(h, dtype=np.uint64) ^ (np.asarray(v, dtype=np.uint64) + np.uint64(0x1234567)))
        return mix64(np.asarray(h, dtype=np.uint64) ^ (np.asarray(f, dtype=np.uint64) * np.uint64(0xD6E8FEB86659FD93)))

    def pos(h):
        h2 = mix64(h) | np.uint64(1)
        j = np.arange(k, dtype=np.uint64)
        return ((h[..., None] + h2[..., None] * j) % np.uint64(B)).astype(np.int64)

    for m in range(R):
        others = [rec[:, r].astype(np.uint64) for r in range(R) if r != m]
        bl[pos(keyhash(others, m, rec[:, m].astype(np.uint64))).ravel()] = True
    ok = 0.0
    cand = np.arange(Vf, dtype=np.uint64)
    for i, m in zip(qi, miss):
        others = [np.full(Vf, rec[i, r], dtype=np.uint64) for r in range(R) if r != m]
        hit = bl[pos(keyhash(others, m, cand))].all(axis=1)
        npos = hit.sum()
        ok += (1.0 / npos) if hit[rec[i, m]] and npos > 0 else 0.0
    return ok / len(qi)


def k4b_cell(N, mul, rng, R=6, Vf=256, nq=200):
    rec = rng.integers(0, Vf, size=(N, R))
    qi = rng.integers(0, N, size=min(nq, N))
    miss = rng.integers(0, R, size=len(qi))
    B = max(int(N * R * 8 * mul), 64)
    out = {"table_subset": min(1.0, B / (N * R * 8))}
    for k in (1, 2, 3, 4, 6, 8):
        out[f"bloomtriples|k{k}"] = bloom_triples_acc(rec, qi, miss, B, k, Vf, rng)
    for w in (1, 2, 4):
        D = B // w
        if D < 64:
            continue
        RA = atoms(np.arange(R), D, 21)
        FA = [atoms(np.arange(Vf), D, 100 + r) for r in range(R)]
        S = np.zeros(D, dtype=np.float32)
        for i in range(N):
            v = RA[0] * FA[0][rec[i, 0]]
            for r in range(1, R):
                v = v * RA[r] * FA[r][rec[i, r]]
            S += v
        for kap in ([1.0] if w == 1 else (1.0, 2.0, 3.0)):
            Sq = quantise(S, w, kap, N)
            ok = 0
            for i, m in zip(qi, miss):
                v = Sq * RA[m]
                for r in range(R):
                    if r != m:
                        v = v * RA[r] * FA[r][rec[i, r]]
                ok += int((FA[m] @ v).argmax() == rec[i, m])
            out[f"vsa|w{w}|k{kap}"] = ok / len(qi)
    return out


def run_k4b(a):
    rng = np.random.default_rng(13)
    res = {}
    for N in (200, 800, 1600):
        for mul in (0.0625, 0.125, 0.25, 0.5, 0.75, 1.0, 2.0):
            tune = [k4b_cell(N, mul, rng) for _ in range(2)]
            rep = [k4b_cell(N, mul, rng) for _ in range(10)]
            fam = {}
            for arm in rep[0]:
                fam.setdefault(arm.split("|")[0], []).append(arm)
            best = {f: max(arms, key=lambda x: np.mean([t[x] for t in tune if x in t] or [0])) for f, arms in fam.items()}
            per = {f: np.array([r[best[f]] for r in rep]) for f in best}
            cell = {f: {"arm": best[f], "acc": float(per[f].mean())} for f in per}
            lossy_base = max((f for f in per if f != "vsa"), key=lambda f: per[f].mean())
            m, lo, hi = boot_ci(per["vsa"] - per[lossy_base], rng)
            cell["strongest_baseline"] = lossy_base
            cell["vsa_minus_strongest"] = {"mean": m, "ci95": [lo, hi]}
            # information bound: best accuracy for any code at R bits per answer (rate-distortion, uniform symbol, 0/1 loss)
            Rbits = B_per_answer = max(int(N * 6 * 8 * mul), 64) / (N * 6)
            Vf = 256
            pe = np.linspace(1e-6, 1 - 1 / Vf, 20000)
            rate = np.log2(Vf) + pe * np.log2(np.maximum(pe, 1e-12) / (Vf - 1)) + (1 - pe) * np.log2(np.maximum(1 - pe, 1e-12))
            ok = pe[rate <= Rbits]
            cell["rate_distortion_ceiling_acc"] = float(1 - ok.min()) if len(ok) else 1.0 / Vf
            res[f"N={N}|bits_mul={mul}"] = cell
            print("k4b", N, mul, {f: round(c["acc"], 3) for f, c in cell.items() if isinstance(c, dict) and "acc" in c}, cell["rate_distortion_ceiling_acc"], flush=True)
            json.dump(res, open(a.out, "w"), indent=1)


# ---------------------------------------------------------------- K4c: u unknown roles, strongest conventional = parity-address tables
PAT_BITS = 1.23 * 8  # idealised Bloomier (static function) cost per stored 8-bit symbol per key


def pat_build(rec, comps, budget, rng):
    """Parity-address tables. comps: [(address_fields, n_syms)], filled in order within a per-record bit budget."""
    N = len(rec)
    tabs, left = [], float(budget)
    for addr, ns in comps:
        c = PAT_BITS * ns
        frac = min(1.0, max(0.0, left / (c * N)))
        left -= c * N * frac
        if frac <= 0:
            break
        inc = np.arange(N) if frac >= 1 else rng.choice(N, size=int(N * frac), replace=False)
        tab = {}
        for i in inc:
            syms = [int(sum((j + 1) ** t * rec[i, j] for j in range(rec.shape[1])) % 257) for t in range(ns)]
            tab[tuple(rec[i, list(addr)])] = syms
        tabs.append((addr, ns, tab))
    return tabs


def pat_answer(tabs, rec_row, U, R=6):
    K = [j for j in range(R) if j not in U]
    for addr, ns, tab in tabs:
        if any(a in U for a in addr) or len(U) > ns:
            continue
        got = tab.get(tuple(rec_row[list(addr)]))
        if got is None:
            continue
        ps = [(got[t] - sum((j + 1) ** t * int(rec_row[j]) for j in K)) % 257 for t in range(len(U))]
        if len(U) == 1:
            if ps[0] <= 255:
                return {U[0]: ps[0]}
        else:
            a, b = U
            wa, wb = a + 1, b + 1
            fb = ((ps[1] - wa * ps[0]) * pow((wb - wa) % 257, 255, 257)) % 257
            fa = (ps[0] - fb) % 257
            if fa <= 255 and fb <= 255:
                return {a: fa, b: fb}
    return None


def k4c_cell(N, mul, u, rng, R=6, Vf=256, nq=60):
    rec = rng.integers(0, Vf, size=(N, R))
    qi = rng.integers(0, N, size=min(nq, N))
    Us = [sorted(rng.choice(R, size=u, replace=False).tolist()) for _ in qi]
    B = max(int(N * mul), 64)  # mul = bits per record here
    out = {"table_subset": min(1.0, B / (N * R * 8))}
    comps = [((0, 1), u), ((2, 3), u), ((4, 5), u)]
    tabs = pat_build(rec, comps, B, rng)
    ok = 0
    for i, U in zip(qi, Us):
        ans = pat_answer(tabs, rec[i], U, R)
        ok += int(ans is not None and all(ans[j] == rec[i, j] for j in U))
    out["pat"] = ok / len(qi)
    for w in (1, 2):
        D = B // w
        if D < 64:
            continue
        RA = atoms(np.arange(R), D, 21)
        FA = [atoms(np.arange(Vf), D, 100 + r) for r in range(R)]
        S = np.zeros(D, dtype=np.float32)
        for i in range(N):
            v = RA[0] * FA[0][rec[i, 0]]
            for r in range(1, R):
                v = v * RA[r] * FA[r][rec[i, r]]
            S += v
        for kap in ([1.0] if w == 1 else (1.0, 2.0)):
            Sq = quantise(S, w, kap, N)
            ok = 0
            for i, U in zip(qi, Us):
                v = Sq.copy()
                for r in range(R):
                    v = v * RA[r]
                    if r not in U:
                        v = v * FA[r][rec[i, r]]
                if u == 1:
                    m = U[0]
                    ok += int((FA[m] @ v).argmax() == rec[i, m])
                else:
                    a, b = U
                    Z = (FA[a] * v[None, :]) @ FA[b].T
                    ia, ib = np.unravel_index(Z.argmax(), Z.shape)
                    ok += int(ia == rec[i, a] and ib == rec[i, b])
            out[f"vsa|w{w}|k{kap}"] = ok / len(qi)
    return out


def run_k4c(a):
    rng = np.random.default_rng(17)
    res = {}
    for R in [int(x) for x in a.Rs.split(',')]:
      for u in [int(x) for x in a.us.split(',')]:
        for N in [int(x) for x in a.Ns.split(',')]:
            for mul in [int(x) for x in a.bpr.split(',')]:
                tune = [k4c_cell(N, mul, u, rng, R=R) for _ in range(2)]
                rep = [k4c_cell(N, mul, u, rng, R=R) for _ in range(8)]
                fam = {}
                for arm in rep[0]:
                    fam.setdefault(arm.split("|")[0], []).append(arm)
                best = {f: max(arms, key=lambda x: np.mean([t.get(x, 0) for t in tune])) for f, arms in fam.items()}
                per = {f: np.array([r[best[f]] for r in rep]) for f in best}
                cell = {f: {"arm": best[f], "acc": float(per[f].mean())} for f in per}
                sb = max((f for f in per if f != "vsa"), key=lambda f: per[f].mean())
                m, lo, hi = boot_ci(per["vsa"] - per[sb], rng)
                cell["strongest_baseline"] = sb
                cell["vsa_minus_strongest"] = {"mean": m, "ci95": [lo, hi]}
                res[f"R={R}|u={u}|N={N}|bits_per_rec={mul}"] = cell
                print("k4c", R, u, N, mul, {f: round(c["acc"], 3) for f, c in cell.items() if isinstance(c, dict) and "acc" in c}, flush=True)
                json.dump(res, open(a.out, "w"), indent=1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("task", choices=["k2", "k3", "k4", "k4b", "k4c"])
    ap.add_argument("--source", choices=["synth", "locomo"], default="synth")
    ap.add_argument("--locomo", default="/data/locomo10.json")
    ap.add_argument("--Ns", default="100,300,1000,3000")
    ap.add_argument("--Bs", default="65536,262144")
    ap.add_argument("--seeds", type=int, default=10)
    ap.add_argument("--nq", type=int, default=300)
    ap.add_argument("--k3pairs", type=int, default=40)
    ap.add_argument("--Rs", default="6,12")
    ap.add_argument("--us", default="1,2")
    ap.add_argument("--bpr", default="6,12,24,36,48,60,96")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    t0 = time.time()
    {"k2": run_k2, "k3": run_k3, "k4": run_k4, "k4b": run_k4b, "k4c": run_k4c}[a.task](a)
    print("done", time.time() - t0, flush=True)


if __name__ == "__main__":
    main()
