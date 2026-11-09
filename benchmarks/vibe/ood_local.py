# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Compare WARM adapter build arms on one prepared diagnostic dataset.

Loads benchmarks/vibe/warm/module.py unchanged, builds every arm once, then times all arms
interleaved: for each query configuration and repeat the arm order rotates, so machine load
hits every arm alike. One query at a time on one thread. Recall is VIBE's distance-tolerant
recall@k (epsilon 1e-3) from run.score, with strict neighbor-ID recall kept beside it.
A diagnostic on a corpus prefix, not a VIBE result; no CPU pinning (macOS), so QPS is relative.

An arm is `degree:build_ef:code_bits:query_aware[:escape:reach:projected_degree:training_neighbors:training_ef:learn_queries]`,
for example `64:500:8:1`.
"""

import argparse
import hashlib
import importlib.util
import json
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import h5py
import numpy as np
import yaml

HERE = Path(__file__).resolve().parent
TARGETS = (0.90, 0.95, 0.99, 1.0)
# Wider than the registered grid so a 1.00 target that the registered grid misses is still visible.
EXTENDED_EF = (1500, 2000)
EXTENDED_RERANK = (1000,)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def registered_grid(metric):
    """The ef x rerank pairs pre-registered for this metric section in warm/config.yml."""
    section = yaml.safe_load((HERE / "warm" / "config.yml").read_text())["float"][metric][0]
    query = section["run_groups"]["base"]["query_args"]
    return [(ef, rr) for ef in query["ef"] for rr in query["rerank"]]


ARM_FIELDS = ("degree", "build_ef", "code_bits", "query_aware", "escape_edges", "reach_edges",
              "projected_degree", "training_neighbors", "training_ef", "learn_queries")
ARM_DEFAULTS = (None, None, None, None, 0, 0, 32, 100, 0, 32768)


def parse_arm(text):
    """Colon-separated ARM_FIELDS; trailing fields default to the submitted adapter's values."""
    values = [int(v) for v in text.split(":")]
    if not 4 <= len(values) <= len(ARM_FIELDS):
        raise ValueError(f"arm {text!r}: expected 4 to {len(ARM_FIELDS)} fields")
    values += list(ARM_DEFAULTS[len(values):])
    arm = dict(zip(ARM_FIELDS, values))
    arm["query_aware"] = bool(arm["query_aware"])
    arm["escape_edges"] = bool(arm["escape_edges"])
    arm["reach_edges"] = bool(arm["reach_edges"])
    arm["name"] = text
    return arm


def run(args):
    if args.out.exists():
        raise FileExistsError(args.out)
    sys.path.insert(0, str(args.vibe.resolve()))
    harness = load("vibe_run", HERE / "run.py")
    adapter_module = load("vibe.algorithms.warm.module", args.module)
    manifest = json.loads(args.data.with_suffix(".json").read_text())
    if harness.sha(args.data) != manifest["sha256"]:
        raise ValueError("input hash mismatch")
    with h5py.File(args.data, "r") as handle:
        train, queries = handle["train"][:], handle["test"][:]
        truth, distances = handle["neighbors"][:, :args.k], handle["distances"][:, :args.k]
        metric = str(handle.attrs["distance"])
        learn = handle["learn"][:] if "learn" in handle else None
    if args.max_queries:
        queries, truth, distances = queries[:args.max_queries], truth[:args.max_queries], distances[:args.max_queries]
    if not np.isfinite(train).all() or not np.isfinite(queries).all():
        raise ValueError("nonfinite input")
    registered = set(registered_grid(metric))
    efs = sorted({ef for ef, _ in registered} | set(EXTENDED_EF))
    rerank = sorted({rr for _, rr in registered} | set(EXTENDED_RERANK))
    grid = [(ef, rr, 0, 0) for ef in efs for rr in rerank]
    if args.points:
        grid = [tuple(int(v) for v in (p.split(":") + ["0", "0"])[:4]) for p in args.points.split(",")]

    arms = [parse_arm(a) for a in args.arms]

    def build(arm):
        index = adapter_module.WARM(metric, arm["degree"], arm["build_ef"], arm["code_bits"], arm["query_aware"],
                                        arm["escape_edges"], arm["reach_edges"], arm["projected_degree"],
                                        arm["training_neighbors"], arm["training_ef"], arm["learn_queries"])
        started = time.perf_counter()
        cached = args.cache / (hashlib.sha256((str(args.data) + arm["name"]).encode()).hexdigest()[:16] + ".warm") if args.cache else None
        if cached is not None and cached.exists():
            index.load_index(cached)
            arm["build_seconds"] = 0.0
            arm["trained"] = bool(index.trained)
            print(f"loaded {arm['name']} from {cached.name}", flush=True)
            return index
        if learn is not None:
            index.fit_ood(train.copy(), learn, None)
        else:
            index.fit(train.copy())
        arm["build_seconds"] = time.perf_counter() - started
        if cached is not None:
            args.cache.mkdir(parents=True, exist_ok=True)
            index.save_index(cached)
        arm["trained"] = bool(index.trained)
        arm["learn_queries"] = int(index.learn_queries)
        print(f"built {arm['name']} in {arm['build_seconds']:.1f}s trained={arm['trained']}", flush=True)
        return index

    # Each build is one native thread that releases the GIL, so arms may build side by side.
    with ThreadPoolExecutor(max_workers=args.build_threads) as pool:
        built = list(pool.map(build, arms))

    rows = {arm["name"]: [] for arm in arms}
    for ef, rr, reservoir, patience in grid:
        runs = {arm["name"]: {"qps": [], "ids": None, "load": []} for arm in arms}
        for repeat in range(args.repeats):
            order = list(range(len(arms)))
            order = order[repeat % len(order):] + order[:repeat % len(order)]
            for position in order:
                index, name = built[position], arms[position]["name"]
                index.set_query_arguments(ef, rr, patience, reservoir)
                index.query(queries[0], args.k)
                elapsed, answers = [], []
                runs[name]["load"].append(os.getloadavg()[0])
                for query in queries:
                    t0 = time.perf_counter_ns()
                    answer = index.query(query, args.k)
                    elapsed.append(time.perf_counter_ns() - t0)
                    found = np.asarray(answer)
                    if found.ndim != 1 or found.dtype.kind not in "iu" or len(found) > args.k:
                        raise ValueError("adapter returned invalid IDs")
                    answers.append(np.pad(found.astype(np.int64), (0, args.k - len(found)), constant_values=-1))
                current = np.asarray(answers)
                if runs[name]["ids"] is not None and not np.array_equal(current, runs[name]["ids"]):
                    raise ValueError("search results changed across timing repeats")
                runs[name]["ids"] = current
                runs[name]["qps"].append(len(queries) * 1e9 / sum(elapsed))
        for arm in arms:
            name = arm["name"]
            row = harness.score(runs[name]["ids"], train, queries, truth, distances, metric)
            del row["query_recalls"], row["strict_query_recalls"]
            row.update(ef=ef, rerank=rr, reservoir=reservoir, patience=patience, registered=(ef, rr) in registered and not reservoir and not patience, qps_runs=runs[name]["qps"],
                       median_qps=float(np.median(runs[name]["qps"])), load_1m=runs[name]["load"])
            rows[name].append(row)
        print(f"ef={ef} rerank={rr} reservoir={reservoir} patience={patience}: " + " | ".join(
            f"{arm['name']} {rows[arm['name']][-1]['recall']:.4f}@{rows[arm['name']][-1]['median_qps']:.0f}"
            for arm in arms), flush=True)

    summary = {}
    for arm in arms:
        name = arm["name"]
        summary[name] = {"max_recall": max(r["recall"] for r in rows[name]),
                         "max_recall_registered": max(r["recall"] for r in rows[name] if r["registered"])}
        for target in TARGETS:
            for scope, pool in (("registered", [r for r in rows[name] if r["registered"]]), ("extended", rows[name])):
                eligible = [r for r in pool if r["recall"] >= target]
                best = max(eligible, key=lambda r: r["median_qps"]) if eligible else None
                summary[name][f"{target}:{scope}"] = None if best is None else {
                    "ef": best["ef"], "rerank": best["rerank"], "reservoir": best["reservoir"], "patience": best["patience"], "recall": best["recall"],
                    "strict": best["strict_id_recall"], "median_qps": best["median_qps"]}
    result = {"schema_version": 1, "input": manifest, "metric": metric, "k": args.k, "repeats": args.repeats,
              "queries": len(queries), "train_rows": len(train), "arms": arms, "summary": summary, "rows": rows,
              "adapter_sha256": harness.sha(args.module), "harness_sha256": harness.sha(Path(__file__)),
              "host_load_start": os.getloadavg(),
              "scope": "exploratory corpus prefix, interleaved arms, macOS unpinned; relative QPS only"}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
    for name, s in summary.items():
        print(name, json.dumps(s))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vibe", type=Path, required=True, help="VIBE checkout providing vibe.algorithms.base")
    parser.add_argument("--module", type=Path, default=HERE / "warm" / "module.py")
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--arms", nargs="+", required=True)
    parser.add_argument("--k", type=int, default=100)
    parser.add_argument("--points", help="comma-separated ef:rerank[:reservoir[:patience]] list replacing the default grid")
    parser.add_argument("--cache", type=Path, help="directory of saved builds; a build already there is loaded, not rebuilt")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--max-queries", type=int, default=0, help="score and time only the first N queries")
    parser.add_argument("--build-threads", type=int, default=1, help="arms built concurrently (timing is unaffected)")
    run(parser.parse_args())
