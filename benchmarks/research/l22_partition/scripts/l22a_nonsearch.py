"""Non-search cost of the L22 tier on wl-03 (STATE.md 23:40Z): insert assignment with SOAR spill,
pre-train brute-force search, and training at op 32. numpy only; times are wall seconds on the
calling process's cpus. Prints one RESULT line per component."""
import importlib.util, json, os, sys, time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("a3", os.path.join(HERE, "l22a3.py"))
a3 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a3)

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(a3.PISO, "l22a", "out5")
rng = np.random.default_rng(7)
n_tot = int(np.fromfile(f"{a3.D}/30M-clustered64.fbin", dtype=np.uint32, count=1)[0])
mm = np.memmap(f"{a3.D}/30M-clustered64.fbin", dtype=np.float32, mode="r", offset=8, shape=(n_tot, a3.DIM))
Q = np.fromfile(f"{a3.D}/testQuery10K.fbin", dtype=np.float32, offset=8).reshape(-1, a3.DIM)
res = {}

# live set at op 32 (search 16)
live = np.zeros(n_tot, bool)
for o in a3.parse_runbook(a3.RUNBOOK):
    op = o.get("operation")
    if op == "insert":
        live[int(o["start"]):int(o["end"])] = True
    elif op == "delete":
        live[int(o["start"]):int(o["end"])] = False
    elif op == "search" and o["i"] == 32:
        break
ids = np.nonzero(live)[0]
X = np.asarray(mm[ids])
print("live at op 32", len(ids), flush=True)

# (c) training
t = time.time()
C = a3.kmeans(np.asarray(mm[np.sort(rng.choice(ids, 262144, replace=False))]), 5000, 10, rng)
res["train_kmeans_s"] = time.time() - t
t = time.time()
a1 = a3.nearest(X, C)
a2 = a3.soar_spill(X, C, a1, 1.0)
res["train_assign_live_s"] = time.time() - t
print(f"RESULT train kmeans={res['train_kmeans_s']:.1f}s assign+spill {len(ids)} rows={res['train_assign_live_s']:.1f}s", flush=True)

# (a) insert assignment: 1M rows outside the op-32 live set, nearest + spill
rows = np.arange(15_000_000, 16_000_000)
Xi = np.asarray(mm[rows])
t = time.time()
b1 = a3.nearest(Xi, C)
res["insert_nearest_1M_s"] = time.time() - t
t = time.time()
a3.soar_spill(Xi, C, b1, 1.0)
res["insert_spill_1M_s"] = time.time() - t
per_m = res["insert_nearest_1M_s"] + res["insert_spill_1M_s"]
res["insert_run_s_projected"] = per_m * 29.998834
print(f"RESULT insert nearest 1M={res['insert_nearest_1M_s']:.1f}s spill 1M={res['insert_spill_1M_s']:.1f}s run x29.999={res['insert_run_s_projected']:.0f}s", flush=True)

# (b) one brute-force search step at op-32 live, Q = 10k, top-10, chunked over rows
t = time.time()
q2 = (Q * Q).sum(1)
bd = np.full((len(Q), 10), np.inf, np.float32)
bi = np.full((len(Q), 10), -1, np.int64)
CH = 65536
for s in range(0, len(X), CH):
    x = X[s:s + CH]
    d = q2[:, None] - 2.0 * (Q @ x.T) + (x * x).sum(1)[None, :]
    cd = np.concatenate([bd, d], 1)
    sel = np.argpartition(cd, 9, axis=1)[:, :10]
    ci = np.concatenate([bi, np.broadcast_to(ids[s:s + CH][None, :], d.shape)], 1)
    bd, bi = np.take_along_axis(cd, sel, 1), np.take_along_axis(ci, sel, 1)
res["bruteforce_step_at_op32_s"] = time.time() - t
res["bruteforce_pretrain_s_projected"] = res["bruteforce_step_at_op32_s"] * 4_733_307 / len(ids)
print(f"RESULT bruteforce step {len(ids)} live={res['bruteforce_step_at_op32_s']:.1f}s pre-train 15 steps (sum live 4.733M)={res['bruteforce_pretrain_s_projected']:.0f}s", flush=True)

res["deletes_s_assumed"] = 30.0
res["nonsearch_total_s"] = (res["train_kmeans_s"] + res["train_assign_live_s"] + res["insert_run_s_projected"]
                            + res["bruteforce_pretrain_s_projected"] + res["deletes_s_assumed"])
print(f"RESULT nonsearch_total={res['nonsearch_total_s']:.0f}s", flush=True)
os.makedirs(OUT, exist_ok=True)
json.dump(res, open(os.path.join(OUT, "nonsearch.json"), "w"), indent=1)
