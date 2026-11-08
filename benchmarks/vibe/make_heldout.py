# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Derive a held-out evaluation file from a prepared OOD file whose learn split is larger than
the build sample.

The adapter builds from `default_rng(24301).permutation(len(learn))[:32768]`. This takes the rows
that draw leaves out as the evaluation queries (exact truth over the same corpus) and stores the
drawn rows as the learn split, so selecting parameters on the output never touches the VIBE test
queries and no evaluation query is a build input.
"""

import argparse
import sys
from pathlib import Path

import h5py
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare import LEARN_SEED, exact_truth  # noqa: E402

BUILD_ROWS = 32768


def main(args):
    with h5py.File(args.source, "r") as src:
        train, learn = src["train"][:], src["learn"][:]
        metric, dim = str(src.attrs["distance"]), int(src.attrs["dimension"])
    if len(learn) < BUILD_ROWS + args.queries:
        raise ValueError(f"learn split has {len(learn)} rows; need {BUILD_ROWS + args.queries}")
    order = np.random.default_rng(LEARN_SEED).permutation(len(learn))
    build, held = learn[order[:BUILD_ROWS]], learn[order[BUILD_ROWS:BUILD_ROWS + args.queries]]
    ids, distances = exact_truth(train, held, metric)
    with h5py.File(args.out, "w") as dst:
        dst.attrs.update(dimension=dim, distance=metric, point_type="float")
        for name, rows in (("train", train), ("test", held), ("neighbors", ids),
                           ("distances", distances), ("learn", build)):
            dst.create_dataset(name, data=rows)
    print(f"held-out {len(held)} queries from {len(learn) - BUILD_ROWS} unused learn rows -> {args.out}")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("source", type=Path)
    p.add_argument("out", type=Path)
    p.add_argument("--queries", type=int, default=2000)
    main(p.parse_args())
