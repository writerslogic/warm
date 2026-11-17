"""Shared helpers for the fact-store kill tests (see PREREG.md)."""
import json
import zlib
import numpy as np
from scipy import sparse
from scipy.stats import rankdata

TUNE_SEEDS = [0, 1]
REPORT_SEEDS = list(range(100, 120))


def auc(pos, neg):
    s = np.concatenate([pos, neg])
    r = rankdata(s)
    n1, n0 = len(pos), len(neg)
    return float((r[:n1].sum() - n1 * (n1 + 1) / 2) / (n1 * n0))


def tpr_at_fpr(pos, neg, fpr=0.01):
    thr = np.quantile(neg, 1 - fpr)
    return float((pos > thr).mean())


def boot_ci(diffs, rng, n=4000):
    d = np.asarray(diffs, dtype=np.float64)
    idx = rng.integers(0, len(d), size=(n, len(d)))
    m = d[idx].mean(axis=1)
    return float(d.mean()), float(np.quantile(m, 0.025)), float(np.quantile(m, 0.975))


_M = np.uint64(0xFFFFFFFFFFFFFFFF)


def mix64(x):
    """splitmix64 finaliser on a uint64 array."""
    with np.errstate(over="ignore"):
        x = x + np.uint64(0x9E3779B97F4A7C15)
        x = (x ^ (x >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
        x = (x ^ (x >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
        return x ^ (x >> np.uint64(31))


def proj(X, P):
    """Sign random projection. X dense (n,d) or csr; P (d,D). Returns +-1 float32."""
    Z = X @ P if not sparse.issparse(X) else np.asarray(X.dot(P))
    return np.where(Z > 0, 1.0, -1.0).astype(np.float32)


def proj_sum(X, P, chunk=256):
    """Sum over items of sign(X P): the unquantised bundle."""
    S = np.zeros(P.shape[1], dtype=np.float32)
    for i in range(0, X.shape[0], chunk):
        S += proj(X[i:i + chunk], P).sum(axis=0)
    return S


def quantise(S, w, kappa, n):
    """Static w-bit quantisation of a bundle sum; clip at kappa*sqrt(n)."""
    if w == 1:
        return np.where(S > 0, 1.0, -1.0).astype(np.float32)
    if w >= 8:
        return S
    c = 2 ** (w - 1) - 1
    lim = kappa * np.sqrt(n)
    return np.clip(np.round(S * (c / lim)), -c, c).astype(np.float32)


def unit(x):
    return x / np.linalg.norm(x, axis=1, keepdims=True)


class Synth:
    name = "synth"
    levels = [0.95, 0.9, 0.8, 0.7]

    def __init__(self, d=128):
        self.d = d

    def sample(self, rng, N, nq):
        d = self.d
        X = unit(rng.standard_normal((N, d)).astype(np.float32))
        mi = rng.choice(N, size=min(N, nq), replace=False)
        nm = unit(rng.standard_normal((nq, d)).astype(np.float32))
        mem = {}
        for rho in self.levels:
            u = unit(rng.standard_normal((len(mi), d)).astype(np.float32))
            mem[rho] = unit(rho * X[mi] + np.sqrt(1 - rho ** 2) * u)
        return X, mi, nm, mem

    def max_n(self):
        return 10 ** 9


class LoCoMoText:
    """Real facts: LoCoMo observation and dialog sentences, hashed char-3gram tf-idf, character-noise queries."""
    name = "locomo"
    levels = [0.1, 0.2, 0.3]

    def __init__(self, path, d=2048):
        data = json.load(open(path))
        texts = []
        for conv in data:
            obs = conv.get("observation", {})
            for sess in obs.values():
                for spk, items in sess.items():
                    for it in items:
                        if isinstance(it, list) and it and isinstance(it[0], str):
                            texts.append(it[0])
            c = conv.get("conversation", {})
            for k, v in c.items():
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
        self.texts = out
        self.d = d
        df = np.zeros(d)
        for t in out:
            df[np.unique(self._idx(t))] += 1
        self.idf = np.log((1 + len(out)) / (1 + df)) + 1.0
        self.mu = None
        self.mu = np.asarray(self.vec(out).mean(axis=0)).ravel().astype(np.float32)  # shared encoder centring, all arms

    def _idx(self, t):
        t = " " + t.lower() + " "
        return np.array([zlib.crc32(t[i:i + 3].encode()) % self.d for i in range(len(t) - 2)], dtype=np.int64)

    def vec(self, texts):
        rows, cols, vals = [], [], []
        for r, t in enumerate(texts):
            ix = self._idx(t)
            u, c = np.unique(ix, return_counts=True)
            v = (1 + np.log(c)) * self.idf[u]
            v = v / np.linalg.norm(v)
            rows += [r] * len(u)
            cols += u.tolist()
            vals += v.tolist()
        M = sparse.csr_matrix((vals, (rows, cols)), shape=(len(texts), self.d), dtype=np.float32)
        if getattr(self, "mu", None) is None:
            return M
        Z = M.toarray() - self.mu
        return (Z / np.linalg.norm(Z, axis=1, keepdims=True)).astype(np.float32)

    @staticmethod
    def corrupt(t, eps, rng):
        al = "abcdefghijklmnopqrstuvwxyz "
        ch = list(t)
        for i in range(len(ch)):
            if rng.random() < eps:
                ch[i] = al[rng.integers(0, len(al))]
        return "".join(ch)

    def max_n(self):
        return len(self.texts) - 600

    def sample(self, rng, N, nq):
        order = rng.permutation(len(self.texts))
        st = [self.texts[i] for i in order[:N]]
        nmt = [self.texts[i] for i in order[N:N + nq]]
        X = self.vec(st)
        mi = rng.choice(N, size=min(N, nq), replace=False)
        nm = self.vec(nmt)
        mem = {eps: self.vec([self.corrupt(st[i], eps, rng) for i in mi]) for eps in self.levels}
        return X, mi, nm, mem
