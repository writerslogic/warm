"""Replay the first STEPS steps of the big-ann final_runbook through writerslogic_warm.PartitionIndex
with per-operation timers, the same data reads and id types as the neurips23 streaming adapter.

Output (one JSON line): per-operation seconds (binding vs read), the search digest (FNV of all
returned ids, equal digests mean bit-identical results), tie-aware recall@10 against the harness
GT100 files, the index stats() terms, peak RSS and the THP-backed share of the process.
"""
import argparse, json, os, re, resource, struct, sys, time
import numpy as np

FNV0, FNVP = 0xCBF29CE484222325, 0x100000001B3


def parse_runbook(path):
    ops, cur = [], None
    for line in open(path):
        if re.match(r"\s+\d+:\s*$", line):
            cur = {}
            ops.append(cur)
            continue
        m = re.match(r"\s+(\w+):\s*'?(\w+)'?\s*$", line)
        if m and cur is not None:
            cur[m.group(1)] = m.group(2)
    return ops


def read_fbin_range(fn, start, end, rows, dim):
    with open(fn, "rb") as f:
        assert f.read(8) == struct.pack("<II", rows, dim)
        f.seek(8 + start * dim * 4)
        v = np.fromfile(f, dtype="<f4", count=(end - start) * dim)
    return v.reshape(end - start, dim)


def read_queries(fn):
    n, d = np.fromfile(fn, dtype="uint32", count=2)
    return np.fromfile(fn, dtype="<f4", offset=8).reshape(int(n), int(d))


def read_gt(fn):
    n, d = map(int, np.fromfile(fn, dtype="uint32", count=2))
    with open(fn, "rb") as f:
        f.seek(8)
        I = np.fromfile(f, dtype="int32", count=n * d).reshape(n, d)
        D = np.fromfile(f, dtype="float32", count=n * d).reshape(n, d)
    return I, D


def tie_recall(gt_i, gt_d, res, k=10):
    """Per-query recall count with the harness rule (ties within 1e-6 of the k-th GT distance)."""
    out = np.zeros(len(res))
    for q in range(len(res)):
        end = gt_i.shape[1]
        t = gt_d[q, k - 1]
        for i in range(k, gt_i.shape[1]):
            if abs(t - gt_d[q, i]) >= 1e-6:
                end = i
                break
        out[q] = len(set(gt_i[q, :end].tolist()) & set(res[q].tolist()))
    return out


def fnv(h, a):
    for x in a.ravel().tolist():
        h = ((h ^ x) * FNVP) & 0xFFFFFFFFFFFFFFFF
    return h


def smaps():
    d = {}
    try:
        for l in open("/proc/self/smaps_rollup"):
            p = l.split()
            if p[0] in ("AnonHugePages:", "Rss:"):
                d[p[0][:-1]] = int(p[1])
    except OSError:
        pass
    return d


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--data", required=True, help="dir with 30M-clustered64.fbin, testQuery10K.fbin")
    a.add_argument("--runbook", required=True)
    a.add_argument("--gt", help="dir of stepN.gt100 files")
    a.add_argument("--steps", type=int, default=240)
    a.add_argument("--np", type=int, default=600)
    a.add_argument("--ro", type=int, default=1000)
    a.add_argument("--threads", type=int, default=8)
    a.add_argument("--label", default="")
    a.add_argument("--out")
    a.add_argument("--dump", default="", help="comma list of steps whose result ids are saved to DIR/stepN.npy")
    a.add_argument("--dump-dir", default=".")
    a.add_argument("--no-gt", action="store_true")
    a.add_argument("--lists", type=int, default=5000)
    a.add_argument("--train-at", type=int, default=1_000_000)
    a.add_argument("--settle", type=float, default=0.0)
    a.add_argument("--prefetch", type=int, default=0, help="read-ahead depth in insert ranges (0 = off)")
    args = a.parse_args()
    from writerslogic_warm import PartitionIndex

    base = os.path.join(args.data, "30M-clustered64.fbin")
    rows, dim = struct.unpack("<II", open(base, "rb").read(8))
    Q = np.ascontiguousarray(read_queries(os.path.join(args.data, "testQuery10K.fbin")), dtype=np.float32)
    ops = parse_runbook(args.runbook)[: args.steps]
    dump = {int(x) for x in args.dump.split(",") if x}
    idx = PartitionIndex.create(dim, lists=args.lists, train_at=args.train_at, nprobe=args.np,
                                reorder=args.ro, threads=args.threads)
    pre = None
    if args.prefetch:
        import threading
        fd = os.open(base, os.O_RDONLY)
        ranges = [(int(o["start"]), int(o["end"])) for o in ops if o["operation"] == "insert"]
        state = {"done": 0, "pos": 0}
        cond = threading.Condition()

        def worker():
            while True:
                with cond:
                    while state["pos"] >= len(ranges) or state["pos"] - state["done"] >= args.prefetch:
                        if state["pos"] >= len(ranges):
                            return
                        cond.wait()
                    s, e = ranges[state["pos"]]
                    state["pos"] += 1
                os.posix_fadvise(fd, 8 + s * dim * 4, (e - s) * dim * 4, os.POSIX_FADV_WILLNEED)
                with open(base, "rb") as f:
                    f.seek(8 + s * dim * 4)
                    f.read((e - s) * dim * 4)

        threading.Thread(target=worker, daemon=True).start()
        pre = (state, cond)
    t = {"insert": 0.0, "delete": 0.0, "search": 0.0, "read": 0.0, "convert": 0.0}
    h, recalls, steps_rec = FNV0, [], []
    t_all = time.perf_counter()
    for n, o in enumerate(ops, 1):
        op = o["operation"]
        t0 = time.perf_counter()
        if op == "insert":
            s, e = int(o["start"]), int(o["end"])
            ids = np.arange(s, e, dtype=np.uint32)
            t1 = time.perf_counter()
            X = read_fbin_range(base, s, e, rows, dim)
            t2 = time.perf_counter()
            X = np.ascontiguousarray(X, dtype=np.float32)
            i64 = ids.astype(np.int64)
            t3 = time.perf_counter()
            idx.insert(i64, X)
            t4 = time.perf_counter()
            t["read"] += t2 - t1
            t["convert"] += (t1 - t0) + (t3 - t2)
            t["insert"] += t4 - t3
            if pre:
                with pre[1]:
                    pre[0]["done"] += 1
                    pre[1].notify()
        elif op == "delete":
            ids = np.arange(int(o["start"]), int(o["end"]), dtype=np.uint32).astype(np.int64)
            t1 = time.perf_counter()
            idx.delete(ids)
            t["delete"] += time.perf_counter() - t1
        else:
            t1 = time.perf_counter()
            res = np.asarray(idx.search(Q, 10))
            dt = time.perf_counter() - t1
            t["search"] += dt
            h = fnv(h, res)
            rec = None
            if args.gt and not args.no_gt:
                gi, gd = read_gt(os.path.join(args.gt, f"step{n}.gt100"))
                rec = float(tie_recall(gi, gd, res).mean() / 10)
                recalls.append(rec)
            steps_rec.append((n, round(dt, 3), rec))
            if n in dump:
                np.save(os.path.join(args.dump_dir, f"step{n}.npy"), res.astype(np.int32))
    wall = time.perf_counter() - t_all
    st = idx.stats()
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024 / 1024
    rep = {"label": args.label, "steps": args.steps, "np": args.np, "ro": args.ro, "threads": args.threads,
           "env_thp": os.environ.get("WARM_THP"), "wall_s": round(wall, 2),
           **{k + "_s": round(v, 2) for k, v in t.items()},
           "digest": f"{h:016x}", "recall_mean": (round(float(np.mean(recalls)), 6) if recalls else None),
           "searches": len(steps_rec), "peak_rss_gib": round(rss, 3), "smaps_kb": smaps(),
           "stats": {k: (float(v) if not isinstance(v, str) else v) for k, v in st.items()},
           "search_steps": steps_rec[-3:]}
    line = json.dumps(rep)
    print(line)
    if args.out:
        open(args.out, "a").write(line + "\n")


main()
