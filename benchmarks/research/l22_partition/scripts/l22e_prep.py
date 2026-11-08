"""Inputs for the PartitionTier A2 acceptance (STATE.md 02:40Z): A2's centres reproduced by seed
(same procedure as l22a2 / l22b2 run_prefix), queries 0..499 and their prefix GT top-10, raw LE.
usage: l22e_prep.py OUTDIR"""
import importlib.util, os, sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
s_ = importlib.util.spec_from_file_location("l22b", os.path.join(HERE, "l22b.py"))
b = importlib.util.module_from_spec(s_)
s_.loader.exec_module(b)
a3 = b.a3
out = sys.argv[1]
os.makedirs(out, exist_ok=True)
Q = np.load(f"{a3.PISO}/queries1000.npy")[:500].astype("<f4")
Q.tofile(f"{out}/queries500.bin")
n_tot = int(np.fromfile(f"{b.D2}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
mm = np.memmap(f"{b.D2}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, a3.DIM))
for n in (1_000_000, 3_000_000):
    gt = np.load(f"{a3.PISO}/gt_{n}.npy")[:500, :10].astype("<i4")
    gt.tofile(f"{out}/gt_{n}_500.bin")
    X = np.ascontiguousarray(mm[:n])
    rng = np.random.default_rng(7)
    C = a3.kmeans(X[rng.choice(n, 262144, replace=False)], 5000, 10, rng).astype("<f4")
    C.tofile(f"{out}/centres_{n}.bin")
    print("prep", n, C.shape, gt.shape, flush=True)
