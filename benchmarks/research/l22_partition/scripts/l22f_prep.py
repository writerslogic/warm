"""Inputs for the L22 live sweep (STATE.md 05:45Z): stage-1 op-32 fixed centres (A3's exact
procedure and seed) and, for the search ops whose live size is nearest each target, the live rows
in id order as an fbin, plus official step GT (queries 0..499, top-10) remapped to row positions.
usage: l22f_prep.py OUTDIR TARGET_LIVE..."""
import importlib.util, json, os, sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
s_ = importlib.util.spec_from_file_location("l22a3", os.path.join(HERE, "l22a3.py"))
a3 = importlib.util.module_from_spec(s_)
s_.loader.exec_module(a3)
out, targets = sys.argv[1], [int(t) for t in sys.argv[2:]]
os.makedirs(out, exist_ok=True)
n_tot = int(np.fromfile(f"{a3.D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
mm = np.memmap(f"{a3.D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, a3.DIM))
ops = a3.parse_runbook(a3.RUNBOOK)
live = np.zeros(n_tot, bool)
sizes, rng, snaps = [], np.random.default_rng(7), {}
for o in ops:
    op = o.get("operation")
    if op == "insert":
        live[int(o["start"]):int(o["end"])] = True
    elif op == "delete":
        live[int(o["start"]):int(o["end"])] = False
    elif op == "search":
        i, n = o["i"], int(live.sum())
        sizes.append((i, n))
        if i == 32:
            ids = np.nonzero(live)[0]
            C = a3.kmeans(np.asarray(mm[np.sort(rng.choice(ids, 262144, replace=False))]), 5000, 10, rng)
            C.astype("<f4").tofile(f"{out}/centres_op32.bin")
            print("centres op32 live", len(ids), flush=True)
post = [(i, n) for i, n in sizes if i > 32]
pick = {min(post, key=lambda s: abs(s[1] - t))[0] for t in targets}
live[:] = False
meta = []
for o in ops:
    op = o.get("operation")
    if op == "insert":
        live[int(o["start"]):int(o["end"])] = True
    elif op == "delete":
        live[int(o["start"]):int(o["end"])] = False
    elif op == "search" and o["i"] in pick:
        i = o["i"]
        ids = np.nonzero(live)[0]
        gt = a3.read_gt(f"{a3.GTD}/step{i}.gt100")[:500]
        assert live[gt.ravel()].all()
        pos = np.searchsorted(ids, gt).astype("<i4")
        assert (ids[pos] == gt).all()
        pos.tofile(f"{out}/gt_op{i}_500.bin")
        with open(f"{out}/base_op{i}.fbin", "wb") as f:
            np.array([len(ids), a3.DIM], "<u4").tofile(f)
            for s in range(0, len(ids), 1_000_000):
                np.asarray(mm[ids[s:s + 1_000_000]], dtype="<f4").tofile(f)
        meta.append({"op": i, "live": int(len(ids))})
        print("op", i, "live", len(ids), flush=True)
json.dump({"targets": targets, "picked": meta}, open(f"{out}/sweep_meta.json", "w"))
