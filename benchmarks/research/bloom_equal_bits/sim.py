"""Equal-bits check of PREREGISTRATION-binding-readout.md section 19 (counting bundle vs binary Bloom).
Arms (membership AUC, members vs non-members, 20 seeds, 200+200 queries):
  count:   D=16384 u32 counters (524,288 bits), k=64, Poisson z-sum readout (the section 19 arm)
  bin_k64: D=16384 bits, k=64, corrected containment (the section 19 incumbent; matched D, NOT matched bits)
  bin_kopt:D=16384 bits, k=round(ln2*D/n) (matched D, optimal k)
  bloom_eq:D=524288 bits (matched BITS to the counting arm), k=round(ln2*m/n), plain containment
"""
import json, sys
import numpy as np
D = 16384
BITS = D * 32
NS = [700, 1000, 1500, 2000, 3000, 4000, 6000, 8000]
SEEDS = 20

def auc(pos, neg):
    allv = np.concatenate([pos, neg]); ranks = allv.argsort().argsort() + 1.0
    # average ranks for ties
    _, inv, cnt = np.unique(allv, return_inverse=True, return_counts=True)
    sums = np.zeros(len(cnt)); np.add.at(sums, inv, ranks); avg = (sums / cnt)[inv]
    return (avg[:len(pos)].sum() - len(pos) * (len(pos) + 1) / 2) / (len(pos) * len(neg))

def subsets(rng, m, k, n):
    return np.stack([rng.choice(m, k, replace=False) for _ in range(n)])

def run(n, seed):
    rng = np.random.default_rng(seed)
    out = {}
    # counting arm and bin_k64 share the item set
    k = 64
    items = subsets(rng, D, k, n); cnt = np.zeros(D); np.add.at(cnt, items.ravel(), 1)
    members = items[rng.choice(n, 200, replace=False)]; non = subsets(rng, D, k, 200)
    lam = n * k / D
    z = lambda Q: ((cnt[Q] - lam) / np.sqrt(lam)).sum(axis=1)
    out["count"] = auc(z(members), z(non))
    union = cnt > 0; d = union.mean()
    cont = lambda Q: (union[Q].mean(axis=1) - d) / (1 - d)
    out["bin_k64"] = auc(cont(members), cont(non))
    for name, m, kk in (("bin_kopt", D, max(1, round(np.log(2) * D / n))), ("bloom_eq", BITS, max(1, round(np.log(2) * BITS / n)))):
        it = subsets(rng, m, kk, n); u = np.zeros(m, bool); u[it.ravel()] = True
        mem = it[rng.choice(n, 200, replace=False)]; nm = subsets(rng, m, kk, 200)
        f = lambda Q: u[Q].mean(axis=1)
        out[name] = auc(f(mem), f(nm)); out[name + "_k"] = int(kk)
    return out

res = {}
for n in NS:
    runs = [run(n, s) for s in range(SEEDS)]
    res[n] = {k: float(np.mean([r[k] for r in runs])) for k in runs[0]}
    print(n, {k: round(v, 4) if not k.endswith("_k") else int(v) for k, v in res[n].items()}, flush=True)
with open(sys.argv[1], "w") as out:
    json.dump({"D": D, "bits_counting_arm": BITS, "seeds": SEEDS, "auc_mean": res}, out, indent=1)
