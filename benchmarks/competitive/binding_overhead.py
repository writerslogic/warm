# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""How much of a per-query timing is the language binding, not the search.

  trivial      (run with the pinned hnswlib environment's python) per-call cost of hnswlib's Python
               `knn_query` on a one-vector index with k=1 and ef=1, where the search is a single distance
               computation, next to the same call made natively (driver.cpp `trivial`), a bare pybind
               method call, a Python no-op call and the timer itself; also thread and context-switch counts.
  warm-python  (run with an environment that has the warm-python wheel) the WARM serve sweep through the
               Python `UpdatableIndex.search`, one query per call, on the index the harness persisted.
  report       combine a competitive results file with the files above into
               benchmarks/results/hnswlib_python_overhead_v1.json: per-ef native vs Python hnswlib on the
               same index and the fraction of a Python-driven query spent outside the C++ search.
Every number is a per-trial or per-round measurement; intervals are percentile bootstrap intervals over them.
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

K = 10


def stats(ns):
    ns = np.asarray(ns, dtype=np.float64)
    return {"mean_ns": float(ns.mean()), "median_ns": float(np.median(ns)), "p99_ns": float(np.percentile(ns, 99))}


def proc_counters():
    fields = {}
    for line in Path("/proc/self/status").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in ("Threads", "voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"):
            fields[key] = int(value)
    return fields


def timed(fn, calls):
    out = np.empty(calls, dtype=np.int64)
    clock = time.perf_counter_ns
    for i in range(calls):
        t = clock()
        fn()
        out[i] = clock() - t
    return out


def trivial(a):
    import hnswlib
    dim, calls = a.dim, a.calls
    rng = np.random.default_rng(1)
    base = rng.standard_normal((1, dim)).astype(np.float32)
    q = rng.standard_normal((1, dim)).astype(np.float32)
    idx = hnswlib.Index(space="l2", dim=dim)
    idx.init_index(max_elements=1, M=16, ef_construction=200, random_seed=100)
    idx.add_items(base, np.arange(1))
    idx.set_ef(1)
    idx.set_num_threads(1)
    trials = []
    for t in range(a.trials):
        native = subprocess.run([a.driver, "trivial", str(dim), str(calls)], capture_output=True, check=True).stdout
        arr = np.frombuffer(native, dtype="<i8")
        entry = {"native_searchKnn": stats(arr[:calls]), "timer_floor_native": stats(arr[calls:])}
        for _ in range(1000):
            idx.knn_query(q, k=1)
        before = proc_counters()
        entry["python_knn_query"] = stats(timed(lambda: idx.knn_query(q, k=1)[0][0], calls))
        after = proc_counters()
        entry["python_knn_query_counters"] = {"threads_before": before["Threads"], "threads_after": after["Threads"],
                                               "voluntary_ctxt_switches": after["voluntary_ctxt_switches"] - before["voluntary_ctxt_switches"],
                                               "nonvoluntary_ctxt_switches": after["nonvoluntary_ctxt_switches"] - before["nonvoluntary_ctxt_switches"], "calls": calls}
        entry["python_bare_pybind_call"] = stats(timed(idx.get_current_count, calls))
        entry["python_noop_call"] = stats(timed(lambda: None, calls))
        trials.append(entry)
    # The timer floor is the cost of the two perf_counter_ns reads themselves: time an empty statement.
    for entry in trials:
        floor = np.empty(calls, dtype=np.int64)
        clock = time.perf_counter_ns
        for i in range(calls):
            t = clock()
            floor[i] = clock() - t
        entry["python_timer_floor"] = stats(floor)
    out = {"kind": "hnswlib-trivial-call", "dim": dim, "calls_per_trial": calls, "k": 1, "ef": 1, "num_threads": 1, "elements": 1,
           "hnswlib": "0.8.0", "python": sys.version.split()[0], "load_1m": os.getloadavg()[0], "trials": trials}
    Path(a.out).write_text(json.dumps(out, indent=1))


def warm_python(a):
    import writerslogic_warm as w
    data = Path(a.data)
    meta = json.loads((data / "meta.json").read_text())
    dim = meta["dim"]
    test = np.fromfile(data / "test.f32", dtype="<f4").reshape(-1, dim)
    truth = np.fromfile(data / "neighbors.i32", dtype="<i4").reshape(-1, meta["n_neighbors"])[: len(test), :K]
    t0 = time.perf_counter()
    idx = w.UpdatableIndex.open(a.index, threads=1)
    open_s = time.perf_counter() - t0
    idx.set_query_arguments(int(a.ef.split(",")[0]), int(a.rerank.split(",")[0]))
    idx.search(test[0:1], K)
    for q in test[:50]:
        idx.search(q[None], K)
    configs = []
    for rerank in map(int, a.rerank.split(",")):
        for ef in map(int, a.ef.split(",")):
            idx.set_query_arguments(ef, rerank)
            hits = 0
            for q, t in zip(test, truth):
                found = idx.search(q[None], K)[0]
                hits += len(set(int(x) for x in found) & set(int(x) for x in t))
            repeats = []
            for _ in range(a.repeats):
                lat = np.empty(len(test), dtype=np.int64)
                for i, q in enumerate(test):
                    qq = q[None]
                    t = time.perf_counter_ns()
                    idx.search(qq, K)
                    lat[i] = time.perf_counter_ns() - t
                order = np.sort(lat) / 1e3
                repeats.append({"qps": len(test) / (lat.sum() / 1e9), "p50_us": float(np.percentile(order, 50)), "p99_us": float(np.percentile(order, 99))})
            configs.append({"params": {"ef": ef, "rerank": rerank}, "recall_at_10": hits / (K * len(test)), "repeats": repeats, "load_1m": os.getloadavg()[0]})
    Path(a.out).write_text(json.dumps({"kind": "warm-python-serve", "open_s": open_s, "n_queries": len(test), "threads": 1, "configs": configs}, indent=1))


def report(a):
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from warmbench.common import bootstrap_ci
    import statistics
    res = json.loads(Path(a.results).read_text())
    trivial_doc = json.loads(Path(a.trivial).read_text())
    med = statistics.median

    def ci(v):
        return bootstrap_ci(v, level=0.95)

    out = {"schema_version": 1, "kind": "binding-overhead", "source_results": Path(a.results).name, "commit": res["commit"],
           "hardware": res["hardware"]["class"], "date": res["date"], "interval": "95% percentile bootstrap over rounds (per-ef) or trials (call overhead)"}
    # (a) fixed per-call cost
    t = trivial_doc["trials"]
    out["fixed_call_overhead"] = {"setup": {k: trivial_doc[k] for k in ("dim", "elements", "k", "ef", "num_threads", "calls_per_trial", "hnswlib", "python", "load_1m")}, "trials": len(t)}
    for name in ("native_searchKnn", "python_knn_query", "python_bare_pybind_call", "python_noop_call", "python_timer_floor", "timer_floor_native"):
        for stat in ("mean_ns", "median_ns", "p99_ns"):
            v = [x[name][stat] for x in t]
            out["fixed_call_overhead"].setdefault(name, {})[stat] = {"median": med(v), "ci": ci(v), "n": len(v)}
    diff = [x["python_knn_query"]["mean_ns"] - x["native_searchKnn"]["mean_ns"] for x in t]
    diff_med = [x["python_knn_query"]["median_ns"] - x["native_searchKnn"]["median_ns"] for x in t]
    out["fixed_call_overhead"]["python_minus_native_mean_us"] = {"median": med(diff) / 1e3, "ci": [c / 1e3 for c in ci(diff)], "n": len(diff)}
    out["fixed_call_overhead"]["python_minus_native_median_us"] = {"median": med(diff_med) / 1e3, "ci": [c / 1e3 for c in ci(diff_med)], "n": len(diff_med)}
    c = [x["python_knn_query_counters"] for x in t]
    out["fixed_call_overhead"]["thread_and_switch_counters"] = {
        "threads_before_after": sorted({(x["threads_before"], x["threads_after"]) for x in c}),
        "voluntary_ctxt_switches_per_call": med([x["voluntary_ctxt_switches"] / x["calls"] for x in c]),
        "nonvoluntary_ctxt_switches_per_call": med([x["nonvoluntary_ctxt_switches"] / x["calls"] for x in c])}
    # (b) per ef, same index file, native vs Python
    builds = {}
    for build_id, py in res["systems"]["hnswlib"]["builds"].items():
        cpp = res["systems"].get("hnswlib-cpp", {}).get("builds", {}).get(build_id)
        if not cpp:
            continue
        rows = []
        for i, cfg in enumerate(py["serve_rounds"][0]["configs"]):
            if any(i >= len(r["configs"]) for r in py["serve_rounds"] + cpp["serve_rounds"]) or len(py["serve_rounds"]) != len(cpp["serve_rounds"]):
                continue
            per = lambda rounds: [1e6 / med([x["qps"] for x in r["configs"][i]["repeats"]]) for r in rounds]  # us per query, one value per round
            tp, tc = per(py["serve_rounds"]), per(cpp["serve_rounds"])
            frac = [1 - c_ / p_ for p_, c_ in zip(tp, tc)]
            rows.append({"ef": cfg["params"]["ef"], "recall_python": cfg["recall_at_10"], "recall_cpp": cpp["serve_rounds"][0]["configs"][i]["recall_at_10"],
                         "python_us_per_query": {"median": med(tp), "ci": ci(tp), "per_round": tp},
                         "cpp_us_per_query": {"median": med(tc), "ci": ci(tc), "per_round": tc},
                         "outside_cpp_us": {"median": med([p_ - c_ for p_, c_ in zip(tp, tc)]), "ci": ci([p_ - c_ for p_, c_ in zip(tp, tc)])},
                         "fraction_outside_cpp": {"median": med(frac), "ci": ci(frac), "per_round": frac},
                         "qps_ratio_cpp_over_python": {"median": med([p_ / c_ for p_, c_ in zip(tp, tc)]), "ci": ci([p_ / c_ for p_, c_ in zip(tp, tc)])}})
        builds[build_id] = rows
    out["per_ef_same_index"] = builds
    if a.warm_python:
        wp = [json.loads(Path(p).read_text()) for p in a.warm_python]
        native = res["systems"]["warm"]["builds"]["vertex-d32"]["serve_rounds"]
        rows = []
        for i, cfg in enumerate(native[0]["configs"]):
            pys = [w["configs"][i] for w in wp if i < len(w["configs"]) and w["configs"][i]["params"] == cfg["params"]]
            if not pys:
                continue
            tn = [1e6 / med([x["qps"] for x in r["configs"][i]["repeats"]]) for r in native]
            tw = [1e6 / med([x["qps"] for x in c_["repeats"]]) for c_ in pys]
            rows.append({"params": cfg["params"], "recall_native": cfg["recall_at_10"], "recall_python": pys[0]["recall_at_10"],
                         "native_us_per_query": {"median": med(tn), "ci": ci(tn)}, "python_us_per_query": {"median": med(tw), "ci": ci(tw)},
                         "outside_native_us": med(tw) - med(tn)})
        out["warm_python_binding"] = {"native_rounds": len(native), "python_rounds": len(wp), "threads": 1, "rows": rows}
    Path(a.out).write_text(json.dumps(out, indent=1))
    print(json.dumps(out["fixed_call_overhead"]["python_minus_native_mean_us"]))


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("trivial")
    t.add_argument("--driver", required=True)
    t.add_argument("--out", required=True)
    t.add_argument("--dim", type=int, default=128)
    t.add_argument("--calls", type=int, default=300000)
    t.add_argument("--trials", type=int, default=10)
    w = sub.add_parser("warm-python")
    w.add_argument("--index", required=True)
    w.add_argument("--data", required=True)
    w.add_argument("--out", required=True)
    w.add_argument("--ef", default="10,16,24,32,48,64,96,128,192,256")
    w.add_argument("--rerank", default="32,64")
    w.add_argument("--repeats", type=int, default=2)
    r = sub.add_parser("report")
    r.add_argument("--results", required=True)
    r.add_argument("--trivial", required=True)
    r.add_argument("--warm-python", nargs="*", default=[])
    r.add_argument("--out", required=True)
    a = p.parse_args()
    {"trivial": trivial, "warm-python": warm_python, "report": report}[a.cmd](a)


if __name__ == "__main__":
    main()
