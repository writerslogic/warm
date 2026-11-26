# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Time one VIBE OOD build over a grid of (ef, rerank, probes, probe_only) points.

Reads a VIBE HDF5 file (train, test, neighbors, learn), builds the index the adapter builds
(seeded learn sample, `fit` with training queries) and optionally query lists, then times one
query at a time on one thread. Reports strict neighbour-ID recall@k (the minimum over repeats)
and median QPS, the protocol of the committed query_aware_graph_v1 pareto. Test queries and
truth never build or tune anything.
"""

import argparse
import json
import statistics
import time
from pathlib import Path

import h5py
import numpy as np
from writerslogic_warm import VectorIndex

LEARN_SEED = 24301


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--degree", type=int, default=32)
    p.add_argument("--build-ef", type=int, default=500)
    p.add_argument("--projected-degree", type=int, default=32)
    p.add_argument("--learn", type=int, default=32768)
    p.add_argument("--training-neighbors", type=int, default=100)
    p.add_argument("--query-lists", type=int, default=0)
    p.add_argument("--list-neighbors", type=int, default=100)
    p.add_argument("--rows", type=int, default=0, help="corpus prefix; truth is recomputed exactly")
    p.add_argument("--cache", type=Path)
    p.add_argument("--k", type=int, default=100)
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--points", required=True, help="ef:rerank[:probes[:probe_only]],...")
    a = p.parse_args()

    with h5py.File(a.data, "r") as h:
        train, queries = h["train"][:], h["test"][:]
        truth = h["neighbors"][:, : a.k]
        metric = str(h.attrs["distance"])
        learn = h["learn"][:] if "learn" in h else None
    if a.rows:
        train = train[: a.rows]
    if metric == "ip":
        # The committed protocol: float64 original inner product, descending score, base-ID ties.
        scores = queries.astype(np.float64) @ train.astype(np.float64).T
        truth = np.argsort(-scores, axis=1, kind="stable")[:, : a.k]
    order = np.random.default_rng(LEARN_SEED).permutation(len(learn))[: a.learn]
    training = np.ascontiguousarray(learn[order], dtype=np.float32)

    index = VectorIndex({"ip": "ip", "euclidean": "euclidean"}.get(metric, "angular"), degree=a.degree,
                        build_ef=a.build_ef, threads=1, code_bits=8, direct_ip=metric == "ip")
    key = f"{a.data.stem}-{a.rows}-{a.degree}-{a.build_ef}-{a.projected_degree}-{a.learn}-{a.training_neighbors}-{a.query_lists}-{a.list_neighbors}.warm"
    cached = a.cache / key if a.cache else None
    started = time.perf_counter()
    if cached is not None and cached.exists():
        index.load(str(cached))
    else:
        options = dict(projected_degree=a.projected_degree, training_neighbors=a.training_neighbors,
                       training_ef=0, training_limit=max(a.learn, len(training)))
        if a.query_lists:
            options.update(query_lists=a.query_lists, list_neighbors=a.list_neighbors)
        index.fit(np.ascontiguousarray(train, dtype=np.float32), training_queries=training, **options)
        if cached is not None:
            a.cache.mkdir(parents=True, exist_ok=True)
            index.save(str(cached))
    build = time.perf_counter() - started
    memory = {}
    try:
        for line in Path("/proc/self/status").read_text().splitlines():
            if line.startswith(("VmHWM", "VmRSS")):
                memory[line.split(":")[0]] = int(line.split()[1]) * 1024
    except OSError:
        pass
    print(f"build/load {build:.1f}s {memory}", flush=True)

    rows = []
    for point in a.points.split(","):
        f = point.split(":")
        ef, rr = int(f[0]), int(f[1])
        probes = int(f[2]) if len(f) > 2 else 0
        only = bool(int(f[3])) if len(f) > 3 else False
        index.set_query_arguments(ef, rr, 0, 0, probes, only)
        index.query(queries[0], a.k)
        qps, recalls = [], []
        for _ in range(a.repeats):
            ids = []
            t0 = time.perf_counter()
            for q in queries:
                ids.append(index.query(q, a.k))
            qps.append(len(queries) / (time.perf_counter() - t0))
            hit = sum(len(set(int(x) for x in r) & set(int(x) for x in t)) for r, t in zip(ids, truth))
            recalls.append(hit / (len(queries) * a.k))
        row = dict(ef=ef, rerank=rr, probes=probes, probe_only=only, recall=min(recalls),
                   qps=statistics.median(qps), qps_min=min(qps), qps_max=max(qps))
        rows.append(row)
        print(json.dumps(row), flush=True)
    a.out.write_text(json.dumps(dict(args={k: str(v) for k, v in vars(a).items()}, build_seconds=build, memory_bytes=memory, rows=rows), indent=1))


if __name__ == "__main__":
    main()
