"""L22 item 4 (STATE.md): price list maintenance (split policy) on the final_runbook stream.

Replays the stream from the train op with centres trained there (seed 7, as l22b). Every inserted row
gets nearest + SOAR spill (l22b exact kernel). Before each search, any list whose live memberships
exceed T x the current mean is split: 2-means (5 it) on its member rows; primary members go to the
nearer child, spill members to the child with the lower SOAR loss w.r.t. their own primary; no
neighbour reassignment. Arms T = none / 3 / 2 run on the same insert stream. At the eval ops the
ceiling (official GT top-10 in probed lists) and exact scanned fraction (from all live memberships)
are reported at several nprobe; maintenance wall seconds (numpy, 8 workers for assignment) and the
number of splits / rows moved are reported per arm. maint_s = split work (2-means, reassignment);
check_s = the numpy over-size scan of all live rows and the full G = C.C^T rebuild, which an
incremental per-list counter and a one-row G update replace in Rust, so it is reported, not priced.
usage: l22c_maint.py --out DIR [--train-op 32] [--K 32] [--kernel restricted|full] | --synthetic
"""
import argparse, importlib.util, json, os, sys, time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, fn):
    s = importlib.util.spec_from_file_location(name, os.path.join(HERE, fn))
    m = importlib.util.module_from_spec(s)
    s.loader.exec_module(m)
    return m


b = load("l22b", "l22b.py")
b2 = load("l22b2", "l22b2.py")
a3 = b.a3
LAM = b.LAM


class Arm:
    def __init__(self, name, T, C, n_tot):
        self.name, self.T = name, T
        self.ce = b.Cent(C)
        self.a1 = np.full(n_tot, -1, np.int32)
        self.a2 = np.full(n_tot, -1, np.int32)
        self.splits = self.moved = 0
        self.maint_s = self.assign_s = self.check_s = 0.0

    def counts(self, live_ids):
        L = len(self.ce.C)
        return np.bincount(self.a1[live_ids], minlength=L) + np.bincount(self.a2[live_ids], minlength=L)

    def maintain(self, mm, live_ids, rng):
        if self.T is None:
            return
        while True:
            t = time.time()
            cnt = self.counts(live_ids)
            big = np.nonzero(cnt > self.T * cnt.mean())[0]
            if len(big) == 0:
                self.check_s += time.time() - t
                break
            in1 = np.isin(self.a1[live_ids], big)
            in2 = np.isin(self.a2[live_ids], big)
            r1, r2 = live_ids[in1], live_ids[in2]
            o1, o2 = np.argsort(self.a1[r1], kind="stable"), np.argsort(self.a2[r2], kind="stable")
            r1, r2 = r1[o1], r2[o2]
            k1 = np.searchsorted(self.a1[r1], big, side="left"), np.searchsorted(self.a1[r1], big, side="right")
            k2 = np.searchsorted(self.a2[r2], big, side="left"), np.searchsorted(self.a2[r2], big, side="right")
            self.check_s += time.time() - t
            t = time.time()
            C = self.ce.C
            stuck = 0
            for j, L in enumerate(big):
                p = r1[k1[0][j]:k1[1][j]]
                s = r2[k2[0][j]:k2[1][j]]
                mem = np.concatenate([p, s])
                Xm = np.asarray(mm[np.sort(mem)])
                ca, cb = self.two_means(Xm, rng)
                if ca is None:
                    stuck += 1
                    continue
                nid = len(C)
                C = np.concatenate([C, cb[None, :]])
                C[L] = ca
                if len(p):
                    Xp = np.asarray(mm[p])
                    to_b = ((Xp - cb) ** 2).sum(1) < ((Xp - ca) ** 2).sum(1)
                    self.a1[p[to_b]] = nid
                    self.moved += int(to_b.sum())
                if len(s):
                    Xs = np.asarray(mm[s]).astype(np.float64)
                    r = Xs - C[self.a1[s]].astype(np.float64)
                    rr = np.maximum((r * r).sum(1), 1e-20)

                    def soar(c):
                        d = Xs - c.astype(np.float64)
                        return (d * d).sum(1) + LAM * (d * r).sum(1) ** 2 / rr

                    to_b = soar(cb) < soar(ca)
                    self.a2[s[to_b]] = nid
                    self.moved += int(to_b.sum())
                self.splits += 1
            self.maint_s += time.time() - t
            t = time.time()
            self.ce = b.Cent(C)
            self.check_s += time.time() - t
            if stuck == len(big):
                break

    @staticmethod
    def two_means(X, rng, iters=5):
        if len(X) < 4:
            return None, None
        c = X[rng.choice(len(X), 2, replace=False)].astype(np.float64)
        for _ in range(iters):
            lab = ((X[:, None, :] - c[None]) ** 2).sum(2).argmin(1) if len(X) < 20000 else \
                ((X * X).sum(1)[:, None] - 2 * X @ c.T + (c * c).sum(1)[None]).argmin(1)
            if lab.min() == lab.max():
                return None, None
            c = np.stack([X[lab == 0].mean(0), X[lab == 1].mean(0)])
        return c[0].astype(np.float32), c[1].astype(np.float32)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(a3.PISO, "l22a", "out7"))
    ap.add_argument("--train-op", type=int, default=32)
    ap.add_argument("--K", type=int, default=32)
    ap.add_argument("--kernel", choices=["restricted", "full"], default="restricted")
    ap.add_argument("--thresholds", type=float, nargs="*", default=[3.0, 2.0])
    ap.add_argument("--eval-ops", type=int, nargs="+", default=[100, 200, 400, 640, 900, 1120, 1280])
    ap.add_argument("--tiers", type=int, nargs="+", default=[250, 300, 350, 400, 450, 500, 600, 700, 800])
    ap.add_argument("--synthetic", action="store_true")
    args = ap.parse_args()
    rng = np.random.default_rng(5)
    if args.synthetic:
        n = 40000
        X = (rng.standard_normal((n, 16)) + 4 * rng.standard_normal((40, 16))[np.repeat(np.arange(40), n // 40)]).astype(np.float32)
        C = a3.kmeans(X[:2000], 20, 5, rng)  # trained on 2 clusters' worth: lists grow unbalanced
        arm = Arm("syn", 2.0, C, n)
        live = np.arange(n)
        arm.a1[:], arm.a2[:] = b.assign_exact(X, arm.ce, 8, ch=5000)
        before = arm.counts(live)
        arm.maintain(X, live, rng)
        after = arm.counts(live)
        a1r = a3.nearest(X, arm.ce.C)
        ok = after.max() <= 2.0 * after.mean() + 1 and after.sum() == before.sum() and arm.splits > 0
        print("splits", arm.splits, "lists", len(arm.ce.C), "max/mean before", before.max() / before.mean(),
              "after", after.max() / after.mean(), "primary == nearest", float((arm.a1 == a1r).mean()))
        print("SELFCHECK", "PASS" if ok else "FAIL")
        sys.exit(0 if ok else 1)
    os.makedirs(args.out, exist_ok=True)
    n_tot = int(np.fromfile(f"{a3.D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
    mm = np.memmap(f"{a3.D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, a3.DIM))
    Q = np.load(f"{a3.PISO}/queries1000.npy")
    ops = a3.parse_runbook(a3.RUNBOOK)
    live = b.live_at(ops, n_tot, args.train_op)
    ids = np.nonzero(live)[0]
    r7 = np.random.default_rng(7)
    C0 = a3.kmeans(np.asarray(mm[np.sort(r7.choice(ids, min(262144, len(ids)), replace=False))]), 5000, 10, r7)
    assign = (lambda X, ce: b2.assign_restricted(X, ce, args.K)) if args.kernel == "restricted" else (lambda X, ce: b.assign_exact(X, ce, 32))
    arms = [Arm("fixed", None, C0.copy(), n_tot)] + [Arm(f"split{T:g}", T, C0.copy(), n_tot) for T in args.thresholds]
    Xl = np.asarray(mm[ids])
    for arm in arms:
        arm.a1[ids], arm.a2[ids] = assign(Xl, arm.ce)
    del Xl
    live[:] = False
    started = False
    res = {"args": vars(args), "eval": []}
    for o in ops:
        op = o.get("operation")
        if op == "search" and o["i"] == args.train_op:
            started = True
            live = b.live_at(ops, n_tot, args.train_op)
            continue
        if not started:
            continue
        if op == "insert":
            s, e = int(o["start"]), int(o["end"])
            live[s:e] = True
            for c0 in range(s, e, 1_000_000):
                c1 = min(c0 + 1_000_000, e)
                Xi = np.ascontiguousarray(mm[c0:c1])
                for arm in arms:
                    t = time.time()
                    arm.a1[c0:c1], arm.a2[c0:c1] = assign(Xi, arm.ce)
                    arm.assign_s += time.time() - t
        elif op == "delete":
            live[int(o["start"]):int(o["end"])] = False
        elif op == "search":
            i = o["i"]
            lid = np.nonzero(live)[0]
            for arm in arms:
                arm.maintain(mm, lid, rng)
            if i not in args.eval_ops:
                continue
            gt = a3.read_gt(f"{a3.GTD}/step{i}.gt100")[:len(Q)]
            row = {"op": i, "live": int(len(lid))}
            for arm in arms:
                cnt = arm.counts(lid)
                rank = b.qrank(Q, arm.ce.C)
                e = b.ceil_from(rank, arm.a1[gt], arm.a2[gt], cnt, args.tiers)
                e.update(lists=len(arm.ce.C), splits=arm.splits, moved=arm.moved, maint_s=arm.maint_s, check_s=arm.check_s, assign_s=arm.assign_s)
                row[arm.name] = e
                print(f"RESULT maint op={i} live={len(lid)} arm={arm.name} lists={len(arm.ce.C)} splits={arm.splits} moved={arm.moved} "
                      f"maint_s={arm.maint_s:.1f} check_s={arm.check_s:.1f} assign_s={arm.assign_s:.1f} " + " ".join(
                          f"np{t}={e[t]['recall']:.4f}/{e[t]['codes_scanned'] / len(lid):.3f}" for t in args.tiers), flush=True)
            res["eval"].append(row)
            with open(os.path.join(args.out, "l22c.json"), "w") as f:
                json.dump(res, f, indent=1, default=str)
    a3.log("done")


if __name__ == "__main__":
    main()
