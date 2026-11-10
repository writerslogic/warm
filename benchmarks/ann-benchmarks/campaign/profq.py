"""profq.py <dataset> '<build json>' ef rerank [reps]: the harness's single-query loop (index.query per test query) on the cached build."""
import json, sys, time, os, h5py, numpy as np
sys.path.insert(0, os.getcwd())
from ann_benchmarks.algorithms.warm.module import WARM
ds, b, ef, rr = sys.argv[1], json.loads(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
reps = int(sys.argv[5]) if len(sys.argv) > 5 else 3
f = h5py.File(f"data/{ds}.hdf5"); metric = f.attrs["distance"]
X = np.array(f["train"]); Q = np.array(f["test"]); gt = np.array(f["neighbors"])
w = WARM(metric, b); w.fit(X); w.set_query_arguments(ef, rr)
Q = [np.ascontiguousarray(q, dtype=np.float32) for q in Q]
for r in range(reps):
    t = time.perf_counter(); res = [w.query(q, 10) for q in Q]; dt = time.perf_counter() - t
    rec = np.mean([len(set(a) & set(g[:10])) / 10 for a, g in zip(res, gt)])
    print(f"rep{r} qps={len(Q)/dt:.0f} recall~{rec:.4f}", flush=True)
