# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Mechanism ablation of the WARM-vs-hnswlib single-thread factor, on one host.

  run      build every WARM variant (one knob changed from the published build), then run interleaved serve
           rounds: each round runs every WARM variant and the hnswlib rows once, order rotated per round,
           each in a fresh pinned process, exactly as `warm-bench run` does for the headline rows.
           Writes <out>/ablation-<dataset>-<hardware class>.json (+ .latency.npz).
  report   read those files (any number of hosts) and the matching `warm-bench run` headline files and write
           <out>/mechanism-<dataset>-<hardware class>.json: per recall target the QPS of every variant, the
           factor of the base build over each variant with a paired-by-round bootstrap interval, the factor of
           base over hnswlib, and distance evaluations per query.

Operating points are chosen exactly as in warmbench.analyze (pooled over rounds, selection half of the queries,
QPS reported on the other half). A variant that never reaches a target recall is recorded as not reached.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import numpy as np  # noqa: E402

from warmbench import analyze, common, data, rawdata, registry, runner  # noqa: E402
from warmbench.common import run_measured  # noqa: E402

TARGETS = (0.90, 0.95, 0.99)
RERANKS = "0,32,64"
EF = {"euclidean": "10,16,24,32,48,64,96,128,192,256,384,512", "angular": "16,32,64,96,128,192,256,384,512,736,1024,1536,2048"}
# variant -> overrides of runner.WARM_ANN_BUILDS[metric]. Graph-identical variants share one graph cache.
VARIANTS = {
    "base": {},
    "bits-other": {"vertex_bits": "other"},  # 8-bit base -> 4-bit codes; 4-bit base -> 8-bit codes
    "no-residual": {"residual": False},
    "degree-32": {"degree": 32},
    "build-ef-200": {"build_ef": 200},
    "degree-32-build-ef-200": {"degree": 32, "build_ef": 200},
    "no-reorder": {"reorder": False},
    "no-align": {"align_rows": False},
    "id-4-bytes": {"id_bytes": 4},
    "layout-off": {"reorder": False, "align_rows": False, "id_bytes": 4},
    "full-build-search": {"quantized_search": False},
}
HNSW_BUILD = {"euclidean": "m32e500", "angular": "m32"}


def resolve(metric, name):
    build = dict(runner.WARM_ANN_BUILDS[metric])
    for key, value in VARIANTS[name].items():
        build[key] = (12 - build["vertex_bits"]) if value == "other" else value
    build["ef"], build["rerank"] = os.environ.get("MECH_EF") or EF[metric], os.environ.get("MECH_RERANK") or RERANKS
    if not build["residual"]:
        build["residual_bits"] = 8
    return build


def graph_key(build):
    return f"d{build['degree']}-e{build['build_ef']}-q{int(build['quantized_search'])}"


def build_variant(directory, meta, threads, work, name, build):
    workdir = work / name
    shutil.rmtree(workdir, ignore_errors=True)
    workdir.mkdir(parents=True)
    cache = work / f"graph-{graph_key(build)}.bin"
    out = work / "warm-build.json"
    out.unlink(missing_ok=True)
    args = runner._warm_ann_args({**build, "ef": "10", "rerank": "0"})
    cmd = [str(runner.warm_binary("public-bench")), "ann-qgraph", "--data", str(directory), "--out", str(out), "--metric", meta["metric"], *args,
           "--repeats", "1", "--max-load", "1000000", "--max-wait-secs", "0", "--graph-cache", str(cache), "--save-updates", str(workdir / "index.updates")]
    stdout, stderr, wall, peak = run_measured(cmd, env={"RAYON_NUM_THREADS": str(threads)}, timeout=14400)
    raw = json.loads(out.read_text())
    out.unlink()
    return {"build_s": raw["build"]["build_secs"], "wall_s": wall, "peak_rss_bytes": peak, "graph_hash": raw["build"]["graph_hash"],
            "in_memory_index_bytes": raw["index_bytes"], "residual_bytes": raw["residual_bytes"], "index_bytes": common.dir_bytes(workdir)}


def estimates_pass(directory, meta, work, name, build, threads):
    """Code estimates per query at every (ef, rerank) of the sweep, from `ann-qgraph` on the cached graph. The search is
    deterministic, so counts need no timing; the build half comes from the graph cache."""
    out = work / "warm-estimates.json"
    out.unlink(missing_ok=True)
    cmd = [str(runner.warm_binary("public-bench")), "ann-qgraph", "--data", str(directory), "--out", str(out), "--metric", meta["metric"], *runner._warm_ann_args(build),
           "--repeats", "1", "--max-load", "1000000", "--max-wait-secs", "0", "--graph-cache", str(work / f"graph-{graph_key(build)}.bin")]
    run_measured(cmd, env={"RAYON_NUM_THREADS": str(threads)}, timeout=14400)
    rows = json.loads(out.read_text())["sweep"]
    out.unlink()
    return {f"{r['params']['ef']}|{r['params'].get('rerank', 0)}": r["mean_exact_evals_per_query"] for r in rows}


def warm_round(directory, workdir, build, tag):
    out = runner.WORK / f"warm-serve-{tag}.json"
    out.unlink(missing_ok=True)
    cmd = [str(runner.warm_binary("public-bench")), "ann-updates", "--action", "serve", "--index", str(workdir / "index.updates"), "--data", str(directory),
           "--out", str(out), "--ef", build["ef"], "--rerank", build["rerank"], "--repeats", "1"]
    stdout, stderr, wall, peak = run_measured(runner._pin(cmd, "serve"), env=runner.SINGLE_THREAD, timeout=14400)
    raw = json.loads(out.read_text())
    out.unlink()
    raw["wall_s"], raw["peak_rss_bytes"] = wall, peak
    return raw


def cmd_run(a):
    runner.set_pinning(a.pin_cpus)
    dataset = a.dataset
    directory, meta = data.ann_subset(dataset, a.n_train, a.n_test)
    rawdata.require_digests(meta, ("train_sha256", "test_sha256"))
    metric = meta["metric"]
    work = runner.WORK / f"mech-{dataset}"
    work.mkdir(parents=True, exist_ok=True)
    threads = a.threads
    names = a.variants or list(VARIANTS)
    hw = common.hardware()
    results = {"schema_version": runner.SCHEMA_VERSION, "kind": "ann-mechanism", "suite": "ablation", "source": common.source_record("heavy"),
               "commit": common.source_record("heavy")["commit"], "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "hardware": hw,
               "dataset": {k: meta[k] for k in ("name", "metric", "n_train", "n_test", "dim") if k in meta} | {"train_sha256": meta.get("train_sha256"), "test_sha256": meta.get("test_sha256")},
               "protocol": {"k": 10, "rounds": a.rounds, "repeats_per_round": 1, "build_threads": threads, "search_threads": 1, "queries": meta["n_test"],
                            "timing": "one query at a time, single client thread, per-query nanoseconds inside the serving process",
                            "interleaving": "each round runs every variant and the hnswlib rows once in a fresh pinned process, order rotated per round",
                            "recall_targets": list(TARGETS), "cpu_pinning": runner.pinning_record(), "serve_rerank_sweep": os.environ.get("MECH_RERANK") or RERANKS,
                            "ef_sweep": os.environ.get("MECH_EF") or EF[metric], "selection": "pooled-median-QPS argmax over rows with recall >= target on the even-indexed queries; QPS reported on the odd-indexed queries"},
               "variants": {}, "systems": {}}
    raw = rawdata.RawStore()
    builds = {}
    for name in names:
        build = resolve(metric, name)
        builds[name] = build
        results["variants"][name] = {"params": build, "build": build_variant(directory, meta, threads, work, name, build)}
        common.log(f"built {name}: {results['variants'][name]['build']['build_s']:.0f}s")
    # hnswlib rows: the Python binding and the native C++ driver over the same index file
    hb = HNSW_BUILD[metric]
    envb = {k: str(threads) for k in runner.SINGLE_THREAD}
    hdir = runner.WORK / f"mech-{dataset}-hnswlib"
    hres = runner._worker("hnswlib", ["build", "--system", "hnswlib", "--build", hb, "--data", str(directory), "--workdir", str(hdir), "--threads", str(threads)], envb)
    env_cpp = {**runner.SINGLE_THREAD, "HNSWLIB_CPP_DRIVER": runner.hnswlib_cpp_driver()}
    plan = [("warm", n) for n in names] + [("hnswlib", hb), ("hnswlib-cpp", hb)]
    serves = {k: [] for k in plan}
    for r in range(a.rounds):
        for system, key in runner._rotate(plan, r):
            label = f"{system}/{key}/round{r}"
            if system == "warm":
                round_out = runner._clean(label, lambda: warm_round(directory, work / key, builds[key], key))
            else:
                args = ["serve", "--system", system, "--build", key, "--data", str(directory), "--workdir", str(hdir), "--repeats", "1"]
                round_out = runner._clean(label, lambda: runner._worker(system, args, env_cpp if system == "hnswlib-cpp" else runner.SINGLE_THREAD))
            serves[(system, key)].append(rawdata.split_serve_round(round_out, raw, label))
    for (system, key), rounds in serves.items():
        entry = results["systems"].setdefault(system if system != "warm" else "warm-variants", {"builds": {}})
        entry["builds"][key] = {"serve_rounds": rounds}
    results["systems"]["hnswlib"]["builds"][hb]["build_runs"] = [hres]
    results["systems"]["hnswlib"]["builds"][hb]["params"] = runner._adapter_class("hnswlib").BUILDS[hb]
    results["hnswlib_distance_evaluations"] = count_hnsw_evals(directory, meta, hdir, hb, serves[("hnswlib-cpp", hb)])
    for name in names:
        results["variants"][name]["estimates_per_query"] = estimates_pass(directory, meta, work, name, builds[name], threads)
    results["load_log"] = list(runner.LOAD_LOG)
    out_dir = Path(a.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"ablation-{dataset}-n{meta['n_train']}-{hw['class']}.json"
    results["raw_latency"] = raw.write(path)
    common.write_json(path, results)
    print(path)


def count_hnsw_evals(directory, meta, hdir, build_id, rounds):
    """Distance computations per query of the native hnswlib search at every ef of the sweep (driver `evals`)."""
    driver = runner.hnswlib_cpp_driver()
    n, dim = meta["n_train"], meta["dim"]
    test = np.fromfile(Path(directory) / "test.f32", dtype="<f4").reshape(-1, dim)
    angular = meta["metric"] == "angular"
    if angular:
        test = test / np.linalg.norm(test, axis=1, keepdims=True)
    queries = Path(hdir) / "queries-evals.f32"
    test.astype("<f4").tofile(queries)
    efs = sorted({max(c["params"]["ef"], 10) for c in rounds[0]["configs"]})
    stdout = subprocess.run([driver, "count", str(Path(hdir) / "index.bin"), str(queries), str(len(test)), str(dim), "ip" if angular else "l2", str(n + 1000), *map(str, efs)],
                            check=True, capture_output=True, text=True).stdout
    out = {ef: {"distance_computations_per_query": float(calls)} for ef, calls in (line.split() for line in stdout.splitlines())}
    return out


# ---------------------------------------------------------------- report

def _rows(rounds, idx, half):
    """Per-round QPS (median over the round's repeats) of config `idx` on `half`."""
    return [float(np.median([x["qps"] for x in analyze._half(r["configs"][idx], half)["repeats"]])) for r in rounds]


def _params(config):
    return config["params"]


def pick(rounds, target, allow=None):
    """(idx, recall) of the best-pooled-QPS config whose min recall over rounds on the selection half reaches `target`."""
    best, closest = None, (-1.0, None)
    for idx in range(len(rounds[0]["configs"])):
        if allow and not allow(_params(rounds[0]["configs"][idx])):
            continue
        views = [analyze._half(r["configs"][idx], "select") for r in rounds]
        recall = min(v["recall_at_10"] for v in views)
        closest = max(closest, (recall, idx))
        if recall < target:
            continue
        pooled = float(np.median([x["qps"] for v in views for x in v["repeats"]]))
        if best is None or pooled > best[0]:
            best = (pooled, idx, recall)
    return best, closest


def operating_point(rounds, target, allow=None, estimates=None):
    best, closest = pick(rounds, target, allow)
    if best is None:
        return {"reached": False, "best_select_recall": closest[0]}
    _, idx, recall = best
    report = [analyze._half(r["configs"][idx], "report") for r in rounds]
    per_round = _rows(rounds, idx, "report")
    config = rounds[0]["configs"][idx]
    return {"reached": True, "config": idx, "params": _params(config), "select_recall": recall, "report_recall": float(np.median([v["recall_at_10"] for v in report])),
            "qps_by_round": per_round, "qps_median": float(np.median(per_round)), "qps_ci": common.bootstrap_ci(per_round, level=0.95),
            "estimates_per_query": (estimates or {}).get(f"{config['params']['ef']}|{config['params'].get('rerank', 0)}")}


def interpolated(builds, target, allow=None):
    """QPS at exactly `target` recall, per round: each round's configs form a recall/QPS upper frontier and QPS is interpolated
    linearly in log-QPS between the frontier points that bracket the target (no overshoot of the ef grid). `builds` is
    {id: [serve rounds]}; the build with the best median is kept. Recall is the whole-query-set recall of the config."""
    best = None
    for build_id, rounds in builds.items():
        if not rounds:
            continue
        per_round = []
        for r in rounds:
            pts = sorted((c["recall_at_10"], float(np.median([x["qps"] for x in c["repeats"]]))) for c in r["configs"] if not allow or allow(c["params"]))
            front, top = [], 0.0
            for rec, q in reversed(pts):  # highest recall first: keep points faster than every higher-recall point
                if q > top:
                    front.append((rec, q))
                    top = q
            front.sort()
            if not front or front[-1][0] < target:
                per_round = None
                break
            hi = next(i for i, (rec, _) in enumerate(front) if rec >= target)
            if front[hi][0] == target or hi == 0:
                per_round.append(front[hi][1])
            else:
                (r0, q0), (r1, q1) = front[hi - 1], front[hi]
                t = (target - r0) / (r1 - r0)
                per_round.append(float(np.exp(np.log(q0) + t * (np.log(q1) - np.log(q0)))))
        if per_round and (best is None or np.median(per_round) > np.median(best["qps_by_round"])):
            best = {"reached": True, "build": build_id, "qps_by_round": per_round, "qps_median": float(np.median(per_round))}
    return best or {"reached": False}


def factor(num, den):
    if not (num["reached"] and den["reached"]):
        return {"reached": False}
    if len(num["qps_by_round"]) != len(den["qps_by_round"]):
        raise SystemExit("paired factor needs the same rounds on both sides")
    point = float(np.median(num["qps_by_round"]) / np.median(den["qps_by_round"]))
    return {"reached": True, "factor": point, "ci95": common.round_ratio_ci(num["qps_by_round"], den["qps_by_round"], paired=True), "rounds": len(num["qps_by_round"])}


def cmd_evals(a):
    """Recount hnswlib's distance calls per query for an existing ablation file (needs its hnswlib index in the work directory)."""
    path = Path(a.file)
    r = json.load(open(path))
    directory, meta = data.ann_subset(r["dataset"]["name"], a.n_train, a.n_test)
    hb = next(iter(r["systems"]["hnswlib-cpp"]["builds"]))
    hdir = runner.WORK / f"mech-{r['dataset']['name']}-hnswlib"
    r["hnswlib_distance_evaluations"] = count_hnsw_evals(directory, meta, hdir, hb, r["systems"]["hnswlib-cpp"]["builds"][hb]["serve_rounds"])
    common.write_json(path, r)
    print(path)


def cmd_report(a):
    for path in a.files:
        r = json.load(open(path))
        warm = r["systems"]["warm-variants"]["builds"]
        out = {"source_file": Path(path).name, "hardware": r["hardware"], "dataset": r["dataset"], "commit": r["commit"], "rounds": r["protocol"]["rounds"], "targets": {}}
        hb = next(iter(r["systems"]["hnswlib-cpp"]["builds"]))
        for target in TARGETS:
            est = r["variants"]
            ops = {n: operating_point(b["serve_rounds"], target, None, est[n]["estimates_per_query"]) for n, b in warm.items()}
            base = ops["base"]
            cpp = operating_point(r["systems"]["hnswlib-cpp"]["builds"][hb]["serve_rounds"], target)
            py = operating_point(r["systems"]["hnswlib"]["builds"][hb]["serve_rounds"], target)
            sub = {"rerank-off (serve-time, base index)": ("base", lambda p: p.get("rerank") == 0),
                   "rerank-on only (base index)": ("base", lambda p: p.get("rerank", 0) > 0)}
            for label, (n, allow) in sub.items():
                ops[label] = operating_point(warm[n]["serve_rounds"], target, allow, est[n]["estimates_per_query"])
            evals = r["hnswlib_distance_evaluations"]
            cpp_evals = evals.get(str(max(cpp["params"]["ef"], 10)), {}).get("distance_computations_per_query") if cpp["reached"] else None
            out["targets"][str(target)] = {
                "operating_points": ops, "hnswlib_cpp": cpp | {"build": hb, "distance_computations_per_query": cpp_evals}, "hnswlib_python": py | {"build": hb},
                "base_over_hnswlib_cpp": factor(base, cpp),
                "timing_boundary": {"hnswlib_cpp_over_hnswlib_python": factor(cpp, py)},
                "base_over_hnswlib_cpp_interpolated": factor(interpolated({"base": warm["base"]["serve_rounds"]}, target), interpolated({hb: r["systems"]["hnswlib-cpp"]["builds"][hb]["serve_rounds"]}, target)),
                "mechanism": {n: {"base_over_variant": factor(base, o), "variant_over_base": factor(o, base),
                                  "base_over_variant_interpolated": factor(interpolated({"b": warm["base"]["serve_rounds"]}, target), interpolated({"v": warm[n]["serve_rounds"]}, target)) if n in warm else None,
                                  "estimates_per_query_variant": o.get("estimates_per_query"), "estimates_per_query_base": base.get("estimates_per_query")}
                              for n, o in ops.items() if n != "base"},
                "evaluation_ratio_hnswlib_over_warm": (cpp_evals / base["estimates_per_query"]) if cpp_evals and base.get("estimates_per_query") else None,
            }
        dest = Path(path).with_name(Path(path).name.replace("ablation-", "mechanism-"))
        common.write_json(dest, out)
        print(dest)


def best_build(builds, target):
    """Operating point of the build (and config) with the best pooled selection-half QPS at `target`; the build id is recorded."""
    best = None
    for build_id, build in builds.items():
        if not build["serve_rounds"]:
            continue
        found, _ = pick(build["serve_rounds"], target)
        if found and (best is None or found[0] > best[0]):
            best = (found[0], build_id)
    if best is None:
        return {"reached": False}
    return operating_point(builds[best[1]]["serve_rounds"], target) | {"build": best[1]}


def cmd_headline(a):
    """Paired factors from `warm-bench run` files: WARM and every comparator ran in the same session, rotated per round."""
    for path in a.files:
        r = json.load(open(path))
        systems = r["systems"]
        rounds = r["protocol"]["rounds"]
        out = {"source_file": Path(path).name, "hardware": r["hardware"], "dataset": r["dataset"], "commit": r["commit"], "rounds": rounds, "targets": {},
               "pairing": "WARM and every comparator ran each round in the same session, order rotated per round; the factor is median-over-rounds QPS ratio, "
                          "interval is a percentile bootstrap over rounds with one round draw for both sides"}
        for target in TARGETS:
            warm = best_build(systems["warm"]["builds"], target)
            iw = interpolated({b: v["serve_rounds"] for b, v in systems["warm"]["builds"].items()}, target)
            row = {"warm": warm, "warm_interpolated": iw}
            for system, entry in systems.items():
                if system == "warm" or system not in ("hnswlib", "hnswlib-cpp", "faiss-hnsw", "usearch"):
                    continue
                theirs = best_build(entry["builds"], target)
                row[system] = {"operating_point": theirs, "warm_over": factor(warm, theirs),
                               "warm_over_interpolated": factor(iw, interpolated({b: v["serve_rounds"] for b, v in entry["builds"].items()}, target))}
            out["targets"][str(target)] = row
        dest = Path(path).with_name("factor-" + Path(path).name)
        common.write_json(dest, out)
        print(dest)


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--dataset", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--rounds", type=int, default=8)
    r.add_argument("--threads", type=int, default=8)
    r.add_argument("--n-train", type=int, help="subset size for a plumbing check; the published runs use the full set")
    r.add_argument("--n-test", type=int, default=10_000)
    r.add_argument("--pin-cpus", default="2")
    r.add_argument("--variants", nargs="*", choices=list(VARIANTS))
    q = sub.add_parser("report")
    q.add_argument("files", nargs="+")
    e = sub.add_parser("evals")
    e.add_argument("file")
    e.add_argument("--n-train", type=int)
    e.add_argument("--n-test", type=int, default=10_000)
    h = sub.add_parser("headline")
    h.add_argument("files", nargs="+")
    a = p.parse_args()
    {"run": cmd_run, "report": cmd_report, "headline": cmd_headline, "evals": cmd_evals}[a.cmd](a)


if __name__ == "__main__":
    main()
