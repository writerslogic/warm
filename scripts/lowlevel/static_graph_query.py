# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Replay complete query sets against frozen native modules; this is not harness QPS."""

import argparse
import hashlib
import importlib.util
import json
import os
import platform
import time
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(8 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--module", type=Path, required=True)
    parser.add_argument("--index", type=Path, required=True)
    parser.add_argument("--queries", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--metric", choices=["angular", "euclidean", "ip"], default="angular")
    parser.add_argument("--k", type=int, default=100)
    parser.add_argument("--ef", type=int, nargs="+", default=[120, 150, 200, 400, 600, 800])
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--seed", type=int, default=898)
    args = parser.parse_args()
    if args.k < 1 or args.repeats < 1 or min(args.ef) < 1:
        parser.error("k, ef and repeats must be positive")

    spec = importlib.util.spec_from_file_location("writerslogic_warm", args.module.resolve())
    if spec is None or spec.loader is None:
        parser.error("module must name a native writerslogic_warm extension")
    warm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(warm)
    queries = np.asarray(pq.read_table(args.queries, columns=["emb"])["emb"].to_pylist(), dtype=np.float32)
    if queries.ndim != 2 or not len(queries):
        parser.error("queries must contain a non-empty rectangular emb column")
    receipt = {
        "kind": "single-process query replay including Python calls and result-array assignment",
        "platform": platform.platform(),
        "module_sha256": sha256(args.module),
        "index_sha256": sha256(args.index),
        "queries_sha256": sha256(args.queries),
        "query_array_sha256": hashlib.sha256(queries.tobytes()).hexdigest(),
        "shape": list(queries.shape),
        "metric": args.metric,
        "k": args.k,
        "seed": args.seed,
        "environment": {key: os.environ.get(key) for key in (
            "WARM_QGRAPH_BATCH_EST", "WARM_QGRAPH_PREFETCH", "WARM_QGRAPH_KERNEL",
            "WARM_QGRAPH_FRONTIER", "WARM_QGRAPH_RERANK_Z", "RAYON_NUM_THREADS",
            "OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS",
        )},
        "results": [],
    }
    index = warm.VectorIndex(metric=args.metric, threads=1)
    index.load(args.index)
    if args.k > len(index):
        parser.error("k must not exceed the index size")
    for ef in args.ef:
        index.set_query_arguments(ef=ef, rerank=ef)
        for query in queries:
            index.query(query, args.k)
        for repeat in range(args.repeats):
            order = np.random.default_rng(args.seed + repeat).permutation(len(queries))
            result = np.empty((len(queries), args.k), dtype=np.int64)
            started = time.perf_counter_ns()
            for position in order:
                result[position] = index.query(queries[position], args.k)
            elapsed = time.perf_counter_ns() - started
            row = {
                "ef": ef, "rep": repeat, "ns_per_query": elapsed / len(queries),
                "digest": hashlib.sha256(result.tobytes()).hexdigest(),
            }
            print(json.dumps(row), flush=True)
            receipt["results"].append(row)
    args.output.write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
