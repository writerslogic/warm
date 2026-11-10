# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Compute optimistic fixed-prefix effort bounds from frozen development traces."""

import argparse
import csv
import fcntl
import gzip
import hashlib
import json
import os
import platform
from contextlib import ExitStack
from pathlib import Path

from packet_eligibility import PINS, SOURCE_COMMIT, identifier, require


PARSER_SHA = "59501edf635f15aa98f6836c82438f6e79533a159871d620886ea5de7f67394f"
CARD_SHA = "617415d3a8f86834600f90f435ea92720d0be7ffe395dc86cd9fe5c3c2bcbed7"
RESEARCH_SHA = "d9e358b9a637c1b36ddb001a2dc32b3c47496966e302e3b75bec74c40e35983d"
E21_SHA = "084def3ff85c4b08616afe3b8c95bb94ca902a52efe2d964a4df3bd2426ac343"
OBJECTIVES = ("adjacency_reads", "fresh_estimates")
FLOORS = (("0.99", 100), ("1.0", 0))


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def checked_json(path, expected):
    raw = path.read_bytes()
    require(hashlib.sha256(raw).hexdigest() == expected, f"hash mismatch: {path}")
    return json.loads(raw)


def write_json(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def meets_frontier(cost, baseline):
    require(type(cost) is int and type(baseline) is int and cost >= 0 and baseline >= 0,
            "invalid frontier counts")
    return 3 * cost <= baseline


def optimize_prefixes(options_by_query, missed_budget, gold_per_query=10):
    require(type(gold_per_query) is int and gold_per_query > 0, "invalid gold count")
    require(type(missed_budget) is int and 0 <= missed_budget <= gold_per_query * len(options_by_query),
            "invalid missed-neighbor budget")
    require(bool(options_by_query), "empty query set")
    for options in options_by_query:
        require(0 < len(options) < 255, "invalid option count")
        for hits, cost, checkpoint in options:
            require(type(hits) is int and 0 <= hits <= gold_per_query
                    and type(cost) is int and cost >= 0
                    and type(checkpoint) is int and checkpoint >= 0, "invalid prefix option")
    maximum_hits = sum(max(option[0] for option in options) for options in options_by_query)
    missing = len(options_by_query) * gold_per_query - maximum_hits
    if missing > missed_budget:
        return dict(feasible=False, maximum_hits=maximum_hits, irreducible_misses=missing,
                    cost=None, missed=None, option_indices=None)
    costs = [None] * (missed_budget + 1)
    costs[0] = 0
    choices = []
    for options in options_by_query:
        next_costs = [None] * len(costs)
        chosen = bytearray([255]) * len(costs)
        for previous_misses, previous_cost in enumerate(costs):
            if previous_cost is None:
                continue
            for index, (hits, cost, _) in enumerate(options):
                misses = previous_misses + gold_per_query - hits
                if misses > missed_budget:
                    continue
                candidate = previous_cost + cost
                if next_costs[misses] is None or candidate < next_costs[misses]:
                    next_costs[misses] = candidate
                    chosen[misses] = index
        costs = next_costs
        choices.append(chosen)
    feasible = [misses for misses, cost in enumerate(costs) if cost is not None]
    require(bool(feasible), "DP contradicts maximum-coverage feasibility")
    best_misses = min(feasible, key=lambda misses: (costs[misses], misses))
    misses = best_misses
    selected = []
    for options, chosen in zip(reversed(options_by_query), reversed(choices)):
        index = chosen[misses]
        require(index != 255 and index < len(options), "invalid optimizer witness")
        selected.append(index)
        misses -= gold_per_query - options[index][0]
    selected.reverse()
    require(misses == 0, "witness does not reach initial state")
    require(sum(options[index][1] for options, index in zip(options_by_query, selected)) == costs[best_misses],
            "witness cost mismatch")
    require(sum(gold_per_query - options[index][0] for options, index in zip(options_by_query, selected))
            == best_misses, "witness recall mismatch")
    return dict(feasible=True, maximum_hits=maximum_hits, irreducible_misses=missing,
                cost=costs[best_misses], missed=best_misses, option_indices=selected)


def read_gold(path, expected_sha, size):
    digest = hashlib.sha256()
    queries = {}
    with path.open("rb") as stream:
        while line := stream.readline(1024 * 1024 + 1):
            require(len(line) <= 1024 * 1024, "oversized oracle record")
            digest.update(line)
            row = json.loads(line)
            if row["type"] != "oracle":
                continue
            if row["state"] != "baseline" or row["filter"] != "all":
                continue
            query = row["query"]
            require(type(query) is int and 0 <= query < 1000 and query not in queries,
                    "invalid oracle query identity")
            require(row["dataset"] == f"glove-{size}" and row["queryId"] == f"query-{query}"
                    and row["eligibleCount"] == size, "oracle dataset mismatch")
            ids = [identifier(hit["ordinal"], size) for hit in row["hits"]]
            require(len(ids) == len(set(ids)) == 10, "invalid independent gold IDs")
            queries[query] = ids
    require(digest.hexdigest() == expected_sha, "independent oracle hash mismatch")
    require(set(queries) == set(range(1000)), "incomplete independent oracle")
    return queries


def query_curve(row, adjacency, gold_ids, size):
    require(row["oracle"] == gold_ids, "trace oracle differs from independent gold")
    gold = set(gold_ids)
    seen = set()
    expanded = set()
    discoveries = set()
    first = {vertex: None for vertex in gold_ids}
    checkpoints = []
    active = None
    pending = []
    position = reads = estimates = seeds = 0

    def finish():
        if active is not None:
            require(position == len(pending), "unscored adjacency entries")
            checkpoints.append([len(expanded), reads, estimates, len(discoveries)])

    for event in row["events"]:
        vertex = identifier(event["id"], size)
        if event["kind"] == "expand":
            finish()
            require(vertex in seen and vertex not in expanded, "invalid expansion")
            expanded.add(vertex)
            active = vertex
            pending = [neighbor for neighbor in adjacency[vertex] if neighbor not in seen]
            position = 0
            reads += len(adjacency[vertex])
            continue
        require(event["kind"] == "offer" and vertex not in seen and event["stored_id"] == vertex,
                "invalid first-visit offer or stored-ID mapping")
        if event["parent"] is None:
            require(not seen and active is None and vertex == row["entry"], "unexpected recovery seed")
            seeds += 1
        else:
            require(active is not None and event["parent"] == active
                    and position < len(pending) and pending[position] == vertex,
                    "incorrect parent or adjacency order")
            position += 1
        seen.add(vertex)
        estimates += 1
        if vertex in gold:
            discoveries.add(vertex)
            first[vertex] = len(checkpoints)
        if event["parent"] is None:
            checkpoints.append([0, 0, estimates, len(discoveries)])
    finish()
    require(seeds == 1 and len(checkpoints) == len(expanded) + 1, "invalid prefix boundaries")
    require(seen.issubset(set(row["evaluated"])), "base offers absent from evaluated union")
    selected = row["stages"]["selected"]
    require(len(selected) == len(set(selected)) == 10, "invalid serving-selected IDs")
    for vertex in selected:
        identifier(vertex, size)
    require(all(all(left <= right for left, right in zip(a, b))
                for a, b in zip(checkpoints, checkpoints[1:])), "nonmonotone discovery curve")
    options = []
    previous_hits = -1
    for checkpoint, (expansions, count_reads, count_estimates, hits) in enumerate(checkpoints):
        if hits != previous_hits:
            options.append(dict(checkpoint=checkpoint, hits=hits, adjacency_reads=count_reads,
                                fresh_estimates=count_estimates, expansions=expansions))
            previous_hits = hits
    return dict(query=row["query"], gold_ids=gold_ids, checkpoints=checkpoints,
                first_gold_checkpoint=first, options=options,
                serving_selected_hits=len(gold.intersection(selected)))


def collect_archive(repo, pin, gold_pin, e21_cells, details):
    size, archive, mode, receipt_sha, raw_sha, graph_hash = pin
    receipt_path = repo / f"docs/research/milestones-20261102/navigation-{size // 1000}k/finished.json"
    receipt = checked_json(receipt_path, receipt_sha)
    require(receipt["sourceCommit"] == SOURCE_COMMIT and receipt["complete"] and receipt["failure"] is None,
            "unverified source receipt")
    require(receipt["efs"] == [128, 256] and receipt["queriesPerEf"] == 1000
            and receipt["result"]["originalRankingsMatched"] == 2000
            and receipt["result"]["originalExactRankingsMatched"] == 1000
            and receipt["result"]["rawSha256"] == raw_sha, "source verification incomplete")
    require(receipt["referenceFiles"]["oracle"] == gold_pin, "oracle reference changed")
    gold = read_gold(Path(gold_pin["path"]), gold_pin["sha256"], size)
    raw = Path("/Volumes/A/.hms-bench-cache") / archive / "navigation.ndjson.gz"
    cells = {ef: dict(rows=size, ef=ef, queries=[], totals=dict(expansions=0, adjacency_reads=0,
                  fresh_estimates=0), full_gold_hits=0, serving_selected_hits=0) for ef in (128, 256)}
    curve_digests = {ef: hashlib.sha256() for ef in cells}
    raw_digest = hashlib.sha256()
    line_count = 0
    with ExitStack() as stack:
        outputs = {}
        for ef in cells:
            path = details / f"curves-{size}-ef{ef}.ndjson.gz"
            stream = stack.enter_context(path.open("xb"))
            outputs[ef] = stack.enter_context(gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0))
            cells[ef]["curve_path"] = str(path.resolve())
        stream = stack.enter_context(gzip.open(raw, "rb"))
        while line := stream.readline(128 * 1024 * 1024 + 1):
            require(len(line) <= 128 * 1024 * 1024, "oversized navigation record")
            raw_digest.update(line)
            row = json.loads(line)
            line_count += 1
            if line_count == 1:
                require(row["type"] == "start" and row["mode"] == mode and row["graphHash"] == graph_hash
                        and row["indexBreakdown"]["original_ids_bytes"] == 0, "header identity mismatch")
                if size == 100000:
                    require("rows" not in row and "corpusRows" not in receipt
                            and receipt["expectedGraphHash"] == graph_hash, "unexpected legacy100k schema")
                else:
                    require(row["rows"] == receipt["corpusRows"] == size
                            and receipt["result"]["graphHash"] == graph_hash, "unexpected10k schema")
                continue
            if line_count == 2:
                require(row["type"] == "topology" and len(row["adjacency"]) == size, "invalid topology")
                adjacency = row["adjacency"]
                for neighbors in adjacency:
                    require(type(neighbors) is list and len(neighbors) <= 256
                            and len(neighbors) == len(set(neighbors)), "invalid adjacency list")
                    for vertex in neighbors:
                        identifier(vertex, size)
                continue
            require(line_count <= 2002 and row["type"] == "navigation"
                    and row["ef"] == (128 if line_count <= 1002 else 256), "unexpected trace record")
            ef = row["ef"]
            cell = cells[ef]
            require(row["query"] == len(cell["queries"]), "missing or duplicate navigation query")
            curve = query_curve(row, adjacency, gold[row["query"]], size)
            encoded = (json.dumps(curve, separators=(",", ":"), allow_nan=False) + "\n").encode()
            outputs[ef].write(encoded)
            curve_digests[ef].update(encoded)
            last = curve["checkpoints"][-1]
            cell["totals"]["expansions"] += last[0]
            cell["totals"]["adjacency_reads"] += last[1]
            cell["totals"]["fresh_estimates"] += last[2]
            cell["full_gold_hits"] += last[3]
            cell["serving_selected_hits"] += curve["serving_selected_hits"]
            cell["queries"].append(dict(query=row["query"], options=curve["options"], full=last))
    require(line_count == 2002 and raw_digest.hexdigest() == raw_sha, "raw navigation hash/count mismatch")
    for ef, cell in cells.items():
        prior = e21_cells[(size, ef)]
        require(len(cell["queries"]) == prior["queries"] == 1000
                and cell["totals"]["expansions"] == prior["expansions"]
                and cell["totals"]["adjacency_reads"] == prior["adjacency_edges"]
                and cell["totals"]["fresh_estimates"] == prior["offers"]
                and prior["seed_offers"] == 1000, "primitive totals differ from frozen E21")
        cell["curve_raw_sha256"] = curve_digests[ef].hexdigest()
        cell["curve_gzip_sha256"] = sha256(Path(cell["curve_path"]))
        cell["raw_source"] = str(raw)
        cell["raw_source_sha256"] = raw_sha
        cell["oracle_source"] = gold_pin
        cell["receipt_source"] = dict(path=str(receipt_path), sha256=receipt_sha)
    return list(cells.values())


def allocate(cells, details):
    summaries = []
    for cell in cells:
        summary = {name: value for name, value in cell.items() if name != "queries"}
        summary["query_count"] = len(cell["queries"])
        summary["floors"] = []
        for floor, budget in FLOORS:
            required_hits = 10000 - budget
            competitors = [other for other in cells if other["rows"] == cell["rows"]]
            eligible = [other for other in competitors if other["full_gold_hits"] >= required_hits]
            result = dict(recall_floor=floor, missed_budget=budget, required_hits=required_hits,
                          maximum_hits=cell["full_gold_hits"], irreducible_misses=10000 - cell["full_gold_hits"],
                          baseline_candidates=[dict(ef=other["ef"], full_gold_hits=other["full_gold_hits"],
                              totals=other["totals"], eligible=other in eligible) for other in competitors],
                          objectives={})
            for objective in OBJECTIVES:
                options = [[(point["hits"], point[objective], point["checkpoint"])
                            for point in query["options"]] for query in cell["queries"]]
                optimum = optimize_prefixes(options, budget)
                require(optimum["maximum_hits"] == cell["full_gold_hits"], "optimizer coverage mismatch")
                baseline = min(eligible, key=lambda other: (other["totals"][objective], other["ef"])) if eligible else None
                baseline_value = baseline["totals"][objective] if baseline else None
                witness_path = details / f"witness-{cell['rows']}-ef{cell['ef']}-r{floor}-{objective}.json"
                witness = dict(rows=cell["rows"], ef=cell["ef"], recall_floor=floor, objective=objective,
                               curve_path=cell["curve_path"], curve_raw_sha256=cell["curve_raw_sha256"],
                               optimum={name: value for name, value in optimum.items() if name != "option_indices"},
                               selections=None)
                if optimum["feasible"]:
                    require(baseline is not None, "feasible prefix without eligible full baseline")
                    witness["selections"] = [dict(query=query["query"], **query["options"][index])
                                              for query, index in zip(cell["queries"], optimum["option_indices"])]
                    require(sum(point[objective] for point in witness["selections"]) == optimum["cost"]
                            and sum(point["hits"] for point in witness["selections"]) >= required_hits,
                            "invalid retained witness")
                    for query, point in zip(cell["queries"], witness["selections"]):
                        require(point["expansions"] <= query["full"][0]
                                and point["adjacency_reads"] <= query["full"][1]
                                and point["fresh_estimates"] <= query["full"][2], "witness exceeds original work")
                write_json(witness_path, witness)
                passed = optimum["feasible"] and meets_frontier(optimum["cost"], baseline_value)
                result["objectives"][objective] = dict(
                    feasible=optimum["feasible"], minimum_work=optimum["cost"],
                    baseline_work=baseline_value, baseline_ef=baseline["ef"] if baseline else None,
                    work_reduction_factor=(baseline_value / optimum["cost"] if optimum["feasible"] and optimum["cost"] else None),
                    zero_cost_oracle=optimum["feasible"] and optimum["cost"] == 0,
                    passes_threefold_necessary_condition=passed,
                    witness=str(witness_path.resolve()), witness_sha256=sha256(witness_path))
            if cell["full_gold_hits"] < required_hits:
                result["status"] = "infeasible_within_frozen_trace"
            else:
                result["status"] = ("necessary_condition_pass" if all(
                    row["passes_threefold_necessary_condition"] for row in result["objectives"].values())
                    else "reject_threefold_work_condition")
            summary["floors"].append(result)
        summaries.append(summary)
    return summaries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--details", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--experiment", choices=("E26",), required=True)
    args = parser.parse_args()
    require(not args.output.exists() and not args.details.exists() and args.ledger.is_file(),
            "existing output/details or missing ledger")
    own = Path(__file__).resolve().parent
    require(sha256(own / "packet_eligibility.py") == PARSER_SHA, "corrected parser source changed")
    require(sha256(own / "next-card5.md") == CARD_SHA, "frozen card changed")
    research = checked_json(own / "research5.json", RESEARCH_SHA)
    prior_path = args.repo / "benchmarks/results/mission/root/E21/result.json"
    prior = checked_json(prior_path, E21_SHA)
    require(prior["experiment"] == "E21" and prior["script_sha256"] == PARSER_SHA, "E21 provenance mismatch")
    e21_cells = {(archive["rows"], cell["ef"]): cell for archive in prior["archives"] for cell in archive["cells"]}
    require(set(e21_cells) == {(size, ef) for size in (10000, 100000) for ef in (128, 256)}, "incomplete E21 cells")
    args.details.mkdir(parents=True, exist_ok=False)
    manifest = dict(experiment=args.experiment, hypothesis="H-A-5", card_sha256=CARD_SHA,
                    research_sha256=RESEARCH_SHA, parser_sha256=PARSER_SHA, e21_sha256=E21_SHA,
                    script_sha256=sha256(Path(__file__)), source_commit=SOURCE_COMMIT,
                    checkpoint_columns=["expansions", "adjacency_reads", "fresh_estimates", "gold_hits"],
                    preregistration=research["preregistration"], inputs=research["exact_counter_design"]["inputs"])
    write_json(args.details / "manifest.json", manifest)
    cells = []
    for pin, source in zip(PINS, manifest["inputs"]):
        require(source["size"] == f"{pin[0] // 1000}k", "input pin order mismatch")
        cells.extend(collect_archive(args.repo, pin, source["oracle_reference"], e21_cells, args.details))
    summaries = allocate(cells, args.details)
    candidates = [cell for cell in summaries if cell["ef"] == 256]
    require(len(candidates) == 2, "missing candidate cells")
    passed = all(cell["floors"][0]["status"] == "necessary_condition_pass" for cell in candidates)
    result = dict(experiment=args.experiment, hypothesis="H-A-5", claim="local", rung=2,
                  question=research["question"], prediction=research["prediction"],
                  decision="compose" if passed else "reject", necessary_condition_passed=passed,
                  candidate_ef=256, performance_acceptance=False, matched_gap_condition="UNPROVED",
                  cells=summaries, manifest=str((args.details / "manifest.json").resolve()),
                  manifest_sha256=sha256(args.details / "manifest.json"), script_sha256=manifest["script_sha256"],
                  limits="Exposed development oracle only; independent objective minima may have different witnesses. Infeasibility applies only to these frozen prefixes, not deeper adaptive search. Perfect1 remains separate. No classifier, native search, runtime gain, fresh holdout or matched competitor conclusion.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    write_json(args.output, result)
    summary = "; ".join(f"{cell['rows']}/ef{cell['ef']}:" + ",".join(
        f"r{floor['recall_floor']}={floor['status']}" for floor in cell["floors"]) for cell in summaries)
    with args.ledger.open("a", newline="") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        csv.writer(stream, delimiter="\t").writerow([
            args.experiment, "experiment", "H-A-5", "static-development", 2,
            result["question"], result["prediction"], summary, result["decision"], SOURCE_COMMIT,
            "ef256-prefixes;ef128/256-baselines;r.99/1;independent-read/estimate-DP", platform.node(), str(args.output.resolve()),
        ])
        stream.flush()
        os.fsync(stream.fileno())
    print(f"{args.experiment} {result['decision']}: {summary}; {args.output}")


if __name__ == "__main__":
    main()
