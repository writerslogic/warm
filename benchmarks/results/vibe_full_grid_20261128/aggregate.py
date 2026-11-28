# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Build screening.csv and targets.csv from evidence/rawA.csv (recall and QPS per config), evidence/rssA.csv (peak RSS per
worker, one worker per index build) and evidence/mtimes.txt (result file times, to attach a worker to its build)."""

import csv
import os
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
TARGETS = (0.90, 0.95, 0.99)
SYSTEM = {"roar": "roargraph", "mlann": "mlann-rf", "new": "warm-541"}


def read(name):
    with open(os.path.join(HERE, "evidence", name)) as handle:
        return list(csv.reader(handle))


def peak_rss():
    """{(tag, dataset, result file): peak RSS in GiB}: each result file is matched to the unused worker of its run that
    exited closest to the file's last write (at most 15 s apart)."""
    workers = defaultdict(list)
    for tag, ds, _, _, kb, _, end in read("rssA.csv"):
        workers[tag].append([float(end), int(kb)])
    out = {}
    files = []
    for line in read("mtimes.txt"):
        t, path = line[0].split(" ")[0], line[0].split(" ")[1]
        parts = path.split("/")
        files.append((float(t), parts[1], parts[3], parts[-1]))
    for t, tag, ds, name in sorted(files):
        free = [w for w in workers[tag] if len(w) == 2]
        if not free:
            continue
        best = min(free, key=lambda w: abs(w[0] - t))
        if abs(best[0] - t) <= 15:
            best.append("used")
            out[(tag, ds, name)] = best[1] / 1048576
    return out


def main():
    rss = peak_rss()
    rows = []
    with open(os.path.join(HERE, "evidence", "rawA.csv")) as handle:
        for r in csv.DictReader(handle):
            tag = f"A-{r['unit']}-{r['dataset']}"
            rows.append({"system": SYSTEM[r["unit"]], "dataset": r["dataset"], "build_file": r["file"], "query_group": r["group"], "config": r["name"],
                         "recall_ids": float(r["recall_f64"]), "qps": float(r["qps_last"]), "build_s": float(r["build_time"]),
                         "peak_rss_gib": rss.get((tag, r["dataset"], r["file"]))})
    fields = list(rows[0])
    with open(os.path.join(HERE, "screening.csv"), "w", newline="") as handle:
        w = csv.DictWriter(handle, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)
    best = defaultdict(list)
    for r in rows:
        best[(r["dataset"], r["system"])].append(r)
    with open(os.path.join(HERE, "targets.csv"), "w", newline="") as handle:
        w = csv.writer(handle)
        w.writerow(["dataset", "system", "target", "recall_ids", "qps", "build_file", "query_group", "build_s", "peak_rss_gib", "configs_run", "builds_run"])
        for (ds, system), rs in sorted(best.items()):
            builds = len({r["build_file"] for r in rs})
            for t in TARGETS:
                ok = [r for r in rs if r["recall_ids"] >= t]
                if not ok:
                    w.writerow([ds, system, t, "", "", "", "", "", "", len(rs), builds])
                    continue
                r = max(ok, key=lambda x: x["qps"])
                w.writerow([ds, system, t, f"{r['recall_ids']:.4f}", f"{r['qps']:.0f}", r["build_file"], r["query_group"], f"{r['build_s']:.0f}",
                            "" if r["peak_rss_gib"] is None else f"{r['peak_rss_gib']:.2f}", len(rs), builds])


if __name__ == "__main__":
    main()
