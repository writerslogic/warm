# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Count removable primitive events in existing, verified development archives."""

import argparse
import csv
import gzip
import hashlib
import json
import platform
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def navigation(raw, receipt_path, expected_rows):
    receipt = json.loads(receipt_path.read_text())
    require(receipt["complete"] and receipt["failure"] is None, "incomplete source receipt")
    require(receipt["efs"] == [128, 256] and receipt["queriesPerEf"] == 1000,
            "unexpected source query configuration")
    require(receipt["result"]["originalRankingsMatched"] == 2000
            and receipt["result"]["originalExactRankingsMatched"] == 1000,
            "source production/oracle verification incomplete")
    digest = hashlib.sha256()
    cells = {}
    adjacency = None
    line_count = 0
    with gzip.open(raw, "rb") as stream:
        while line := stream.readline(128 * 1024 * 1024 + 1):
            require(len(line) <= 128 * 1024 * 1024, "oversized trace record")
            digest.update(line)
            row = json.loads(line)
            line_count += 1
            if line_count == 1:
                require(row["type"] == "start", "missing trace header")
                size = expected_rows
                if size == 100000:
                    require(row["mode"] == "navigation" and "rows" not in row
                            and "corpusRows" not in receipt, "unexpected legacy100k schema")
                    require(row["graphHash"] == receipt["expectedGraphHash"], "source graph identity mismatch")
                elif size == 10000:
                    require(row["mode"] == "navigation10k" and row["rows"] == size
                            and receipt["corpusRows"] == size, "source corpus size mismatch")
                    require(row["graphHash"] == receipt["result"]["graphHash"], "source graph identity mismatch")
                else:
                    raise ValueError("unsupported pinned archive size")
                continue
            if line_count == 2:
                require(row["type"] == "topology" and len(row["adjacency"]) == size, "missing topology")
                require(all(len(ids) == len(set(ids)) for ids in row["adjacency"]), "duplicate adjacency IDs")
                adjacency = [frozenset(ids) for ids in row["adjacency"]]
                require(all(len(ids) <= 256 and all(isinstance(i, int) and 0 <= i < size for i in ids)
                            for ids in adjacency), "invalid adjacency")
                continue
            require(row["type"] == "navigation" and row["ef"] in (128, 256), "unexpected trace kind")
            ef = row["ef"]
            cell = cells.setdefault(ef, dict(ef=ef, queries=0, expansions=0, offers=0,
                                            zero_discovery=0, certified_zero=0, adjacency_reads=0,
                                            zero_adjacency_reads=0, certified_adjacency_reads=0,
                                            rerank_evaluations=0, returned=0,
                                            expansions_without_direct_final_discovery=0))
            require(row["query"] == cell["queries"], "duplicate or missing query")
            parents = {}
            expanded = set()
            active = None
            fresh = 0
            direct_final = 0
            selected = set(row["stages"]["selected"])
            require(len(selected) == 10, "invalid final result count")

            def finish():
                if active is None:
                    return
                degree = len(adjacency[active])
                cell["expansions"] += 1
                cell["adjacency_reads"] += degree
                parent = parents[active]
                certified = parent is not None and all(i == parent or i in adjacency[parent]
                                                       for i in adjacency[active])
                require(not certified or fresh == 0, "subset certificate contradicts observed discovery")
                if fresh == 0:
                    cell["zero_discovery"] += 1
                    cell["zero_adjacency_reads"] += degree
                if certified:
                    cell["certified_zero"] += 1
                    cell["certified_adjacency_reads"] += degree
                if direct_final == 0:
                    cell["expansions_without_direct_final_discovery"] += 1

            for event in row["events"]:
                identifier = event["id"]
                require(isinstance(identifier, int) and 0 <= identifier < size, "invalid event ID")
                if event["kind"] == "expand":
                    finish()
                    require(identifier in parents and identifier not in expanded, "invalid expansion")
                    expanded.add(identifier)
                    active, fresh, direct_final = identifier, 0, 0
                else:
                    require(event["kind"] == "offer" and identifier not in parents, "invalid offer")
                    parent = event["parent"]
                    require(parent is None or parent == active, "incorrect discovery parent")
                    parents[identifier] = parent
                    if parent is not None:
                        fresh += 1
                        direct_final += identifier in selected
            finish()
            cell["queries"] += 1
            cell["offers"] += len(parents)
            cell["rerank_evaluations"] += row["stages"]["rerankCount"]
            cell["returned"] += len(selected)
    require(line_count == 2002 and len(cells) == 2, "incomplete trace")
    require(digest.hexdigest() == receipt["result"]["rawSha256"], "raw trace hash mismatch")
    for cell in cells.values():
        require(cell["queries"] == 1000 and cell["expansions"] > 0, "incomplete cell")
        for name in ("zero_discovery", "certified_zero"):
            fraction = cell[name] / cell["expansions"]
            cell[name + "_fraction"] = fraction
            cell[name + "_equal_expansion_ceiling"] = 1 / (1 - fraction) if fraction < 1 else None
        cell["zero_adjacency_read_fraction"] = cell["zero_adjacency_reads"] / cell["adjacency_reads"]
        cell["certified_adjacency_read_fraction"] = cell["certified_adjacency_reads"] / cell["adjacency_reads"]
        cell["decision"] = "accept" if cell["certified_zero_fraction"] >= 1 / 3 else "reject"
        cell["zero_discovery_main_line_decision"] = "accept" if cell["zero_discovery_fraction"] >= 1 / 3 else "reject"
    return dict(raw=str(raw), raw_sha256=digest.hexdigest(), source_commit=receipt["sourceCommit"],
                rows=size, cells=list(cells.values()))


def profiles(path):
    raw = path.read_bytes()
    value = json.loads(raw)
    require(value["schema_version"] == 1 and len(value["layouts"]) == 2
            and [layout["compact"] for layout in value["layouts"]] == [False, True],
            "unexpected pinned phase-profile schema")
    output = []
    for layout in value["layouts"]:
        queries = layout["queries"]
        require(len(queries) == value["input"]["validation_queries"] == 256, "incomplete phase profile")
        total = sum(q["total_ns"] for q in queries)
        rerank = sum(q["rerank_sort_ns"] for q in queries)
        require(0 <= rerank < total, "invalid phase totals")
        fraction = rerank / total
        output.append(dict(compact=layout["compact"], queries=len(queries),
                           total_ns=total, rerank_sort_ns=rerank, fraction=fraction,
                           free_rerank_ceiling=1 / (1 - fraction),
                           rerank_only_main_line_decision="accept" if fraction >= 1 / 3 else "reject"))
    return dict(path=str(path), sha256=hashlib.sha256(raw).hexdigest(),
                scope=value["scope"], layouts=output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--experiment", required=True)
    args = parser.parse_args()
    require(args.experiment.startswith("E") and args.experiment[1:].isdigit(), "invalid experiment ID")
    require(not args.output.exists() and args.ledger.is_file(), "output exists or ledger missing")
    inputs = [("100k", 100000, "warm-navigation-20261102-v1"), ("10k", 10000, "warm-navigation-10k-20261102-v1")]
    results = [navigation(Path("/Volumes/A/.hms-bench-cache") / archive / "navigation.ndjson.gz",
                          args.repo / "docs/research/milestones-20261102" / ("navigation-" + size) / "finished.json",
                          count)
               for size, count, archive in inputs]
    phase = [profiles(args.repo / "benchmarks/results/query_aware_graph_v2/profiles" / (name + "-high.json"))
             for name in ("yi", "llama")]
    decision = "accept" if all(c["decision"] == "accept" for r in results for c in r["cells"]) else "reject"
    value = dict(experiment=args.experiment, hypothesis="H-A-3", claim="local", rung=2,
                 question="Does exact discovery-parent domination remove enough expansion work for a frontier mechanism?",
                 prediction="zero-discovery fraction>=.50 and exact certificates>=1/3 in every frozen cell",
                 decision=decision, navigation=results, phase_profiles=phase,
                 script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                 limits="Archived development only; hypothetical equal-cost expansion ceilings are not throughput bounds without per-event costs; GloVe and Yi/Llama fractions must not be combined; no-direct-final-discovery events may be necessary bridge expansions.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
    summary = "; ".join(f"{r['rows']}/ef{c['ef']}:zero={c['zero_discovery_fraction']:.6f},cert={c['certified_zero_fraction']:.6f}"
                        for r in results for c in r["cells"])
    with args.ledger.open("a", newline="") as stream:
        csv.writer(stream, delimiter="\t").writerow([args.experiment,"experiment","H-A-3","static-development",2,
            value["question"],value["prediction"],summary,decision,"archived source pins in result",
            "full10k/100k;ef128/256;Yi/Llama archived phases",platform.node(),str(args.output.resolve())])
    print(f"{args.experiment} {decision}: {summary}; {args.output}")


if __name__ == "__main__":
    main()
