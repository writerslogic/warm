# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Summarize complete official VectorDBBench runs and cgroup receipts."""
import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    rows = []
    for directory in sorted(args.out.iterdir()):
        if not directory.is_dir():
            continue
        paths = list(directory.glob("results/Warm/*.json"))
        if len(paths) != 1:
            raise ValueError(f"{directory.name}: expected exactly one result, found {len(paths)}")
        results = json.loads(paths[0].read_text())["results"]
        if len(results) != 1:
            raise ValueError(f"{directory.name}: expected one case")
        metrics = results[0]["metrics"]
        if int((directory / "exit-code").read_text()) != 0:
            raise ValueError(f"{directory.name}: run failed")
        if metrics["conc_num_list"] != [1, 5, 10] or len(metrics["conc_qps_list"]) != 3:
            raise ValueError(f"{directory.name}: incomplete concurrency results")
        if metrics["additional_parameters"]["ground_truth"]["query_count"] != 1000:
            raise ValueError(f"{directory.name}: incomplete query set")
        row = {
            "label": directory.name,
            "raw": str(paths[0].relative_to(args.out)),
            "recall_at_100": metrics["recall"],
            "ndcg": metrics["ndcg"],
            "max_qps": metrics["qps"],
            "concurrency": metrics["conc_num_list"],
            "qps": metrics["conc_qps_list"],
            "peak_cgroup_bytes": int((directory / "memory.peak").read_text()),
            "insert_seconds": metrics["insert_duration"],
            "optimize_seconds": metrics["optimize_duration"],
            "started_utc": (directory / "started-utc").read_text().strip(),
            "finished_utc": (directory / "finished-utc").read_text().strip(),
        }
        snapshot = directory / "snapshot.json"
        if snapshot.exists():
            row["snapshot"] = json.loads(snapshot.read_text())
        rows.append(row)
    if not rows:
        raise ValueError("no benchmark runs found")
    print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
