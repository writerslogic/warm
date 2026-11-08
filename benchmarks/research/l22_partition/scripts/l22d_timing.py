"""Sustained-variance check for the L22 assignment term (STATE.md 01:15Z): GEMM alone (8 workers x
BLAS 1) at start and end, restricted K32 nearest+spill on the 1M insert rows twice, all with C32.
usage: l22d_timing.py C32.npy OUT.json [start|end|both]"""
import importlib.util, json, os, sys, time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
s_ = importlib.util.spec_from_file_location("l22b2", os.path.join(HERE, "l22b2.py"))
b2 = importlib.util.module_from_spec(s_)
s_.loader.exec_module(b2)
b, a3 = b2.b, b2.a3

ce = b.Cent(np.load(sys.argv[1]))
which = sys.argv[3] if len(sys.argv) > 3 else "both"
n_tot = int(np.fromfile(f"{a3.D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
mm = np.memmap(f"{a3.D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, a3.DIM))
Xi = np.ascontiguousarray(mm[15_000_000:16_000_000])
res = {}
if os.path.exists(sys.argv[2]):
    with open(sys.argv[2]) as f:
        res = json.load(f)
gemm = lambda: b.par_rows(lambda s, e: Xi[s:e] @ ce.C.T, len(Xi), 4096)
_, t = b.timed(gemm)
res.setdefault("gemm_w8_s", []).append(t)
if which in ("both", "start"):
    for _ in range(2):
        _, t = b.timed(lambda: b2.assign_restricted(Xi, ce, 32))
        res.setdefault("r32_s_per_1M", []).append(t)
with open(sys.argv[2], "w") as f:
    json.dump(res, f, indent=1)
print("RESULT", which, json.dumps(res), flush=True)
