import sys, time, json, os, numpy as np, argparse
from writerslogic_warm.writerslogic_warm import PartitionIndex
p = argparse.ArgumentParser()
p.add_argument("--n", type=int, default=10**6)
p.add_argument("--lists", type=int, default=1024)
p.add_argument("--pq_m", type=int, default=51)
p.add_argument("--np", default="10,20,40,80")
p.add_argument("--ro", default="400,1000,2000,4000")
p.add_argument("--threads", type=int, default=8)
p.add_argument("--iters", type=int, default=10)
p.add_argument("--extra", default="{}")
p.add_argument("--out", required=True)
p.add_argument("--tag", default="")
a = p.parse_args()
D = os.path.expanduser("~/ood/up/data/text2image1B/"); DIM = 204
def fb(fn, n=None):
    h = np.fromfile(fn, dtype=np.uint32, count=2); n = h[0] if n is None else n
    return np.memmap(fn, dtype=np.float32, mode="r", offset=8, shape=(n, h[1]))
X = np.array(fb(D + "base.1B.fbin.crop_nb_10000000", a.n))
Q = np.array(fb(D + "query.public.100K.fbin"))
perm = np.random.default_rng(7).permutation(len(Q))
gtall = np.load(os.path.expanduser("~/oodpart/gt1m_100k.npy"))
tid, hid = perm[0::2], perm[1::2]
def lift_q(q):
    o = np.zeros((len(q), DIM), np.float32); o[:, :200] = q; return o
tq, hq = lift_q(Q[tid]), lift_q(Q[hid]); tg, hg = gtall[tid], gtall[hid]
nrm = (X ** 2).sum(1); m2 = float(nrm.max())
extra = json.loads(a.extra)
t0 = time.time()
idx = PartitionIndex.create(DIM, lists=a.lists, train_at=a.n, nprobe=10, reorder=400, pq_m=a.pq_m,
        threads=a.threads, train_sample=a.n, kmeans_iters=a.iters, nprobe_by_live=[], retrain_imb=0.0, **extra)
for s in range(0, a.n, 250000):
    e = min(a.n, s + 250000); o = np.zeros((e - s, DIM), np.float32)
    o[:, :200] = X[s:e]; o[:, 200] = np.sqrt(np.maximum(m2 - nrm[s:e], 0))
    idx.insert(np.arange(s, e, dtype=np.int64), o)
ins = time.time() - t0
idx.search(tq[:64], 10); build = time.time() - t0
print(f"lists {a.lists} pq_m {a.pq_m} insert {ins:.1f}s build+train {build:.1f}s", flush=True)
def rec(R, g): return float(np.mean([len(set(R[i]) & set(g[i])) / 10 for i in range(len(g))]))
f = open(a.out, "a")
for npb in map(int, a.np.split(",")):
    for ro in map(int, a.ro.split(",")):
        idx.set_nprobe(npb); idx.set_reorder(ro)
        s0 = idx.stats(); best = 0
        for r in range(2):
            t = time.time(); R = idx.search(tq, 10); best = max(best, len(tq) / (time.time() - t))
        s1 = idx.stats(); rt = rec(R, tg)
        hqps = 0
        for r in range(2):
            t = time.time(); R2 = idx.search(hq, 10); hqps = max(hqps, len(hq) / (time.time() - t))
        rh = rec(R2, hg)
        d = {k: s1[k] - s0[k] for k in ("scan_s", "reorder_s", "route_s", "centres_s", "lut_mean_s", "search_s", "blocks")}
        row = dict(tag=a.tag, lists=a.lists, pq_m=a.pq_m, nprobe=npb, reorder=ro, tune_recall=rt, tune_qps=best,
                   held_recall=rh, held_qps=hqps, build_s=build, phases_2pass=d, extra=extra)
        f.write(json.dumps(row) + "\n"); f.flush()
        print(f"np {npb} ro {ro}: tune {rt:.4f} @ {best:.0f}  held {rh:.4f} @ {hqps:.0f}  scan {d['scan_s']:.1f} reorder {d['reorder_s']:.1f} of {d['search_s']:.1f}", flush=True)
