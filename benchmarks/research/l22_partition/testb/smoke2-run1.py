"""PartitionIndex smoke test: brute force before training, training on the first search with
train_at live rows, inserts and deletes after training, recall vs exact on live rows."""
import numpy as np
from writerslogic_warm import PartitionIndex

rng = np.random.default_rng(3)
dim, n = 100, 24000
cent = rng.normal(0, 4, (40, dim)).astype(np.float32)
X = (cent[rng.integers(0, 40, n)] + rng.normal(0, 1, (n, dim))).astype(np.float32)
Q = (cent[rng.integers(0, 40, 200)] + rng.normal(0, 1, (200, dim))).astype(np.float32)
ix = PartitionIndex.create(dim, lists=64, train_at=8000, nprobe=24, reorder=200, threads=4)


def exact(live, k=10):
    ids = np.array(sorted(live))
    d = ((Q[:, None, :] - X[ids][None, :, :]) ** 2).sum(-1)
    return ids[np.argsort(d, axis=1)[:, :k]]


def recall(got, gt):
    return np.mean([len(set(g) & set(t)) / len(t) for g, t in zip(got, gt)])


live = set()
ix.insert(np.arange(0, 6000), X[:6000]); live |= set(range(6000))
r0 = recall(ix.search(Q, 10), exact(live))
assert not ix.stats()["trained"] and r0 == 1.0, ("brute force", r0)
ix.insert(np.arange(6000, 12000), X[6000:12000]); live |= set(range(6000, 12000))
r1 = recall(ix.search(Q, 10), exact(live))
st = ix.stats()
assert st["trained"] and st["trained_at_live"] == 12000, st
ix.delete(np.arange(0, 4000)); live -= set(range(4000))
ix.insert(np.arange(12000, 24000), X[12000:24000]); live |= set(range(12000, 24000))
got = ix.search(Q, 10)
assert not np.isin(got, np.arange(0, 4000)).any(), "deleted id returned"
r2 = recall(got, exact(live))
assert len(ix) == len(live), (len(ix), len(live))
small = PartitionIndex.create(dim, threads=1)
small.insert(np.arange(3), X[:3])
pad = small.search(Q[:2], 5)
assert pad.shape == (2, 5) and (pad[:, 3:] == -1).all(), pad
try:
    ix.insert(np.array([20000]), X[:1]); raise SystemExit("live id accepted")
except ValueError:
    pass
print(f"top10 recall pre-train {r0:.3f} at-train {r1:.3f} after-churn {r2:.3f}")
print("stats", ix.stats())
assert r1 > 0.9 and r2 > 0.9, (r1, r2)
print("SMOKE OK")
