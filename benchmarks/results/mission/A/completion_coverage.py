# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Development-only oracle bound for unpruned nearest-neighbor completion lists."""

import argparse
import csv
import hashlib
import json
import platform
import subprocess
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq


def load(path, count):
    chunks = []
    remaining = count
    for batch in pq.ParquetFile(path).iter_batches(batch_size=min(4096, count), columns=["emb"]):
        selected = np.asarray(batch.column(0).slice(0, remaining).to_pylist(), dtype=np.float64)
        if selected.ndim != 2 or not 1 <= selected.shape[1] <= 4096:
            raise ValueError("invalid vector dimensions")
        chunks.append(selected)
        remaining -= len(selected)
        if not remaining:
            break
    if remaining:
        raise ValueError(f"insufficient rows in {path.name}")
    rows = np.concatenate(chunks)
    if not np.isfinite(rows).all():
        raise ValueError("nonfinite vectors")
    scales = np.max(np.abs(rows), axis=1)
    if np.any(scales == 0):
        raise ValueError("zero cosine vectors")
    rows /= scales[:, None]
    rows /= np.sqrt(np.sum(rows * rows, axis=1))[:, None]
    return rows


def top(scores, count):
    cutoff = np.partition(scores, len(scores) - count)[len(scores) - count]
    candidates = np.flatnonzero(scores >= cutoff)
    return candidates[np.lexsort((candidates, -scores[candidates]))][:count]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--experiment", required=True)
    parser.add_argument("--rows", type=int, default=100000)
    parser.add_argument("--queries", type=int, default=32)
    args = parser.parse_args()
    if not 257 <= args.rows <= 100000 or not 1 <= args.queries <= 32:
        parser.error("development allocation bounds: rows 257..100000, queries 1..32")
    if not args.experiment.startswith("E") or not args.experiment[1:].isdigit():
        parser.error("experiment must be E followed by digits")
    if args.output.exists():
        parser.error("refusing to overwrite previous evidence")
    if not args.ledger.is_file():
        parser.error("mission ledger must exist")
    base = load(args.data / "shuffle_train.parquet", args.rows)
    queries = load(args.data / "test.parquet", args.queries)
    if base.shape[1] != queries.shape[1]:
        raise ValueError("train/query dimension mismatch")
    truth = [top(base @ query, 100) for query in queries]
    seeds = np.unique(np.concatenate([ids[:10] for ids in truth]))
    lists = {}
    for seed in seeds:
        scores = base @ base[seed]
        scores[seed] = -np.inf
        lists[int(seed)] = top(scores, 256)
    grid = []
    for k in (10, 100):
        for m in (1, 3, 5, 10):
            for width in (32, 64, 128, 256):
                hits = 0
                candidate_sum = 0
                perfect = 0
                for ids in truth:
                    candidates = set(map(int, ids[:m]))
                    for seed in ids[:m]:
                        candidates.update(map(int, lists[int(seed)][:width]))
                    found = len(candidates.intersection(map(int, ids[:k])))
                    hits += found
                    candidate_sum += len(candidates)
                    perfect += found == k
                grid.append(dict(k=k, oracle_seeds=m, width=width,
                                 coverage=hits / (k * args.queries),
                                 perfect_queries=perfect,
                                 mean_candidates=candidate_sum / args.queries,
                                 id_only_index_bytes=args.rows * width * 4))
    selected = next(x for x in grid if (x["k"], x["oracle_seeds"], x["width"]) == (100, 3, 128))
    upper = next(x for x in grid if (x["k"], x["oracle_seeds"], x["width"]) == (100, 10, 256))
    decision = "accept" if selected["coverage"] >= .99 else "reject" if upper["coverage"] < .99 else "compose"
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    result = dict(experiment=args.experiment, hypothesis="H-A-2", rung=2, claim="local",
                  question="Can one unpruned base-neighbor completion round contain 99% of top100 even with oracle seeds?",
                  prediction="coverage >=0.99 with m=3 and width=128; conditional threefold search gain remains unmeasured",
                  decision=decision, rows=args.rows, queries=args.queries, dimensions=base.shape[1],
                  scope="first source rows and exposed public queries; development-only subset; oracle seeds are forbidden at serving",
                  metric="strict ID coverage, float64 normalized dot, deterministic row-ID ties",
                  source_path=str(args.data.resolve()), commit=commit,
                  selected_normalized_base_sha256=hashlib.sha256(base.tobytes()).hexdigest(),
                  selected_normalized_queries_sha256=hashlib.sha256(queries.tobytes()).hexdigest(),
                  script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  exact_seed_rows=len(seeds), exact_dot_products=args.rows*(args.queries+len(seeds)),
                  acceptance="m3/K128 coverage>=.99 necessary-condition pass only; needs independent OOD transfer and implementable seeds",
                  rejection="m10/K256 coverage<.99 rejects base-neighbor-only completion for this workload/size",
                  grid=grid)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    with args.ledger.open("a", newline="") as stream:
        csv.writer(stream, delimiter="\t").writerow([
            args.experiment,"experiment","H-A-2","unassigned-static",2,
            result["question"],result["prediction"],
            f"m3K128={selected['coverage']:.8f}; m10K256={upper['coverage']:.8f}",
            decision,commit,f"rows={args.rows}; queries={args.queries}; m=1,3,5,10; K=32,64,128,256",
            platform.node(),str(args.output.resolve())])
    print(f"{args.experiment} {decision}: m3K128={selected['coverage']:.8f}, m10K256={upper['coverage']:.8f}; {args.output}")


if __name__ == "__main__":
    main()
