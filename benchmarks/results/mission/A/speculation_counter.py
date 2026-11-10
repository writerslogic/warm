# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Replay fixed-eight speculative preparation on pinned development traces."""

import argparse
import bisect
import csv
import fcntl
import gzip
import hashlib
import json
import os
import platform
import signal
import sys
from collections import Counter
from contextlib import ExitStack
from fractions import Fraction
from pathlib import Path

from effort_allocation import checked_json, read_gold, sha256, write_json
from packet_eligibility import (
    COUNTERS, PINS, SOURCE_COMMIT, VERTEX_SHA256, identifier, inspect_query,
    key, packet_shape, require,
)


WINDOW = 8
PARSER_SHA = "59501edf635f15aa98f6836c82438f6e79533a159871d620886ea5de7f67394f"
HELPER_SHA = "db8f68aa9647a6caf2a63433edba4210909f4b7b972b2f7527a9385fd1cb149d"
GOLD_RESEARCH_SHA = "d9e358b9a637c1b36ddb001a2dc32b3c47496966e302e3b75bec74c40e35983d"
CARD_SHA = "3c9f97d6588eca477e7a8e75bcd752e8e6ab3904d4a09ce3a92b8f76f571c57e"
RESEARCH_SHA = "7c81b00a37d0b5c5a3797aebadd4c18d3d220123d2de730d8c6dc075e7067046"
E21_SHA = "084def3ff85c4b08616afe3b8c95bb94ca902a52efe2d964a4df3bd2426ac343"
QUESTION = "Does fixed-eight preparation satisfy every frozen dependence-round and amplification gate?"
PREDICTION = "At least one cell exceeds a dependence-round or amplification cap; retain every cell."
ADDITIVE = (
    "expansions", "groups", "demand_expansions", "ideal_rounds", "snapshot_candidates",
    "unused_snapshot_candidates", "prepare_index_reads", "demand_index_reads", "index_reads",
    "commit_neighbor_inspections", "staged_buffer_reads", "physical_slot_reads",
    "baseline_estimates", "estimates", "seed_estimates", "speculative_estimates", "demand_estimates",
    "unused_speculative_estimates", "cache_presence_checks", "cache_value_reads", "cache_value_writes",
    "modeled_cache_presence_read_bytes", "modeled_cache_value_read_bytes",
    "modeled_cache_write_bytes", "modeled_buffer_write_bytes", "modeled_buffer_read_bytes",
    "modeled_reset_bytes",
)


def ratio(numerator, denominator):
    require(type(numerator) is int and type(denominator) is int
            and numerator >= 0 and denominator > 0, "invalid ratio")
    return dict(numerator=numerator, denominator=denominator, value=numerator / denominator)


def within(numerator, denominator, cap_numerator, cap_denominator):
    require(type(numerator) is int and type(denominator) is int
            and numerator >= 0 and denominator > 0, "invalid gate counts")
    return cap_denominator * numerator <= cap_numerator * denominator


def p99(values):
    require(bool(values), "empty percentile input")
    ordered = sorted(values)
    return ordered[(99 * len(ordered) + 99) // 100 - 1]


class Preparation:
    def __init__(self, adjacency, size, entry):
        self.adjacency = adjacency
        self.size = size
        self.cache = {identifier(entry, size)}
        self.committed_offers = {entry}
        self.speculative_ids = set()
        self.groups = []
        self.current = None
        self.active = None
        self.counts = dict.fromkeys(ADDITIVE, 0)
        self.counts.update(seed_estimates=1, estimates=1, baseline_estimates=1, cache_value_writes=1)
        self.peak_buffer_slots = 0

    def begin(self, snapshot):
        require(self.counts["expansions"] % WINDOW == 0, "group began between boundaries")
        require(0 < len(snapshot) <= WINDOW and len(snapshot) == len(set(snapshot)), "invalid snapshot")
        if self.current is not None:
            self.close_group()
        buffers = {}
        prepared = []
        for vertex in snapshot:
            identifier(vertex, self.size)
            neighbors = tuple(self.adjacency[vertex])
            buffers[vertex] = neighbors
            self.counts["prepare_index_reads"] += len(neighbors)
            for neighbor in neighbors:
                self.counts["cache_presence_checks"] += 1
                if neighbor not in self.cache:
                    self.cache.add(neighbor)
                    self.speculative_ids.add(neighbor)
                    prepared.append(neighbor)
        self.counts["speculative_estimates"] += len(prepared)
        self.counts["estimates"] += len(prepared)
        self.counts["cache_value_writes"] += len(prepared)
        self.counts["snapshot_candidates"] += len(snapshot)
        self.peak_buffer_slots = max(self.peak_buffer_slots, sum(map(len, buffers.values())))
        self.current = dict(index=len(self.groups), begin_expansion=self.counts["expansions"],
                            snapshot=list(snapshot), prepared_ids=prepared, expansions=[])
        self.buffers = buffers

    def expand(self, vertex):
        require(self.current is not None and len(self.current["expansions"]) < WINDOW,
                "expansion without current group")
        staged = vertex in self.buffers
        neighbors = self.buffers[vertex] if staged else self.adjacency[vertex]
        self.counts["expansions"] += 1
        self.counts["commit_neighbor_inspections"] += len(neighbors)
        if staged:
            self.counts["staged_buffer_reads"] += len(neighbors)
        else:
            self.counts["demand_expansions"] += 1
            self.counts["demand_index_reads"] += len(neighbors)
        self.active = dict(id=vertex, staged=staged, neighbor_slots=len(neighbors), offers=[],
                           new_demand_ids=[], cache_value_reads=0)
        self.current["expansions"].append(self.active)

    def offer(self, vertex):
        require(self.active is not None and vertex not in self.committed_offers, "invalid commit offer")
        self.counts["cache_presence_checks"] += 1
        self.counts["baseline_estimates"] += 1
        if vertex in self.cache:
            self.counts["cache_value_reads"] += 1
            self.active["cache_value_reads"] += 1
        else:
            require(not self.active["staged"], "staged adjacency contains unprepared fresh estimate")
            self.cache.add(vertex)
            self.counts["estimates"] += 1
            self.counts["demand_estimates"] += 1
            self.counts["cache_value_writes"] += 1
            self.active["new_demand_ids"].append(vertex)
        self.committed_offers.add(vertex)
        self.active["offers"].append(vertex)

    def close_group(self):
        require(self.current is not None and bool(self.current["expansions"]), "empty group")
        committed = {event["id"] for event in self.current["expansions"]}
        unused = [vertex for vertex in self.current["snapshot"] if vertex not in committed]
        self.current["unused_snapshot"] = unused
        self.counts["unused_snapshot_candidates"] += len(unused)
        self.groups.append(self.current)
        self.current = None
        self.active = None
        self.buffers = {}

    def finish(self):
        self.close_group()
        counts = self.counts
        counts["groups"] = len(self.groups)
        require(counts["groups"] == (counts["expansions"] + WINDOW - 1) // WINDOW,
                "incorrect group count")
        counts["ideal_rounds"] = counts["groups"] + counts["demand_expansions"]
        counts["index_reads"] = counts["prepare_index_reads"] + counts["demand_index_reads"]
        counts["physical_slot_reads"] = counts["index_reads"] + counts["staged_buffer_reads"]
        unused = sorted(self.speculative_ids - self.committed_offers)
        counts["unused_speculative_estimates"] = len(unused)
        require(counts["estimates"] == len(self.cache) == counts["cache_value_writes"]
                == counts["seed_estimates"] + counts["speculative_estimates"] + counts["demand_estimates"],
                "estimate cache accounting mismatch")
        require(counts["baseline_estimates"] == len(self.committed_offers)
                and self.committed_offers <= self.cache
                and counts["estimates"] == counts["baseline_estimates"] + len(unused),
                "unused estimate accounting mismatch")
        require(counts["cache_presence_checks"] == counts["prepare_index_reads"]
                + counts["baseline_estimates"] - 1, "cache lookup accounting mismatch")
        require(counts["commit_neighbor_inspections"]
                == counts["demand_index_reads"] + counts["staged_buffer_reads"], "commit accounting mismatch")
        require(counts["physical_slot_reads"] == counts["commit_neighbor_inspections"]
                + counts["prepare_index_reads"], "double-read accounting mismatch")
        words = (self.size + 63) // 64
        counts["modeled_cache_presence_read_bytes"] = 8 * counts["cache_presence_checks"]
        counts["modeled_cache_value_read_bytes"] = 4 * counts["cache_value_reads"]
        counts["modeled_cache_write_bytes"] = 12 * counts["cache_value_writes"]
        counts["modeled_buffer_write_bytes"] = 4 * counts["prepare_index_reads"]
        counts["modeled_buffer_read_bytes"] = 4 * counts["staged_buffer_reads"]
        counts["modeled_reset_bytes"] = 8 * words + 128
        scratch = 4 * self.size + 8 * words + 4 * self.peak_buffer_slots + 4 * WINDOW + 128
        return dict(counts=counts, groups=self.groups, unused_speculative_ids=unused,
                    peak_cached_rows=len(self.cache), peak_buffer_slots=self.peak_buffer_slots,
                    modeled_scratch_bytes=scratch)


def simulate(events, adjacency, size, ef, entry, final_beam):
    require(type(events) is list and events, "missing events")
    require(type(ef) is int and ef > 0, "invalid beam capacity")
    pool = []
    expanded = set()
    offered = set()
    active = None
    pending = []
    position = 0
    preparation = None
    for event in events:
        vertex = identifier(event["id"], size)
        if event["kind"] == "expand":
            require(position == len(pending), "incomplete preceding expansion")
            candidates = [candidate for candidate in pool if candidate & 0xFFFFFFFF not in expanded]
            require(candidates and vertex == candidates[0] & 0xFFFFFFFF
                    and vertex in offered and vertex not in expanded, "replay expansion order mismatch")
            require(preparation is not None, "expansion before seed")
            if len(expanded) % WINDOW == 0:
                snapshot = [candidate & 0xFFFFFFFF for candidate in candidates[:WINDOW]]
                preparation.begin(snapshot)
                preparation.current["snapshot_keys_hex"] = [f"{candidate:016x}" for candidate in candidates[:WINDOW]]
            preparation.expand(vertex)
            expanded.add(vertex)
            active = vertex
            pending = [neighbor for neighbor in adjacency[vertex] if neighbor not in offered]
            position = 0
            continue
        require(event["kind"] == "offer" and vertex not in offered and event["stored_id"] == vertex,
                "invalid first-visit offer")
        candidate = key(event["distance"], vertex, size)
        if event["parent"] is None:
            require(not offered and active is None and vertex == entry, "unsupported recovery seed")
            preparation = Preparation(adjacency, size, entry)
        else:
            require(active is not None and event["parent"] == active and position < len(pending)
                    and vertex == pending[position], "offer parent/order mismatch")
            position += 1
            preparation.offer(vertex)
        worst = pool[-1] if len(pool) == ef else None
        recorded = event["worst"]
        require((recorded is None) == (worst is None), "replay worst-key presence mismatch")
        if worst is not None:
            require(type(recorded) is list and len(recorded) == 2
                    and key(recorded[0], recorded[1], size) == worst, "replay worst-key mismatch")
        admitted = worst is None or candidate < worst
        require(type(event["admitted"]) is bool and event["admitted"] == admitted,
                "replay scalar admission mismatch")
        require(event["evicted"] == ((worst & 0xFFFFFFFF) if admitted and worst is not None else None),
                "replay eviction mismatch")
        if admitted:
            if worst is not None:
                pool.pop()
            pool.insert(bisect.bisect_left(pool, candidate), candidate)
        offered.add(vertex)
    require(position == len(pending) and preparation is not None, "incomplete final expansion")
    require(all(candidate & 0xFFFFFFFF in expanded for candidate in pool), "undrained final replay beam")
    require([candidate & 0xFFFFFFFF for candidate in pool] == final_beam, "replay final beam mismatch")
    result = preparation.finish()
    require(result["counts"]["expansions"] == len(expanded)
            and result["counts"]["baseline_estimates"] == len(offered), "replay totals mismatch")
    return result


def summarize_query(query, result):
    counts = result["counts"]
    require(counts["expansions"] > 0 and counts["commit_neighbor_inspections"] > 0, "empty query work")
    return dict(query=query, counts=counts, peak_cached_rows=result["peak_cached_rows"],
                peak_buffer_slots=result["peak_buffer_slots"], modeled_scratch_bytes=result["modeled_scratch_bytes"],
                ratios=dict(rounds=ratio(counts["ideal_rounds"], counts["expansions"]),
                            index_reads=ratio(counts["index_reads"], counts["commit_neighbor_inspections"]),
                            estimates=ratio(counts["estimates"], counts["baseline_estimates"]),
                            physical_reads=ratio(counts["physical_slot_reads"], counts["commit_neighbor_inspections"])))


def summarize_cell(size, ef, queries):
    require(len(queries) == 1000, "incomplete query summary")
    totals = {name: sum(query["counts"][name] for query in queries) for name in ADDITIVE}
    tails = {}
    for name in ("rounds", "index_reads", "estimates", "physical_reads"):
        values = [(Fraction(query["ratios"][name]["numerator"], query["ratios"][name]["denominator"]),
                   query["query"]) for query in queries]
        value, query = p99(values)
        tails[name] = dict(**ratio(value.numerator, value.denominator), witness_query=query)
    scratch = max(query["modeled_scratch_bytes"] for query in queries)
    gates = dict(
        rounds=within(totals["ideal_rounds"], totals["expansions"], 1, 4),
        index_reads=within(totals["index_reads"], totals["commit_neighbor_inspections"], 5, 4),
        estimates=within(totals["estimates"], totals["baseline_estimates"], 5, 4),
        p99_index_reads=within(tails["index_reads"]["numerator"], tails["index_reads"]["denominator"], 3, 2),
        p99_estimates=within(tails["estimates"]["numerator"], tails["estimates"]["denominator"], 3, 2),
        scratch=scratch <= 16 * size,
    )
    return dict(rows=size, ef=ef, queries=len(queries), totals=totals, gates=gates,
                necessary_condition_passed=all(gates.values()),
                p99_per_query_ratios=tails,
                p99_counts={name: p99([query["counts"][name] for query in queries]) for name in ADDITIVE},
                max_modeled_scratch_bytes=scratch,
                max_peak_cached_rows=max(query["peak_cached_rows"] for query in queries),
                max_peak_buffer_slots=max(query["peak_buffer_slots"] for query in queries),
                aggregate_ratios=dict(rounds=ratio(totals["ideal_rounds"], totals["expansions"]),
                    index_reads=ratio(totals["index_reads"], totals["commit_neighbor_inspections"]),
                    estimates=ratio(totals["estimates"], totals["baseline_estimates"]),
                    physical_reads=ratio(totals["physical_slot_reads"], totals["commit_neighbor_inspections"])))


def collect(repo, pin, gold_pin, prior_cells, details, progress):
    size, archive, mode, receipt_sha, raw_sha, graph_hash = pin
    progress.update(phase="receipt", rows=size, ef=None, query=None)
    receipt_path = repo / f"docs/research/milestones-20261102/navigation-{size // 1000}k/finished.json"
    receipt = checked_json(receipt_path, receipt_sha)
    require(receipt["sourceCommit"] == SOURCE_COMMIT and receipt["complete"] is True
            and receipt["failure"] is None, "unverified receipt")
    require(receipt["efs"] == [128, 256] and receipt["queriesPerEf"] == 1000
            and receipt["result"]["originalRankingsMatched"] == 2000
            and receipt["result"]["originalExactRankingsMatched"] == 1000
            and receipt["result"]["rawSha256"] == raw_sha, "source verification incomplete")
    require(receipt["referenceFiles"]["oracle"] == gold_pin, "independent gold reference changed")
    gold = read_gold(Path(gold_pin["path"]), gold_pin["sha256"], size)
    raw = Path("/Volumes/A/.hms-bench-cache") / archive / "navigation.ndjson.gz"
    parity = {ef: dict.fromkeys(COUNTERS, 0) for ef in (128, 256)}
    for ef, cell in parity.items():
        cell.update(ef=ef, pending_histogram=Counter())
    queries = {ef: [] for ef in parity}
    digests = {ef: hashlib.sha256() for ef in parity}
    paths = {ef: details / f"queries-{size}-ef{ef}.ndjson.gz" for ef in parity}
    digest = hashlib.sha256()
    count = 0
    progress["phase"] = "archive"
    with ExitStack() as stack:
        outputs = {}
        for ef, path in paths.items():
            file = stack.enter_context(path.open("xb"))
            outputs[ef] = stack.enter_context(gzip.GzipFile(filename="", mode="wb", fileobj=file, mtime=0))
        source = stack.enter_context(gzip.open(raw, "rb"))
        while line := source.readline(128 * 1024 * 1024 + 1):
            require(len(line) <= 128 * 1024 * 1024, "oversized trace record")
            digest.update(line)
            row = json.loads(line)
            count += 1
            progress["line"] = count
            if count == 1:
                require(row["type"] == "start" and row["mode"] == mode and row["graphHash"] == graph_hash
                        and row["indexBreakdown"]["original_ids_bytes"] == 0, "header identity mismatch")
                if size == 100000:
                    require("rows" not in row and "corpusRows" not in receipt
                            and receipt["expectedGraphHash"] == graph_hash, "unexpected legacy100k schema")
                else:
                    require(row["rows"] == receipt["corpusRows"] == size
                            and receipt["result"]["graphHash"] == graph_hash, "unexpected10k schema")
                continue
            if count == 2:
                require(row["type"] == "topology" and type(row["adjacency"]) is list
                        and len(row["adjacency"]) == size, "invalid topology")
                adjacency = row["adjacency"]
                for neighbors in adjacency:
                    require(type(neighbors) is list and len(neighbors) <= 256
                            and len(neighbors) == len(set(neighbors)), "invalid adjacency list")
                    for vertex in neighbors:
                        identifier(vertex, size)
                shapes = [packet_shape(neighbors) for neighbors in adjacency]
                continue
            require(count <= 2002 and row["type"] == "navigation"
                    and row["ef"] == (128 if count <= 1002 else 256), "unexpected trace order/schema")
            ef = row["ef"]
            query = row["query"]
            progress.update(ef=ef, query=query)
            require(type(query) is int and query == len(queries[ef]), "missing or repeated query")
            require(row["oracle"] == gold[query], "trace gold differs from independent gold")
            before = {name: parity[ef][name] for name in ("expansions", "adjacency_edges", "offers", "seed_offers")}
            inspect_query(row, adjacency, shapes, parity[ef], size)
            result = simulate(row["events"], adjacency, size, ef, row["entry"], row["beam"])
            counts = result["counts"]
            require(counts["expansions"] == parity[ef]["expansions"] - before["expansions"]
                    and counts["commit_neighbor_inspections"] == parity[ef]["adjacency_edges"] - before["adjacency_edges"]
                    and counts["baseline_estimates"] == parity[ef]["offers"] - before["offers"]
                    and parity[ef]["seed_offers"] - before["seed_offers"] == 1, "query differs from scalar oracle")
            summary = summarize_query(query, result)
            result.update(query=query, rows=size, ef=ef, ratios=summary["ratios"],
                          unchanged_selected_hits=len(set(row["stages"]["selected"]) & set(gold[query])))
            encoded = (json.dumps(result, separators=(",", ":"), allow_nan=False) + "\n").encode()
            outputs[ef].write(encoded)
            digests[ef].update(encoded)
            queries[ef].append(summary)
    require(count == 2002 and digest.hexdigest() == raw_sha, "archive hash/record count mismatch")
    summaries = []
    for ef in parity:
        old = prior_cells[(size, ef)]
        require(all(parity[ef][name] == old[name] for name in COUNTERS), "scalar counters differ from E21")
        summary = summarize_cell(size, ef, queries[ef])
        require(summary["totals"]["expansions"] == old["expansions"]
                and summary["totals"]["commit_neighbor_inspections"] == old["adjacency_edges"]
                and summary["totals"]["baseline_estimates"] == old["offers"], "cell baseline differs from E21")
        summary.update(details_path=str(paths[ef].resolve()), details_raw_sha256=digests[ef].hexdigest(),
                       details_gzip_sha256=sha256(paths[ef]), raw_path=str(raw), raw_sha256=raw_sha,
                       receipt_path=str(receipt_path), receipt_sha256=receipt_sha, oracle_source=gold_pin,
                       scalar_oracle_counters_matched=True)
        write_json(details / f"summary-{size}-ef{ef}.json", summary)
        summaries.append(summary)
    return summaries


def append_log(args, decision, outcome, failure=False):
    with args.ledger.open("a", newline="") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        csv.writer(stream, delimiter="\t").writerow([
            args.experiment, "experiment", "H-A-6", "static-development", 2, QUESTION, PREDICTION,
            outcome, decision, SOURCE_COMMIT, "window8;R<=U/4;A/Q<=1.25;p99<=1.5;all4cells",
            platform.node(), str(args.failure.resolve() if failure else args.output.resolve()),
        ])
        stream.flush()
        os.fsync(stream.fileno())


def run(args, progress):
    require(args.repo.is_dir() and args.ledger.is_file(), "missing repo or ledger")
    for path in (args.output, args.details, args.failure):
        require(not path.exists() and not path.is_symlink(), "output, details or failure path already exists")
    require(args.output != args.failure and args.output != args.details and args.failure != args.details,
            "overlapping output paths")
    own = Path(__file__).resolve().parent
    check = json.loads(args.check_receipt.read_bytes())
    require(check["status"] == "pass" and len(check["cases"]) == 5
            and all(case["passed"] is True for case in check["cases"]), "missing successful staging check")
    for name in ("speculation_counter.py", "speculation_counter_check.py"):
        require(check["source_hashes"][name] == sha256(own / name), "staging check source changed")
    pins = {"packet_eligibility.py": PARSER_SHA, "effort_allocation.py": HELPER_SHA,
            "research5.json": GOLD_RESEARCH_SHA, "next-card6.md": CARD_SHA, "research6.json": RESEARCH_SHA}
    for name, expected in pins.items():
        require(sha256(own / name) == expected, f"frozen dependency changed: {name}")
    require(sha256(args.repo / "src/core/qgraph/vertex.rs") == VERTEX_SHA256, "scalar source pin changed")
    research = checked_json(own / "research6.json", RESEARCH_SHA)
    gold_research = checked_json(own / "research5.json", GOLD_RESEARCH_SHA)
    prior = checked_json(args.repo / "benchmarks/results/mission/root/E21/result.json", E21_SHA)
    require(prior["experiment"] == "E21" and prior["script_sha256"] == PARSER_SHA, "E21 provenance mismatch")
    prior_cells = {(archive["rows"], cell["ef"]): cell for archive in prior["archives"] for cell in archive["cells"]}
    require(len(prior["archives"]) == 2 and set(prior_cells) == {(size, ef) for size in (10000, 100000)
                                                              for ef in (128, 256)}, "incomplete E21 cells")
    args.details.mkdir(parents=True, exist_ok=False)
    manifest = dict(experiment=args.experiment, hypothesis="H-A-6", question=QUESTION, prediction=PREDICTION,
        window=WINDOW, max_seconds=args.max_seconds, source_commit=SOURCE_COMMIT, source_vertex_sha256=VERTEX_SHA256,
        dependency_sha256=pins, script_sha256=sha256(Path(__file__)), e21_sha256=E21_SHA,
        check_receipt=str(args.check_receipt.resolve()), check_receipt_sha256=sha256(args.check_receipt),
        preregistration=research["frozen_counter_specification"],
        inputs=gold_research["exact_counter_design"]["inputs"],
        percentile="Nearest rank: sorted per-query values at ceil(.99*n), with exact rational ratio ordering.",
        scratch_model="Proposed f32 score array4*N, 64-bit validity words8*ceil(N/64), u32 group adjacency buffers, eight u32 candidate IDs and128 control bytes. Scores need no zeroing; reset validity words and control bytes once/query. This is modeled serving scratch/traffic, not Python RSS or a native allocation measurement.",
        cache_traffic_model="Each presence check reads one8-byte validity word; each new estimate writes one4-byte score and one8-byte validity word; each reused committed estimate reads4bytes. Group buffer writes/reads are4bytes/ID. Index ID byte width and actual cache-line misses are not inferred.")
    write_json(args.details / "manifest.json", manifest)
    cells = []
    sources = manifest["inputs"]
    require(len(sources) == len(PINS), "wrong input pin count")
    for pin, source in zip(PINS, sources):
        require(source["size"] == f"{pin[0] // 1000}k", "input pin order mismatch")
        cells.extend(collect(args.repo, pin, source["oracle_reference"], prior_cells, args.details, progress))
    progress.update(phase="finalize", rows=None, ef=None, query=None)
    passed = all(cell["necessary_condition_passed"] for cell in cells)
    result = dict(status="complete", experiment=args.experiment, hypothesis="H-A-6", claim="local", rung=2,
        question=QUESTION, prediction=PREDICTION, decision="compose" if passed else "reject",
        necessary_condition_passed=passed, performance_acceptance=False, matched_gap_condition="UNPROVED",
        window=WINDOW, integrity_failures=0, cells=cells,
        manifest=str((args.details / "manifest.json").resolve()), manifest_sha256=sha256(args.details / "manifest.json"),
        script_sha256=manifest["script_sha256"],
        limits="Exposed single-hop development traces; logical rounds and modeled scratch are not timing/memory measurements. Full physical slot reads include staged buffers. Original traversal and imperfect recall remain unchanged. No native implementation, new search, matched comparison or fresh confirmation.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    write_json(args.output, result)
    summary = "; ".join(f"{cell['rows']}/ef{cell['ef']}:R/U={cell['aggregate_ratios']['rounds']['value']:.6f},"
                        f"A/E={cell['aggregate_ratios']['index_reads']['value']:.6f},"
                        f"Q/Q0={cell['aggregate_ratios']['estimates']['value']:.6f},"
                        f"pass={cell['necessary_condition_passed']}" for cell in cells)
    append_log(args, result["decision"], summary)
    print(f"{args.experiment} {result['decision']}: {summary}; performance unproved; {args.output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--details", type=Path, required=True)
    parser.add_argument("--failure", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--check-receipt", type=Path, required=True)
    parser.add_argument("--experiment", choices=("E37",), required=True)
    parser.add_argument("--max-seconds", type=int, default=900)
    args = parser.parse_args()
    progress = dict(phase="preflight", rows=None, ef=None, query=None)

    def interrupted(number, frame):
        if number == signal.SIGALRM:
            raise TimeoutError("counter exceeded the registered wall-time ceiling")
        raise InterruptedError(f"counter interrupted by signal {number}")

    try:
        require(1 <= args.max_seconds <= 3600, "invalid wall-time ceiling")
        signal.signal(signal.SIGALRM, interrupted)
        signal.signal(signal.SIGTERM, interrupted)
        signal.setitimer(signal.ITIMER_REAL, args.max_seconds)
        run(args, progress)
    except (Exception, KeyboardInterrupt) as error:
        signal.setitimer(signal.ITIMER_REAL, 0)
        artifacts = []
        if args.details.is_dir():
            for path in sorted(args.details.iterdir()):
                if path.is_file() and not path.is_symlink():
                    artifacts.append(dict(path=str(path.resolve()), bytes=path.stat().st_size, sha256=sha256(path)))
        failure = dict(status="failed", experiment=args.experiment, hypothesis="H-A-6",
            decision=None, necessary_condition_passed=None, performance_acceptance=False,
            progress=progress, error_type=type(error).__name__, error=str(error),
            script_sha256=sha256(Path(__file__)), partial_artifacts=artifacts,
            output_exists=args.output.exists(),
            limits="Failed integrity/execution is not a mechanism rejection or result. Partial artifacts are retained; never resume or overwrite this experiment without a new registered ID.")
        try:
            args.failure.parent.mkdir(parents=True, exist_ok=True)
            write_json(args.failure, failure)
        except Exception as receipt_error:
            print(f"E37 failure receipt could not be written: {type(receipt_error).__name__}: {receipt_error}", file=sys.stderr)
        try:
            if args.ledger.is_file():
                append_log(args, "reject", f"execution_failure, not a mechanism rejection: {type(error).__name__}: {error}; progress={json.dumps(progress, sort_keys=True)}", failure=True)
        except Exception as log_error:
            print(f"E37 failure ledger append failed: {type(log_error).__name__}: {log_error}", file=sys.stderr)
        print(f"E37 failed: {type(error).__name__}: {error}; receipt {args.failure}", file=sys.stderr)
        return 1
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
