"""K1: near-key membership at equal bits. VSA bundle vs stored signatures vs distance-sensitive Bloom."""
import argparse
import itertools
import json
import sys
import time

import numpy as np

from common import (Synth, LoCoMoText, TUNE_SEEDS, REPORT_SEEDS, auc, boot_ci, mix64,
                    proj, proj_sum, quantise, tpr_at_fpr)

WS = [1, 2, 3, 4, 8]
KAPPAS = [1.0, 2.0, 3.0]
FRACS = [1.0, 0.5, 0.25, 0.125]
DS_R = [6, 9, 12, 16]
DS_L = [8, 24, 64]
DS_K = [1, 3, 8]
NBAND = 1024


def bands(code, r, L):
    """code (n, >=r*L) +-1 -> uint64 tokens (n, L)."""
    n = code.shape[0]
    b = (code[:, : r * L] > 0).reshape(n, L, r).astype(np.uint64)
    w = (np.uint64(1) << np.arange(r, dtype=np.uint64))
    tok = (b * w).sum(axis=2)
    return tok + (np.arange(L, dtype=np.uint64) << np.uint64(r))


def positions(tok, k, nbits):
    h1 = mix64(tok)
    h2 = mix64(h1) | np.uint64(1)
    j = np.arange(k, dtype=np.uint64)
    return ((h1[..., None] + h2[..., None] * j) % np.uint64(nbits)).astype(np.int64)


def dsbf_scores(store_code, q_code, nbits, r, L, k):
    bl = np.zeros(nbits, dtype=bool)
    bl[positions(bands(store_code, r, L), k, nbits).ravel()] = True
    hit = bl[positions(bands(q_code, r, L), k, nbits)].all(axis=2)
    return hit.sum(axis=1) / L


def sig_scores(sig, qc):
    out = np.empty(qc.shape[0], dtype=np.float32)
    for i in range(0, qc.shape[0], 128):
        out[i:i + 128] = (qc[i:i + 128] @ sig.T).max(axis=1)
    return out


def one_seed(src, seed, N, Bs, nq, streaming=False):
    rng = np.random.default_rng(seed * 1000003 + N)
    X, mi, nm, mem = src.sample(rng, N, nq)
    Bmax = max(Bs)
    P = rng.standard_normal((src.d, max(Bmax, NBAND))).astype(np.float32)
    S_full = proj_sum(X, P[:, :Bmax])
    levels = list(mem.keys())
    qs = {"nm": nm}
    qs.update({("m", lv): mem[lv] for lv in levels})
    Qc = {k: proj(v, P) for k, v in qs.items()}  # (nq, Bmax+) codes
    Xc_band = proj(X, P[:, :NBAND])
    res = {}  # (B, level, arm_key) -> (auc, tpr)

    def record(B, lv, arm, pos, neg):
        res[(B, lv, arm)] = (auc(pos, neg), tpr_at_fpr(pos, neg))

    for B in Bs:
        # VSA
        for w in WS:
            D = B // w
            for kap in ([1.0] if w in (1, 8) else KAPPAS):
                Sq = quantise(S_full[:D], w, kap, N)
                neg = Qc["nm"][:, :D] @ Sq
                for lv in levels:
                    pos = Qc[("m", lv)][:, :D] @ Sq
                    record(B, lv, f"vsa|w{w}|k{kap}", pos, neg)
        # stored signatures, fraction of items kept
        for f in FRACS:
            nk = max(1, int(N * f))
            m = B // nk
            if m < 4:
                continue
            m = min(m, Bmax)
            keep = np.arange(nk)
            sig = proj(X[keep], P[:, :m])
            neg = sig_scores(sig, Qc["nm"][:, :m])
            for lv in levels:
                pos = sig_scores(sig, Qc[("m", lv)][:, :m])
                # members whose item was dropped are queried but cannot match; keep them in pos
                record(B, lv, f"sig|f{f}", pos, neg)
        # distance-sensitive Bloom and counting variant
        for tag, nb in (("dsbf", B), ("dsbfc", B // 4)):
            for r, L, k in itertools.product(DS_R, DS_L, DS_K):
                if r * L > NBAND:
                    continue
                neg = dsbf_scores(Xc_band, Qc["nm"][:, :NBAND], nb, r, L, k)
                for lv in levels:
                    pos = dsbf_scores(Xc_band, Qc[("m", lv)][:, :NBAND], nb, r, L, k)
                    record(B, lv, f"{tag}|r{r}|L{L}|k{k}", pos, neg)
    if streaming:
        for B in Bs:
            for w in (2, 3, 4):
                D = B // w
                c = 2 ** (w - 1) - 1
                Sq = np.zeros(D, dtype=np.float32)
                for i0 in range(0, N, 256):
                    Z = proj(X[i0:i0 + 256], P[:, :D])
                    for row in Z:
                        Sq = np.clip(Sq + row, -c, c)
                neg = Qc["nm"][:, :D] @ Sq
                for lv in levels:
                    pos = Qc[("m", lv)][:, :D] @ Sq
                    record(B, lv, f"vsastream|w{w}|k0", pos, neg)
    return res


def arm_family(a):
    return a.split("|")[0]


def summarise(allres, Bs, levels):
    """allres: seed -> res dict. Tune on TUNE_SEEDS, report on REPORT_SEEDS."""
    out = {}
    tune = [s for s in allres if s in TUNE_SEEDS]
    rep = [s for s in allres if s not in TUNE_SEEDS]
    rng = np.random.default_rng(7)
    for B in Bs:
        for lv in levels:
            arms = sorted({a for (b, l, a) in allres[rep[0]] if b == B and l == lv})
            fams = sorted({arm_family(a) for a in arms})
            best = {}
            for fam in fams:
                cand = [a for a in arms if arm_family(a) == fam]
                sc = {a: np.mean([allres[s][(B, lv, a)][0] for s in tune]) for a in cand}
                best[fam] = max(sc, key=sc.get)
            per = {}
            for fam, a in best.items():
                aucs = np.array([allres[s][(B, lv, a)][0] for s in rep])
                tprs = np.array([allres[s][(B, lv, a)][1] for s in rep])
                per[fam] = {"arm": a, "auc": float(aucs.mean()), "tpr1": float(tprs.mean()), "aucs": aucs.tolist()}
            base = [f for f in per if f in ("sig", "dsbf", "dsbfc")]
            strongest = max(base, key=lambda f: per[f]["auc"])
            cell = {
                "arms": {f: {k: v for k, v in p.items() if k != "aucs"} for f, p in per.items()},
                "strongest_baseline": strongest,
            }
            for fam in ("vsa", "vsastream"):
                if fam not in per:
                    continue
                d = np.array(per[fam]["aucs"]) - np.array(per[strongest]["aucs"])
                mean, lo, hi = boot_ci(d, rng)
                dt = np.array([allres[s][(B, lv, per[fam]["arm"])][1] for s in rep]) - np.array(
                    [allres[s][(B, lv, per[strongest]["arm"])][1] for s in rep])
                tm, tlo, thi = boot_ci(dt, rng)
                cell[f"{fam}_minus_strongest_auc"] = {"mean": mean, "ci95": [lo, hi]}
                cell[f"{fam}_minus_strongest_tpr1"] = {"mean": tm, "ci95": [tlo, thi]}
            out[f"B={B}|level={lv}"] = cell
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--source", choices=["synth", "locomo"], default="synth")
    ap.add_argument("--locomo", default="/data/locomo10.json")
    ap.add_argument("--Ns", default="100,300,1000,3000,10000,30000")
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--nq", type=int, default=400)
    ap.add_argument("--stream-maxN", type=int, default=3000)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    src = Synth() if a.source == "synth" else LoCoMoText(a.locomo)
    seeds = TUNE_SEEDS + REPORT_SEEDS[: a.seeds]
    results = {}
    for N in [int(x) for x in a.Ns.split(",")]:
        if N > src.max_n():
            print(f"skip N={N} > max {src.max_n()}", flush=True)
            continue
        Bs = [B for B in (2 ** 14, 2 ** 16, 2 ** 18)
              if (B == 2 ** 16) or (B == 2 ** 14 and N <= 3000) or (B == 2 ** 18 and N >= 3000)]
        t0 = time.time()
        allres = {s: one_seed(src, s, N, Bs, a.nq, streaming=N <= a.stream_maxN) for s in seeds}
        results[str(N)] = summarise(allres, Bs, src.levels)
        print(f"N={N} done {time.time() - t0:.0f}s", flush=True)
        json.dump({"source": src.name, "seeds": seeds, "levels": src.levels, "results": results},
                  open(a.out, "w"), indent=1)


if __name__ == "__main__":
    main()
