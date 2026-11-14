import sys, time, json, os, numpy as np
from writerslogic_warm import VectorIndex
D = os.path.expanduser("~/ood/up/data/text2image1B/")
h = np.fromfile(D + "query.public.100K.fbin", dtype=np.uint32, count=2)
Q = np.array(np.memmap(D + "query.public.100K.fbin", dtype=np.float32, mode="r", offset=8, shape=(h[0], h[1])))
perm = np.random.default_rng(7).permutation(len(Q)); tid, hid = perm[0::2], perm[1::2]
gt = np.load(os.path.expanduser("~/oodpart/gt1m_100k.npy"))
idx = VectorIndex("ip", degree=32, build_ef=500, threads=8, code_bits=8, residual_bits=8, direct_ip=True)
t = time.time(); idx.load(os.path.expanduser("~/ood/i1m.idx")); print("load", time.time() - t, flush=True)
for g in sys.argv[1].split(","):
    idx.set_query_arguments(*[int(x) for x in g.split(":")])
    out = {}
    for name, ids in (("tune", tid), ("held", hid)):
        q = Q[ids]; best = 0
        for r in range(3):
            t = time.time(); R = idx.batch_query_array(q, 10); best = max(best, len(q) / (time.time() - t))
        out[name] = (float(np.mean([len(set(R[i]) & set(gt[ids[i]])) / 10 for i in range(len(q))])), best)
    print(f"graph {g} tune {out['tune'][0]:.4f} @ {out['tune'][1]:.0f} held {out['held'][0]:.4f} @ {out['held'][1]:.0f}", flush=True)
