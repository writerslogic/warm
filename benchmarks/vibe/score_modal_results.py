# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Score VIBE runner result files (one HDF5 per build) with VIBE's own recall rule.

Recall is export_results.get_recall_values: stored run distances <= the dataset's k-th exact
distance + 1e-3, averaged over queries. Ground-truth distances come from the pinned VIBE dataset
by a range read of the small `distances` array, so the multi-GB corpus is never downloaded.
QPS is the file's best_qps (best of the runner's repeats), not a median.
"""
import argparse
import json
from pathlib import Path

import fsspec
import h5py
import numpy as np

REVISION = "07b387891a221b7b073b83d2f752b76462e5fa03"
TARGETS = (0.90, 0.94, 0.95, 0.99, 1.0)


def dataset_distances(dataset):
    url = f"https://huggingface.co/datasets/vector-index-bench/vibe/resolve/{REVISION}/{dataset}.hdf5"
    with fsspec.open(url, "rb", block_size=8 << 20) as remote:
        return h5py.File(remote, "r")["distances"][:]


def score(path, truth, k=100, epsilon=1e-3):
    rows = []
    with h5py.File(path, "r") as handle:
        for key in handle:
            group = handle[key]
            found = group["distances"][:, :k]
            recall = float((found <= truth[:, k - 1, None] + epsilon).sum(axis=1).mean() / k)
            rows.append({"config": key, "recall": recall, "best_qps": float(group.attrs["best_qps"]),
                         "name": str(group.attrs["name"])})
    return rows


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", help="for example laion-clip-512-normalized")
    parser.add_argument("results", type=Path, nargs="+", help="per-build result .hdf5 files")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    truth = dataset_distances(args.dataset)
    scored = {path.name: score(path, truth) for path in args.results}
    best = {}
    for target in TARGETS:
        eligible = [(name, row) for name, rows in scored.items() for row in rows if row["recall"] >= target]
        best[str(target)] = max(eligible, key=lambda item: item[1]["best_qps"]) if eligible else None
    args.out.write_text(json.dumps({"dataset": args.dataset, "builds": scored, "best": best}, indent=1) + "\n")
    for name, rows in scored.items():
        print(name, "max recall", max(row["recall"] for row in rows))
    for target, value in best.items():
        print(target, value and (value[0], value[1]["config"], round(value[1]["recall"], 5), round(value[1]["best_qps"])))
