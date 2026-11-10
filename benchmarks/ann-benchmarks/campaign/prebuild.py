"""prebuild.py <dataset> '<build json>' : populate the WARM_BUILD_CACHE entry the wrapper would use (run from harness dir)."""
import json, sys, time, os, h5py, numpy as np
sys.path.insert(0, os.getcwd())
from ann_benchmarks.algorithms.warm.module import WARM
ds, b = sys.argv[1], json.loads(sys.argv[2])
f = h5py.File(f"data/{ds}.hdf5"); metric = f.attrs["distance"]
X = np.array(f["train"])
t = time.time()
w = WARM(metric, b)
w.fit(X)
print("fit", ds, b, round(time.time() - t, 1), "s", flush=True)
