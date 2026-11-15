"""Round 2, Lead B: real-text near-key membership at equal bits. See PREREG.md (Round 2, Lead B).

Stage 'tune' runs the tuning seeds with every configuration and writes the per-cell selection;
stage 'report' runs the listed report seeds with only the selected configurations.
Metrics per (config, N, noise type, level): protocol M (noise-matched negatives), protocol C (clean negatives),
control K (corrupted non-members against clean non-members), each AUC / TPR@1% / TPR@0.1%.
"""
import argparse
import itertools
import json
import os
import re
import sys
import time
import zlib

import numpy as np
from scipy import sparse

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
from common import auc, mix64, quantise, tpr_at_fpr  # noqa: E402

DIM = 2048
KINDS = ["c3", "c5", "w1", "w2"]
ENCS = {"c3": ["c3"], "c5": ["c5"], "w12": ["w1", "w2"]}
NOISES = [("sub", 0.1), ("sub", 0.2), ("sub", 0.3), ("drop", 0.2), ("drop", 0.4), ("drop", 0.6)]
NS = [100, 300, 1000, 3000]
WS = [1, 2, 3, 4, 8]
KAPPAS = [1.0, 2.0, 3.0]
FRACS = [1.0, 0.5, 0.25, 0.125]
NBAND = 1024
PRIME = (1 << 31) - 1
ALPHA = "abcdefghijklmnopqrstuvwxyz "


# ----------------------------------------------------------------------------- corpora
def load_locomo(path):
    data = json.load(open(path))
    texts = []
    for conv in data:
        for sess in conv.get("observation", {}).values():
            for items in sess.values():
                for it in items:
                    if isinstance(it, list) and it and isinstance(it[0], str):
                        texts.append(it[0])
        for v in conv.get("conversation", {}).values():
            if isinstance(v, list):
                for turn in v:
                    if isinstance(turn, dict) and isinstance(turn.get("text"), str):
                        texts.append(turn["text"])
    seen, out = set(), []
    for t in texts:
        t = " ".join(t.split())
        if len(t) >= 30 and t not in seen:
            seen.add(t)
            out.append(t)
    return out


def load_lme(path, pool=12000):
    docs = json.load(open(path))["documents"]
    seen, out = set(), []
    for d in docs:
        parts = re.split(r"\b(user|assistant): ", d["text"])
        for role, body in zip(parts[1::2], parts[2::2]):
            if role != "user":
                continue
            for s in re.split(r"(?<=[.!?])\s+", " ".join(body.split())):
                if 30 <= len(s) <= 300 and s not in seen:
                    seen.add(s)
                    out.append(s)
    rs = np.random.RandomState(12345)
    ix = rs.permutation(len(out))[:pool]
    return [out[i] for i in ix]


def raw_shingles(text, kind):
    t = text.lower()
    if kind in ("c3", "c5"):
        n = 3 if kind == "c3" else 5
        s = " " + t + " "
        toks = [s[i:i + n] for i in range(max(0, len(s) - n + 1))]
    else:
        w = t.split()
        toks = w if kind == "w1" else [a + " " + b for a, b in zip(w, w[1:])]
    return np.array([zlib.crc32(x.encode()) for x in toks], dtype=np.uint32)


def corrupt(text, typ, lvl, rng):
    if typ == "sub":
        ch = list(text)
        hit = rng.random(len(ch)) < lvl
        for i in np.flatnonzero(hit):
            ch[i] = ALPHA[rng.integers(len(ALPHA))]
        return "".join(ch)
    w = text.split()
    keep = rng.random(len(w)) >= lvl
    if not keep.any():
        keep[rng.integers(len(w))] = True
    return " ".join(x for x, k in zip(w, keep) if k)


class Encoder:
    """Hashed log-TF x IDF, unit norm, optionally centred on the corpus mean (applied in code space)."""

    def __init__(self, name, pool):
        self.name = name
        self.kinds = ENCS[name]
        df = np.zeros(DIM)
        for t in pool:
            df[np.unique(self.idx(t))] += 1
        self.idf = np.log((1 + len(pool)) / (1 + df)) + 1.0
        self.mu = np.asarray(self.vec_rows([self.idx(t) for t in pool]).mean(axis=0)).ravel().astype(np.float32)

    def idx(self, t):
        return np.concatenate([raw_shingles(t, k) for k in self.kinds]) % DIM

    def vec_rows(self, idxs):
        rows, cols, vals = [], [], []
        for r, ix in enumerate(idxs):
            if len(ix) == 0:
                ix = np.array([0])
            u, c = np.unique(ix, return_counts=True)
            v = (1 + np.log(c)) * self.idf[u]
            v = v / np.linalg.norm(v)
            rows += [r] * len(u)
            cols += u.tolist()
            vals += v.tolist()
        return sparse.csr_matrix((vals, (rows, cols)), shape=(len(idxs), DIM), dtype=np.float32)

    def vec(self, texts):
        return self.vec_rows([self.idx(t) for t in texts])


# ----------------------------------------------------------------------------- hashing structures
def tok64(h, salt):
    return mix64(h.astype(np.uint64) + np.uint64(salt))


def bloom_pos(tok, k, nbits):
    h1 = mix64(tok)
    h2 = mix64(h1) | np.uint64(1)
    j = np.arange(k, dtype=np.uint64)
    return ((h1[..., None] + h2[..., None] * j) % np.uint64(nbits)).astype(np.int64)


def bands(code, r, L):
    n = code.shape[0]
    b = (code[:, : r * L] > 0).reshape(n, L, r).astype(np.uint64)
    return (b * (np.uint64(1) << np.arange(r, dtype=np.uint64))).sum(axis=2) + (np.arange(L, dtype=np.uint64) << np.uint64(r))


def mh_params(K, seed=99):
    rs = np.random.RandomState(seed)
    a = (rs.randint(1, PRIME, size=K).astype(np.uint64) | np.uint64(1)) % np.uint64(PRIME)
    b = rs.randint(0, PRIME, size=K).astype(np.uint64)
    return a, b


def minhash(tok_list, K, a, b):
    """tok_list: list of unique uint32 arrays. Returns (n,K) uint32 minima of (a*h+b) mod p."""
    out = np.full((len(tok_list), K), PRIME, dtype=np.uint64)
    for i, t in enumerate(tok_list):
        if len(t):
            h = (t.astype(np.uint64) % np.uint64(PRIME))[:, None]
            out[i] = ((a[None, :K] * h + b[None, :K]) % np.uint64(PRIME)).min(axis=0)
    return out.astype(np.uint32)


# ----------------------------------------------------------------------------- seed run
class Run:
    def __init__(self, corpus, encs, seed, a, wanted):
        self.a, self.wanted = a, wanted
        self.encs = encs
        self.pool = corpus
        rng = np.random.default_rng(seed * 7919 + 17)
        self.rng = rng
        perm = rng.permutation(len(corpus))
        self.store_all = [corpus[i] for i in perm[:max(a.Ns)]]
        self.negs = [corpus[i] for i in perm[max(a.Ns):max(a.Ns) + a.nneg]]
        Dm = a.dmax
        self.Dm = Dm
        self.P = rng.standard_normal((DIM, Dm)).astype(np.float32)
        self.muP = {e: (enc.mu @ self.P).astype(np.float32) for e, enc in encs.items()}
        self.cache = {}

    def want(self, key):
        return self.wanted is None or key in self.wanted

    def shingles(self, text_id, text, kind):
        k = (text_id, kind)
        if k not in self.cache:
            self.cache[k] = np.unique(raw_shingles(text, kind))
        return self.cache[k]

    # ---- stores
    def build_store(self, N):
        a = self.a
        texts = self.store_all[:N]
        st = {"N": N, "cfg": {}}
        # projection-based: vsa, sig, dsbf
        for e, enc in self.encs.items():
            X = enc.vec(texts)
            for cen in (0, 1):
                S = np.zeros(self.Dm, np.float32)
                sigs = {}
                band = None
                for i0 in range(0, N, 128):
                    Z = np.asarray(X[i0:i0 + 128].dot(self.P))
                    if cen:
                        Z -= self.muP[e]
                    C = np.where(Z > 0, 1.0, -1.0).astype(np.float32)
                    S += C.sum(0)
                    sigs[i0] = C[:, :max(1, max(B // max(1, int(N * min(FRACS))) for B in a.Bs))]
                    if e == "c3" and cen == 1 and i0 == 0:
                        band = []
                    if band is not None:
                        band.append(C[:, :NBAND])
                C_first = np.concatenate([sigs[k] for k in sorted(sigs)], 0)
                for B in a.Bs:
                    for w in WS:
                        for kap in ([1.0] if w in (1, 8) else KAPPAS):
                            key = ("vsa", e, cen, B, w, kap)
                            if self.want(key):
                                D = B // w
                                sq = np.zeros(self.Dm, np.float32)
                                sq[:D] = quantise(S[:D], w, kap, N) if w > 1 else np.where(S[:D] > 0, 1.0, -1.0)
                                st["cfg"][key] = ("vsa", sq)
                    for f in FRACS:
                        nk = max(1, int(N * f))
                        m = B // nk
                        key = ("sig", e, cen, B, f)
                        if m >= 4 and self.want(key):
                            m = min(m, C_first.shape[1])
                            st["cfg"][key] = ("sig", C_first[:nk, :m].copy())
                if band is not None:
                    band = np.concatenate(band, 0)
                    for B in a.Bs:
                        for tag, nb in (("dsbf", B), ("dsbfc", B // 4)):
                            for r, L, k in itertools.product([9, 12, 16], [24, 64], [1, 3]):
                                key = (tag, B, r, L, k)
                                if self.want(key):
                                    bl = np.zeros(nb, dtype=bool)
                                    bl[bloom_pos(bands(band, r, L), k, nb).ravel()] = True
                                    st["cfg"][key] = ("dsbf", bl)
        # shingle structures
        toks = {kind: [self.shingles(("s", i), t, kind) for i, t in enumerate(texts)] for kind in KINDS}
        for B in a.Bs:
            for kind in KINDS:
                for k in (1, 2, 4, 8):
                    key = ("sbfg", B, kind, k)
                    if self.want(key):
                        bl = np.zeros(B, dtype=bool)
                        allt = np.concatenate(toks[kind]) if toks[kind] else np.zeros(0, np.uint32)
                        bl[bloom_pos(tok64(allt, 1), k, B).ravel()] = True
                        st["cfg"][key] = ("sbfg", bl)
            for kind in ("c5", "w2"):
                m = B // N
                for k in (1, 2, 3):
                    key = ("sbfr", B, kind, k)
                    if m >= 4 and self.want(key):
                        bt = np.zeros((m, N), dtype=bool)
                        for i, t in enumerate(toks[kind]):
                            if len(t):
                                bt[bloom_pos(tok64(t, 2), k, m).ravel(), i] = True
                        st["cfg"][key] = ("sbfr", bt)
                for w in (2, 4, 8):
                    for rows in (1, 2, 4):
                        key = ("cms", B, kind, w, rows)
                        if self.want(key):
                            D = B // w // rows
                            allt = np.concatenate(toks[kind]) if toks[kind] else np.zeros(0, np.uint32)
                            cs = []
                            for j in range(rows):
                                pos = (mix64(tok64(allt, 10 + j)) % np.uint64(D)).astype(np.int64)
                                cs.append(np.minimum(np.bincount(pos, minlength=D), 2 ** w - 1).astype(np.int32))
                            st["cfg"][key] = ("cms", cs)
        mhK = max([B // N for B in a.Bs] + [1])
        a_, b_ = mh_params(mhK)
        st["mh"] = {}
        for kind in ("c5", "w1", "w2"):
            if any(self.want(("mh", B, kind, b)) for B in a.Bs for b in (1, 2, 4, 8)):
                st["mh"][kind] = minhash(toks[kind], mhK, a_, b_)
        st["mhab"] = (a_, b_, mhK)
        return st

    # ---- scoring of one query set against one or more stores (codes are computed once per chunk)
    def score(self, sts, texts, tid):
        n = len(texts)
        outs = {N: {} for N in sts}
        sh = {kind: [self.shingles((tid, i), t, kind) for i, t in enumerate(texts)] for kind in KINDS}
        groups = {}
        for N, st in sts.items():
            for key, (typ, obj) in st["cfg"].items():
                if typ == "vsa":
                    g = groups.setdefault((key[1], key[2]), {"vsa": [], "ps": []})
                    g["vsa"].append((N, key, obj))
                elif typ == "sig":
                    groups.setdefault((key[1], key[2]), {"vsa": [], "ps": []})["ps"].append((N, key))
                elif typ == "dsbf":
                    groups.setdefault(("c3", 1), {"vsa": [], "ps": []})["ps"].append((N, key))
        acc = {}
        for g in groups.values():
            g["SQ"] = np.stack([o for _, _, o in g["vsa"]], 1) if g["vsa"] else None
            g["res"] = np.zeros((n, len(g["vsa"])), np.float32)
            for nk in g["ps"]:
                acc[nk] = []
        for e, enc in self.encs.items():
            if not any(k[0] == e for k in groups):
                continue
            for i0 in range(0, n, 250):
                Z = np.asarray(enc.vec(texts[i0:i0 + 250]).dot(self.P))
                for cen in (0, 1):
                    if (e, cen) not in groups:
                        continue
                    g = groups[(e, cen)]
                    if cen:
                        Z -= self.muP[e]
                    C = np.where(Z > 0, 1.0, -1.0).astype(np.float32)
                    if g["SQ"] is not None:
                        g["res"][i0:i0 + 250] = C @ g["SQ"]
                    for (N, k) in g["ps"]:
                        typ, obj = sts[N]["cfg"][k]
                        if typ == "sig":
                            acc[(N, k)].append((C[:, :obj.shape[1]] @ obj.T).max(1))
                        else:
                            _, B, r, L, kk = k
                            nb = B if k[0] == "dsbf" else B // 4
                            pos = bloom_pos(bands(C[:, :NBAND], r, L), kk, nb)
                            acc[(N, k)].append(obj[pos].all(axis=2).sum(1) / L)
        for g in groups.values():
            for j, (N, key, _) in enumerate(g["vsa"]):
                outs[N][key] = g["res"][:, j]
        for (N, k), v in acc.items():
            outs[N][k] = np.concatenate(v)
        for N, st in sts.items():
            out = outs[N]
            for key, (typ, obj) in st["cfg"].items():
                if typ not in ("sbfg", "cms", "sbfr"):
                    continue
                toks = sh[key[2]]
                lens = np.array([len(t) for t in toks])
                allt = np.concatenate(toks) if lens.sum() else np.zeros(0, np.uint32)
                off = np.concatenate([[0], np.cumsum(lens)[:-1]])
                if typ == "sbfg":
                    hit = obj[bloom_pos(tok64(allt, 1), key[3], len(obj))].all(axis=1).astype(np.float64)
                    sc = np.add.reduceat(hit, off) / np.maximum(lens, 1) if len(hit) else np.zeros(n)
                    sc[lens == 0] = 0
                    out[key] = sc
                elif typ == "cms":
                    est = None
                    for j, c in enumerate(obj):
                        v = c[(mix64(tok64(allt, 10 + j)) % np.uint64(len(c))).astype(np.int64)]
                        est = v if est is None else np.minimum(est, v)
                    sc = np.add.reduceat(est.astype(np.float64), off) / np.maximum(lens, 1) if len(est) else np.zeros(n)
                    sc[lens == 0] = 0
                    out[key] = sc
                else:
                    sc = np.zeros(n)
                    for i, t in enumerate(toks):
                        if len(t):
                            p = bloom_pos(tok64(t, 2), key[3], obj.shape[0])
                            sc[i] = obj[p].all(axis=1).sum(axis=0).max() / len(t)
                    out[key] = sc
            a_, b_, mhK = st["mhab"]
            N = st["N"]
            for kind, ssig in st["mh"].items():
                qsig = minhash(sh[kind], mhK, a_, b_)
                for b in (1, 2, 4, 8):
                    for B in self.a.Bs:
                        key = ("mh", B, kind, b)
                        K = B // (N * b)
                        if K >= 1 and self.want(key):
                            mask = np.uint32((1 << b) - 1)
                            sc = np.zeros(n)
                            s_ = ssig[:, :K] & mask
                            for i0 in range(0, n, 32):
                                q = qsig[i0:i0 + 32, :K] & mask
                                sc[i0:i0 + 32] = (q[:, None, :] == s_[None, :, :]).mean(axis=2).max(axis=1)
                            out[key] = sc
        return outs


def metrics(pos, neg):
    return [auc(pos, neg), tpr_at_fpr(pos, neg, 0.01), tpr_at_fpr(pos, neg, 0.001)]


def run_seed(src, corpus, encs, seed, a, wanted):
    t0 = time.time()
    R = Run(corpus, encs, seed, a, wanted)
    stores = {N: R.build_store(N) for N in a.Ns}
    print(f"  seed {seed} stores {time.time() - t0:.0f}s", flush=True)
    rng = R.rng
    npos = a.npos
    res = {}  # (cfg, N, typ, lvl) -> [aucM,t1M,t01M, aucC,t1C,t01C, aucK,t1K,t01K]
    negs_clean = {}
    negs_noisy = {}
    for typ, lvl in NOISES:
        nt = [corrupt(t, typ, lvl, rng) for t in R.negs]
        negs_noisy[(typ, lvl)] = nt
    clean = R.score(stores, R.negs, ("nc",))
    noisy = {}
    for (typ, lvl), nt in negs_noisy.items():
        o = R.score(stores, nt, ("nn", typ, lvl))
        for N in a.Ns:
            noisy[(N, typ, lvl)] = o[N]
    print(f"  seed {seed} negatives {time.time() - t0:.0f}s", flush=True)
    for N in a.Ns:
        mem = rng.integers(0, N, npos)
        for typ, lvl in NOISES:
            pt = [corrupt(R.store_all[i], typ, lvl, rng) for i in mem]
            ps = R.score({N: stores[N]}, pt, ("p", N, typ, lvl))[N]
            for cfg in ps:
                row = metrics(ps[cfg], noisy[(N, typ, lvl)][cfg]) + metrics(ps[cfg], clean[N][cfg]) + \
                    metrics(noisy[(N, typ, lvl)][cfg], clean[N][cfg])
                res[json.dumps([cfg, N, typ, lvl])] = row
        print(f"  seed {seed} N={N} done {time.time() - t0:.0f}s", flush=True)
    return res


def selftest(a):
    rng = np.random.default_rng(0)
    # Bloom FPR vs theory
    n, k, B = 2000, 4, 2 ** 15
    keys = rng.integers(0, 2 ** 32, n, dtype=np.uint64)
    bl = np.zeros(B, dtype=bool)
    bl[bloom_pos(tok64(keys, 1), k, B).ravel()] = True
    probe = rng.integers(0, 2 ** 32, 400000, dtype=np.uint64)
    emp = bl[bloom_pos(tok64(probe, 1), k, B)].all(axis=1).mean()
    th = (1 - np.exp(-k * n / B)) ** k
    se = np.sqrt(th * (1 - th) / 400000)
    print("bloom fpr emp %.5f theory %.5f se %.5f ok=%s" % (emp, th, se, abs(emp - th) < 4 * se), flush=True)
    # MinHash vs exact Jaccard
    K = 2048
    a_, b_ = mh_params(K)
    errs = []
    for _ in range(40):
        A = rng.choice(10000, 80, replace=False).astype(np.uint32)
        Bs = np.concatenate([A[:rng.integers(20, 80)], rng.choice(np.arange(10000, 20000), 40, replace=False).astype(np.uint32)])
        j = len(np.intersect1d(A, Bs)) / len(np.union1d(A, Bs))
        sa, sb = minhash([np.unique(A), np.unique(Bs)], K, a_, b_)
        errs.append((sa == sb).mean() - j)
    print("minhash bias %.4f rms %.4f ok=%s" % (np.mean(errs), np.sqrt(np.mean(np.square(errs))), abs(np.mean(errs)) < 0.01), flush=True)
    t = "the quick brown fox jumps over the lazy dog again and again"
    ok = all(len(raw_shingles(t, k)) > 0 for k in KINDS) and (raw_shingles(t, "c3") == raw_shingles(t, "c3")).all()
    print("shingle copy agreement ok=%s" % ok, flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stage", choices=["selftest", "tune", "report"])
    ap.add_argument("--source", choices=["locomo", "lme"], default="locomo")
    ap.add_argument("--locomo", default="/home/davidcondrey/locomo10.json")
    ap.add_argument("--lme", default="/home/davidcondrey/lme_input.json")
    ap.add_argument("--Ns", type=lambda s: [int(x) for x in s.split(",")], default=NS)
    ap.add_argument("--Bs", type=lambda s: [int(x) for x in s.split(",")], default=[2 ** 14, 2 ** 16])
    ap.add_argument("--seeds", type=lambda s: [int(x) for x in s.split(",")], default=[0, 1])
    ap.add_argument("--nneg", type=int, default=4000)
    ap.add_argument("--npos", type=int, default=1000)
    ap.add_argument("--wanted")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    a.dmax = max(a.Bs) if max(a.Bs) <= 2 ** 16 else max(a.Bs)
    if a.stage == "selftest":
        return selftest(a)
    corpus = load_locomo(a.locomo) if a.source == "locomo" else load_lme(a.lme)
    print(f"corpus {a.source} n={len(corpus)}", flush=True)
    encs = {e: Encoder(e, corpus) for e in ENCS}
    wanted = None
    if a.stage == "report":
        wanted = {tuple(json.loads(x)) if False else tuple(x) for x in json.load(open(a.wanted))}
    allres = {}
    if os.path.exists(a.out):
        allres = json.load(open(a.out))
    for s in a.seeds:
        if str(s) in allres:
            continue
        allres[str(s)] = run_seed(a.source, corpus, encs, s, a, wanted)
        json.dump(allres, open(a.out, "w"))


if __name__ == "__main__":
    main()
