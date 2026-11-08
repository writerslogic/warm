"""L22 kill test A3: drift of a fixed coarse quantizer over the final_runbook stream (numpy only).

Replays live-set membership from final_runbook.yaml. Centers (5,000, k-means 10 iterations on a
262,144 sample) are trained once on the live ids at search op `--train-op` and kept fixed; rows get
their nearest list plus a SOAR (lambda) spill list. At each evaluated search op the spilled ceiling
|official GT top-10 ∩ probed rows| / 10 is measured for queries 0..999, with mean codes scanned per
query estimated from a uniform live sample. A retrained arm (centers trained on that step's live set)
separates drift from step difficulty.
usage: l22a3.py [--train-op 32] [--eval-ops 32 100 ...] [--out DIR]
"""
import argparse, json, os, re, time

import numpy as np

BAB = "/home/davidcondrey/bigann/s0/bab"
D = f"{BAB}/data/MSTuring-30M-clustered"
GTD = f"{D}/29998994/final_runbook.yaml"
RUNBOOK = f"{BAB}/neurips23/runbooks/final_runbook.yaml"
PISO = "/home/davidcondrey/bigann/piso"
DIM = 100


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def parse_runbook(path):
    ops, cur = [], None
    with open(path) as fh:
        lines = fh.read().splitlines()
    for line in lines:
        s = line.strip()
        m = re.match(r"^(\d+):$", s)
        if m:
            cur = {"i": int(m.group(1))}
            ops.append(cur)
            continue
        m = re.match(r"^(\w+): '?([\w]+)'?$", s)
        if m and cur is not None:
            cur[m.group(1)] = m.group(2)
    return ops


def read_gt(path, k=10):
    n, kk = np.fromfile(path, dtype=np.uint32, count=2)
    ids = np.fromfile(path, dtype=np.int32, count=int(n) * int(kk), offset=8).reshape(int(n), int(kk))
    return ids[:, :k]


def nearest(X, C, chunk=20000):
    c2 = (C * C).sum(1)
    out = np.empty(len(X), np.int32)
    for s in range(0, len(X), chunk):
        x = X[s:s + chunk]
        out[s:s + chunk] = (c2[None, :] - 2.0 * (x @ C.T)).argmin(1)
    return out


def kmeans(X, k, iters, rng):
    C = X[rng.choice(len(X), k, replace=False)].astype(np.float32)
    for _ in range(iters):
        a = nearest(X, C)
        counts = np.bincount(a, minlength=k)
        sums = np.zeros((k, X.shape[1]), np.float64)
        np.add.at(sums, a, X)
        nz = counts > 0
        C[nz] = (sums[nz] / counts[nz, None]).astype(np.float32)
        empty = np.nonzero(~nz)[0]
        if len(empty):
            C[empty] = X[rng.choice(len(X), len(empty), replace=False)]
    return C


def soar_spill(X, C, a1, lam, chunk=20000):
    c2 = (C * C).sum(1)
    out = np.empty(len(X), np.int32)
    for s in range(0, len(X), chunk):
        x, p = X[s:s + chunk], a1[s:s + chunk]
        r = x - C[p]
        rr = np.maximum((r * r).sum(1), 1e-20)
        d = (x * x).sum(1)[:, None] - 2.0 * (x @ C.T) + c2[None, :]
        proj = (x * r).sum(1)[:, None] - r @ C.T
        d = d + lam * proj * proj / rr[:, None]
        d[np.arange(len(x)), p] = np.inf
        out[s:s + chunk] = d.argmin(1)
    return out


def evaluate(mm, C, Q, gt, live_ids, lam, tiers, sample, rng):
    """Spilled ceiling and estimated codes scanned per query at each nprobe tier."""
    k = len(C)
    rank = np.argsort(np.argsort((C * C).sum(1)[None, :] - 2.0 * (Q @ C.T), axis=1, kind="stable"), axis=1)
    g = np.unique(gt.ravel())
    Xg = np.asarray(mm[np.sort(g)])
    a1g = nearest(Xg, C)
    a2g = soar_spill(Xg, C, a1g, lam)
    pos = np.searchsorted(np.sort(g), gt)
    l1, l2 = a1g[pos], a2g[pos]
    s = np.sort(rng.choice(live_ids, min(sample, len(live_ids)), replace=False))
    Xs = np.asarray(mm[s])
    a1s = nearest(Xs, C)
    a2s = soar_spill(Xs, C, a1s, lam)
    counts = (np.bincount(a1s, minlength=k) + np.bincount(a2s, minlength=k)) * (len(live_ids) / len(s))
    cum = np.cumsum(counts[np.argsort(rank, axis=1)], axis=1)
    out = {}
    for t in tiers:
        hit = (np.take_along_axis(rank, l1, 1) < t) | (np.take_along_axis(rank, l2, 1) < t)
        out[t] = {"recall": float(hit.mean()), "codes_scanned": float(cum[:, t - 1].mean())}
    out["max_list_share"] = float(counts.max() / counts.sum())
    out["lists_over_4x_mean"] = int((counts > 4 * counts.mean()).sum())
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--train-op", type=int, default=32)
    ap.add_argument("--eval-ops", type=int, nargs="+", default=[32, 100, 200, 400, 640, 900, 1120, 1280])
    ap.add_argument("--retrain", action="store_true", default=True)
    ap.add_argument("--no-retrain", dest="retrain", action="store_false")
    ap.add_argument("--lists", type=int, default=5000)
    ap.add_argument("--train-sample", type=int, default=262144)
    ap.add_argument("--lam", type=float, default=1.0)
    ap.add_argument("--tiers", type=int, nargs="+", default=[350, 500, 700])
    ap.add_argument("--sample", type=int, default=500_000)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--out", default=os.path.join(PISO, "l22a", "out3"))
    args = ap.parse_args()
    rng = np.random.default_rng(args.seed)
    n_tot = int(np.fromfile(f"{D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
    mm = np.memmap(f"{D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, DIM))
    Q = np.load(f"{PISO}/queries1000.npy")
    ops = parse_runbook(RUNBOOK)
    live = np.zeros(n_tot, bool)
    C_fixed = None
    res = {"args": vars(args), "steps": []}
    for o in ops:
        op = o.get("operation")
        if op == "insert":
            live[int(o["start"]):int(o["end"])] = True
        elif op == "delete":
            live[int(o["start"]):int(o["end"])] = False
        elif op == "search":
            i = o["i"]
            if i == args.train_op:
                ids = np.nonzero(live)[0]
                t = time.time()
                C_fixed = kmeans(np.asarray(mm[np.sort(rng.choice(ids, min(args.train_sample, len(ids)), replace=False))]), args.lists, 10, rng)
                log(f"trained fixed centers at op {i} live {len(ids)} in {time.time() - t:.0f}s")
            if i not in args.eval_ops or C_fixed is None:
                continue
            ids = np.nonzero(live)[0]
            gt = read_gt(f"{GTD}/step{i}.gt100")[:len(Q)]
            assert live[gt.ravel()].all(), f"GT at op {i} names non-live rows"
            row = {"op": i, "live": int(len(ids))}
            t = time.time()
            row["fixed"] = evaluate(mm, C_fixed, Q, gt, ids, args.lam, args.tiers, args.sample, rng)
            if args.retrain and i != args.train_op:
                C_r = kmeans(np.asarray(mm[np.sort(rng.choice(ids, min(args.train_sample, len(ids)), replace=False))]), args.lists, 10, rng)
                row["retrained"] = evaluate(mm, C_r, Q, gt, ids, args.lam, args.tiers, args.sample, rng)
            elif args.retrain:
                row["retrained"] = row["fixed"]
            res["steps"].append(row)
            for arm in ("fixed", "retrained"):
                if arm not in row:
                    continue
                e = row[arm]
                print(f"RESULT op={i} live={len(ids)} arm={arm} " + " ".join(
                    f"np{t}={e[t]['recall']:.4f}/{e[t]['codes_scanned'] / len(ids):.3f}" for t in args.tiers)
                    + f" max_list_share={e['max_list_share']:.5f} lists_over_4x={e['lists_over_4x_mean']}", flush=True)
            log(f"op {i} done in {time.time() - t:.0f}s")
    os.makedirs(args.out, exist_ok=True)
    with open(os.path.join(args.out, "l22a3.json"), "w") as f:
        json.dump(res, f, indent=1, default=str)
    log("done")


if __name__ == "__main__":
    main()
