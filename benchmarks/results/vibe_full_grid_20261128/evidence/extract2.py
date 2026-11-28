import csv, glob, os, sys, h5py, numpy as np
root = os.path.expanduser(sys.argv[2])
data = os.path.expanduser("~/vibe/data")
truth = {}
def get_truth(ds):
    if ds in truth: return truth[ds]
    f = h5py.File(f"{data}/{ds}.hdf5", "r")
    X = np.asarray(f["train"], dtype=np.float64); Q = np.asarray(f["test"], dtype=np.float64)
    h5n = np.asarray(f["neighbors"])[:, :100]
    top = np.empty((len(Q), 100), dtype=np.int64)
    for i, q in enumerate(Q):
        s = X @ q
        idx = np.lexsort((np.arange(len(s)), -s))[:100]   # descending score, base-id ties
        top[i] = idx
    truth[ds] = (top, h5n); return truth[ds]
def rec(nb, tr):
    return float(np.mean([len(set(a.tolist()) & set(b.tolist())) / 100 for a, b in zip(nb, tr)]))
def finished(d):
    try:
        with open(f"{d}/exit") as fh:
            return fh.read().strip() == "0"
    except FileNotFoundError:
        return False
with open(sys.argv[1], "w", newline="") as fh:
    out = csv.writer(fh)
    out.writerow("block,unit,dataset,file,group,name,recall_f64,recall_h5,qps_last,best_qps,build_time".split(","))
    for d in sorted(glob.glob(f"{root}/*-*-*")):
        if not finished(d): continue
        b, unit, ds = os.path.basename(d).split("-", 2)
        top, h5n = get_truth(ds)
        for fn in sorted(glob.glob(f"{d}/results/{ds}/100/*/*.hdf5")):
            with h5py.File(fn, "r") as f:
                for g in f:
                    grp = f[g]
                    nb = np.asarray(grp["neighbors"])[:, :100]
                    t = np.asarray(grp["times"], dtype=np.float64)
                    out.writerow([b, unit, ds, os.path.basename(fn), g, grp.attrs["name"], rec(nb, top), rec(nb, h5n),
                                  1.0 / t.mean(), grp.attrs["best_qps"], grp.attrs["build_time"]])
