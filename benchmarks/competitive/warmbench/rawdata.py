# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Raw per-query measurements beside each result file, and the selection/report split.

Every timed serve phase hands back per-query nanoseconds. They are written to `<result>.latency.npz`
(one array per timed pass, unsigned 32-bit nanoseconds) and the result JSON records the file name,
its sha256 and the array keys, so a reader can recompute every interval from the two files.

ANN operating points are chosen on one half of the queries and scored on the other: query `i` belongs
to the selection half when `i` is even and to the report half when it is odd, so both halves cover the
same distribution. Per-half recall and QPS are computed here from the raw arrays the workers return.
"""

import hashlib
from pathlib import Path

import numpy as np

SELECT, REPORT = "select", "report"


def halves(n):
    """(selection indices, report indices) of `n` queries."""
    index = np.arange(n)
    return index[index % 2 == 0], index[index % 2 == 1]


def sha256_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as handle:
        while block := handle.read(1 << 22):
            digest.update(block)
    return digest.hexdigest()


def require_digests(meta, keys):
    """Refuse to publish a run whose inputs are not pinned by hash."""
    missing = [k for k in keys if not meta.get(k)]
    if missing:
        raise RuntimeError(f"dataset digests missing, refusing to run: {', '.join(missing)}")


class RawStore:
    """Named arrays collected during a run and written once, with their digest, next to the result."""

    def __init__(self):
        self.arrays = {}

    def add_latency(self, key, nanoseconds):
        values = np.asarray(nanoseconds, dtype=np.int64)
        self.arrays[key] = np.minimum(values, np.iinfo(np.uint32).max).astype(np.uint32)

    def add_counts(self, key, counts):
        self.arrays[key] = np.asarray(counts, dtype=np.uint16)

    def write(self, result_path):
        """Writes `<result stem>.latency.npz`; returns the metadata record for the result JSON."""
        result_path = Path(result_path)
        path = result_path.with_name(result_path.stem + ".latency.npz")
        np.savez_compressed(path, **{k.replace("/", "|"): v for k, v in self.arrays.items()})
        return {"file": path.name, "sha256": sha256_file(path), "bytes": path.stat().st_size, "arrays": len(self.arrays),
                "unit": "ns (uint32, clipped at 4.29 s)", "key_separator": "|", "format": "numpy npz; one array per timed pass or per-config hit counts"}


def _summary(ns):
    ordered = np.sort(np.asarray(ns, dtype=np.int64)) / 1e3
    at = lambda q: float(ordered[int((len(ordered) - 1) * q)])  # noqa: E731
    return {"qps": len(ordered) / (float(np.sum(ordered)) * 1e-6), "p50_us": at(0.5), "p99_us": at(0.99)}


def split_serve_round(serve, store, key, truth_width=10):
    """Moves a serve round's raw arrays into `store` and adds per-half summaries to each config.

    Workers return, per config, `hits_per_query` (true neighbours found among the `truth_width` returned,
    per query) and, per repeat, `lat_ns`. Each config gains `select` and `report` records with the recall
    and per-repeat summaries of that half; the whole-set fields stay as they were.
    """
    for c, config in enumerate(serve.get("configs", [])):
        hits = config.pop("hits_per_query", None)
        repeats = config.get("repeats", [])
        raws = [r.pop("lat_ns", None) for r in repeats]
        if hits is None or any(r is None for r in raws) or not raws:
            continue
        store.add_counts(f"{key}/c{c}/hits", hits)
        for k, raw in enumerate(raws):
            store.add_latency(f"{key}/c{c}/r{k}", raw)
        hits = np.asarray(hits)
        for name, index in zip((SELECT, REPORT), halves(len(hits))):
            config[name] = {"queries": int(len(index)), "recall_at_10": float(hits[index].sum()) / (truth_width * len(index)),
                            "repeats": [_summary(np.asarray(raw)[index]) for raw in raws]}
    return serve
