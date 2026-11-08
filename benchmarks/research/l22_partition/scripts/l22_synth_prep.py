"""Synthetic stand-in for l22f_prep.py when the MSTuring-30M-clustered data is not at hand.
Writes the inputs `partition_prefix_acceptance` reads (WARM_L22_ACC_{BASE,QUERIES,GT,CENTRES}):
a Gaussian-mixture fbin (dim 100), 500 queries from the same mixture, exact L2 top-10 GT as
row positions, and 5000 k-means centres of a 262144-row sample (10 Lloyd iterations).
usage: l22_synth_prep.py OUTDIR N [SEED]"""
import json, os, sys, time

import numpy as np

DIM, NQ, K, TRUE_CLUSTERS, SIGMA = 100, 500, 5000, 20000, 0.5
out, n = sys.argv[1], int(sys.argv[2])
seed = int(sys.argv[3]) if len(sys.argv) > 3 else 7
os.makedirs(out, exist_ok=True)
rng = np.random.default_rng(seed)
true_c = rng.standard_normal((TRUE_CLUSTERS, DIM), dtype=np.float32)


def draw(rows, r):
    c = r.integers(0, TRUE_CLUSTERS, rows)
    return true_c[c] + SIGMA * r.standard_normal((rows, DIM), dtype=np.float32)


t0 = time.time()
base_path = f"{out}/base_synth.fbin"
CH = 500_000
with open(base_path, "wb") as f:
    np.array([n, DIM], "<u4").tofile(f)
    for s in range(0, n, CH):
        draw(min(CH, n - s), rng).astype("<f4").tofile(f)
print("base", n, f"{time.time() - t0:.0f}s", flush=True)
q = draw(NQ, np.random.default_rng(seed + 1))
q.astype("<f4").tofile(f"{out}/queries_synth.bin")

mm = np.memmap(base_path, dtype=np.float32, mode="r", offset=8, shape=(n, DIM))
qn = (q * q).sum(1)
best_d = np.full((NQ, 10), np.inf, np.float32)
best_i = np.zeros((NQ, 10), np.int64)
for s in range(0, n, CH):
    blk = np.asarray(mm[s:s + CH])
    d = (blk * blk).sum(1)[None, :] - 2.0 * (q @ blk.T)  # + qn is constant per query
    top = np.argpartition(d, 10, axis=1)[:, :10]
    cd = np.take_along_axis(d, top, 1)
    alld = np.concatenate([best_d, cd], 1)
    alli = np.concatenate([best_i, top + s], 1)
    sel = np.argsort(alld, 1)[:, :10]
    best_d, best_i = np.take_along_axis(alld, sel, 1), np.take_along_axis(alli, sel, 1)
best_i.astype("<i4").tofile(f"{out}/gt_synth_500.bin")
print("gt", f"{time.time() - t0:.0f}s", flush=True)

sample = np.asarray(mm[np.sort(rng.choice(n, min(262144, n), replace=False))])
cent = sample[rng.choice(len(sample), K, replace=False)].copy()
for _ in range(10):
    cn = (cent * cent).sum(1)
    assign = np.empty(len(sample), np.int32)
    for s in range(0, len(sample), 8192):
        blk = sample[s:s + 8192]
        assign[s:s + 8192] = np.argmin(cn[None, :] - 2.0 * (blk @ cent.T), 1)
    sums = np.zeros((K, DIM), np.float64)
    np.add.at(sums, assign, sample)
    cnt = np.bincount(assign, minlength=K)
    ok = cnt > 0
    cent[ok] = (sums[ok] / cnt[ok, None]).astype(np.float32)
cent.astype("<f4").tofile(f"{out}/centres_synth.bin")
with open(f"{out}/meta.json", "w") as f:
    json.dump({"n": n, "dim": DIM, "nq": NQ, "lists": K, "true_clusters": TRUE_CLUSTERS, "sigma": SIGMA, "seed": seed}, f)
print("done", f"{time.time() - t0:.0f}s", flush=True)
