# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Paired English query-latency check of two `competitive_text` builds on one warm-bench text
folder: each binary builds its own lexical store, then serve runs alternate A B A B ... so load
drift hits both alike. Reports per-round total query time and the paired ratio (B / A), not
absolute latency: the host may be loaded.

  uv run --no-project python -I benchmarks/competitive/warmbench/analyzer_latency.py \\
      --folder CACHE/text-scifact-train --a before=BIN --b new=BIN --rounds 5 --out FILE.json
"""

import argparse
import hashlib
import json
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from warmbench.common import log  # noqa: E402


def run(cmd):
    done = subprocess.run(cmd, capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"{cmd[0]} failed ({done.returncode}): {done.stderr[-2000:]}")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--folder", type=Path, required=True)
    p.add_argument("--a", required=True, metavar="LABEL=BINARY")
    p.add_argument("--b", required=True, metavar="LABEL=BINARY")
    p.add_argument("--rounds", type=int, default=5)
    p.add_argument("--passes", type=int, default=3)
    p.add_argument("--out", type=Path, required=True)
    a = p.parse_args()
    systems = [s.split("=", 1) for s in (a.a, a.b)]
    work = Path(tempfile.mkdtemp(prefix="analyzer-latency-"))
    try:
        stores = {}
        for label, binary in systems:
            stores[label] = work / f"store-{label}"
            run([binary, "build", "--input", str(a.folder / "input.json"), "--store", str(stores[label]),
                 "--out", str(work / f"build-{label}.json"), "--encode", "lexical"])
        rounds = []
        for _ in range(a.rounds):
            row = {}
            for label, binary in systems:
                out = work / f"serve-{label}.json"
                run([binary, "serve", "--input", str(a.folder / "input.json"), "--store", str(stores[label]),
                     "--out", str(out), "--t0-ns", str(time.time_ns()), "--passes", str(a.passes)])
                passes = json.loads(out.read_text())["passes_ns"]
                # Per query, the fastest of the passes; the round's figure is their sum.
                row[label] = sum(min(times) for times in zip(*passes)) / 1e9
            rounds.append(row)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    (la, ba), (lb, bb) = systems
    ratios = [r[lb] / r[la] for r in rounds]
    result = {
        "folder": str(a.folder.name),
        "method": "per round: each binary serves every query `passes` times; per-query minimum, summed; "
                  "rounds alternate A then B",
        "binaries": {la: hashlib.sha256(Path(ba).read_bytes()).hexdigest(),
                     lb: hashlib.sha256(Path(bb).read_bytes()).hexdigest()},
        "rounds": rounds,
        f"ratio_{lb}_over_{la}": {"median": statistics.median(ratios), "min": min(ratios), "max": max(ratios)},
    }
    a.out.write_text(json.dumps(result, indent=1) + "\n")
    log(json.dumps(result[f"ratio_{lb}_over_{la}"]))


if __name__ == "__main__":
    main()
