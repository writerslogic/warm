"""Held-out split: 10,000 random train rows become the queries and leave the index. Seeded.
Writes data/<name>-heldout.hdf5 in ann-benchmarks layout (train, test, neighbors, distances)."""
import os, sys, h5py, numpy as np
sys.path.insert(0, os.path.expanduser("~/ab-hms/ann-benchmarks"))
from ann_benchmarks.distance import metrics as pd
name = sys.argv[1]; NQ = 10000; K = 100
src = os.path.expanduser(f"~/ab-hms/ann-benchmarks/data/{name}.hdf5")
out = os.path.expanduser(f"~/ab-a/data/{name}-heldout.hdf5")
os.makedirs(os.path.dirname(out), exist_ok=True)
f = h5py.File(src); dist = f.attrs["distance"]
X = np.array(f["train"], dtype=np.float32)
rng = np.random.default_rng(20261108)
qi = np.sort(rng.choice(len(X), NQ, replace=False))
mask = np.ones(len(X), bool); mask[qi] = False
train = np.ascontiguousarray(X[mask]); test = np.ascontiguousarray(X[qi])
if dist == "angular":
    idx = train / np.linalg.norm(train, axis=1, keepdims=True); lens = None
else:
    idx = train; lens = (train.astype(np.float64) ** 2).sum(1).astype(np.float32)
nbr = np.zeros((NQ, K), np.int64); dd = np.zeros((NQ, K), np.float64)
B = 200
for s in range(0, NQ, B):
    q = test[s:s+B]
    sc = -(q @ idx.T) if dist == "angular" else lens[None, :] - 2 * (q @ idx.T)
    part = np.argpartition(sc, K, axis=1)[:, :K]
    ps = np.take_along_axis(sc, part, 1)
    o = np.argsort(ps, 1)
    nbr[s:s+B] = np.take_along_axis(part, o, 1)
    if s % 2000 == 0: print(s, flush=True)
m = pd[dist]
for i in range(NQ):
    dd[i] = [m.distance(test[i], train[j]) for j in nbr[i]]
with h5py.File(out, "w") as g:
    g.attrs["type"] = "dense"; g.attrs["distance"] = dist; g.attrs["dimension"] = train.shape[1]; g.attrs["point_type"] = "float"
    g["train"] = train; g["test"] = test; g["neighbors"] = nbr; g["distances"] = dd
print("done", out, train.shape, test.shape)
