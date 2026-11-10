# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Count fixed-eight packet eligibility in pinned, previously verified traces."""

import argparse
import bisect
import csv
import fcntl
import gzip
import hashlib
import heapq
import itertools
import json
import math
import os
import platform
import struct
from collections import Counter
from pathlib import Path


SOURCE_COMMIT = "ab978afd94a2d92a89c162c1e2af77372ffeeb16"
VERTEX_SHA256 = "98dbb33e82c316b02ff815f5f098d5f1c3fb3f2b4752bcfd19ff54ab1ee62508"
PINS = (
    (100000, "warm-navigation-20261102-v1", "navigation",
     "ecf21d85bd3ab7294bd1b32af8e7c3d45e447e5a29dbdc3aeec5d2b3715756aa",
     "2fa90ae2c8694787728bf4ca81cd1f6c49fc4cf17280da0b8675d40a7c19812e",
     "b032161111580eb9"),
    (10000, "warm-navigation-10k-20261102-v1", "navigation10k",
     "097400b995962ca71e875827890e47b3a4b1aeb1e9bccce6c978db0e5547dca2",
     "61fd4343b36ae8d2b60ed4f58f24a46c8e247e572b6ce307f10a107127354ba5",
     "1252cb6267c5dd80"),
)
COUNTERS = (
    "queries", "expansions", "adjacency_edges", "full_packets", "eligible_packets",
    "conflicting_packets", "word_collision_pairs", "eligible_edges", "scalar_tail_edges",
    "fresh_edges", "fresh_eligible_edges", "fresh_conflicting_edges", "fresh_tail_edges",
    "offers", "seed_offers", "scalar_key_shifts", "scalar_key_writes",
    "expansion_scalar_key_writes", "merge_groups", "merge_groups_ge8",
    "merge_pending_before_filter", "merge_pending_after_filter", "merge_filter_comparisons",
    "merge_pending_in_ge8_groups", "merge_output_key_writes", "batch_pending_key_writes",
    "batch_key_write_lower_bound",
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def identifier(value, size):
    require(type(value) is int and 0 <= value < size, "invalid vertex ID")
    return value


def key(distance, stored_id, size):
    identifier(stored_id, size)
    require(type(distance) in (int, float) and math.isfinite(distance), "invalid distance")
    packed = struct.pack("<f", distance)
    require(struct.unpack("<f", packed)[0] == distance, "distance is not an exact f32")
    bits = struct.unpack("<I", packed)[0]
    ordered = (~bits & 0xFFFFFFFF) if bits >> 31 else bits | 0x80000000
    return (ordered << 32) | stored_id


def packet_shape(neighbors):
    stop = len(neighbors) // 8 * 8
    eligible = conflicts = pairs = mask = 0
    for begin in range(0, stop, 8):
        counts = Counter(vertex >> 6 for vertex in neighbors[begin:begin + 8])
        if len(counts) == 8:
            eligible += 1
            mask |= 255 << begin
        else:
            conflicts += 1
            pairs += sum(count * (count - 1) // 2 for count in counts.values())
    return eligible, conflicts, pairs, len(neighbors) - stop, mask


def inspect_query(row, adjacency, shapes, cell, size):
    require(row["query"] == cell["queries"], "missing or duplicate query")
    ef = row["ef"]
    pool = []
    seen = set()
    expanded = set()
    active = None
    selected = row["stages"]["selected"]
    require(len(selected) == len(set(selected)) == 10, "invalid selected results")
    for vertex in selected:
        identifier(vertex, size)

    def finish():
        if active is None:
            return
        require(active["position"] == len(active["fresh"]), "unscored outgoing neighbors")
        before = active["before"]
        offers = active["keys"]
        pending = [candidate for candidate in offers if candidate < before[-1]] if len(before) == ef else offers
        merged = list(itertools.islice(heapq.merge(before, sorted(pending)), ef))
        require(merged == pool, "existing batch merge changes recorded beam")
        cell["merge_groups"] += 1
        cell["merge_pending_before_filter"] += len(offers)
        cell["merge_pending_after_filter"] += len(pending)
        cell["merge_filter_comparisons"] += len(offers) if len(before) == ef else 0
        cell["pending_histogram"][len(pending)] += 1
        cell["expansion_scalar_key_writes"] += active["writes"]
        cell["batch_pending_key_writes"] += len(offers)
        if len(pending) >= 8:
            cell["merge_groups_ge8"] += 1
            cell["merge_pending_in_ge8_groups"] += len(pending)
            cell["merge_output_key_writes"] += len(merged)
            cell["batch_key_write_lower_bound"] += len(offers) + len(merged)
        else:
            cell["batch_key_write_lower_bound"] += len(offers) + active["writes"]

    for event in row["events"]:
        vertex = identifier(event["id"], size)
        if event["kind"] == "expand":
            finish()
            expected = next((candidate & 0xFFFFFFFF for candidate in pool
                             if candidate & 0xFFFFFFFF not in expanded), None)
            require(vertex == expected and vertex in seen and vertex not in expanded,
                    "invalid best-first expansion")
            expanded.add(vertex)
            neighbors = adjacency[vertex]
            eligible, conflicts, pairs, tail, mask = shapes[vertex]
            fresh = []
            for position, neighbor in enumerate(neighbors):
                if neighbor not in seen:
                    fresh.append(neighbor)
                    cell["fresh_edges"] += 1
                    if mask >> position & 1:
                        cell["fresh_eligible_edges"] += 1
                    elif position >= len(neighbors) - tail:
                        cell["fresh_tail_edges"] += 1
                    else:
                        cell["fresh_conflicting_edges"] += 1
            cell["expansions"] += 1
            cell["adjacency_edges"] += len(neighbors)
            cell["full_packets"] += eligible + conflicts
            cell["eligible_packets"] += eligible
            cell["conflicting_packets"] += conflicts
            cell["word_collision_pairs"] += pairs
            cell["eligible_edges"] += 8 * eligible
            cell["scalar_tail_edges"] += tail
            active = dict(vertex=vertex, before=pool.copy(), fresh=fresh, position=0, keys=[], writes=0)
            continue
        require(event["kind"] == "offer" and vertex not in seen, "invalid or repeated offer")
        require(event["stored_id"] == vertex, "archive is not in stored-ID order")
        candidate = key(event["distance"], event["stored_id"], size)
        if event["parent"] is None:
            finish()
            active = None
            require(all(value & 0xFFFFFFFF in expanded for value in pool) and len(pool) < 18,
                    "invalid recovery seed")
            if not seen:
                require(vertex == row["entry"], "entry seed mismatch")
            cell["seed_offers"] += 1
        else:
            require(active is not None and event["parent"] == active["vertex"], "incorrect parent")
            require(active["position"] < len(active["fresh"])
                    and active["fresh"][active["position"]] == vertex, "incorrect first-visit order")
            active["position"] += 1
            active["keys"].append(candidate)
        worst = pool[-1] if len(pool) == ef else None
        recorded_worst = event["worst"]
        require((recorded_worst is None) == (worst is None), "worst-key presence mismatch")
        if recorded_worst is not None:
            require(type(recorded_worst) is list and len(recorded_worst) == 2
                    and key(recorded_worst[0], recorded_worst[1], size) == worst, "worst key mismatch")
        admitted = worst is None or candidate < worst
        require(type(event["admitted"]) is bool and event["admitted"] == admitted,
                "incorrect scalar admission")
        require(event["evicted"] == ((worst & 0xFFFFFFFF) if admitted and worst is not None else None),
                "incorrect eviction")
        if admitted:
            position = bisect.bisect_left(pool, candidate)
            if worst is not None:
                pool.pop()
            shifts = len(pool) - position
            require(shifts >= 0, "invalid insertion position")
            pool.insert(position, candidate)
            cell["scalar_key_shifts"] += shifts
            cell["scalar_key_writes"] += shifts + 1
            if active is not None:
                active["writes"] += shifts + 1
        seen.add(vertex)
        cell["offers"] += 1
    finish()
    require(all(candidate & 0xFFFFFFFF in expanded for candidate in pool), "undrained final beam")
    require([candidate & 0xFFFFFFFF for candidate in pool] == row["beam"], "final beam mismatch")
    require(seen.issubset(set(row["evaluated"])), "scored ID missing from evaluated set")
    cell["queries"] += 1


def inspect_archive(repo, pin):
    size, archive, mode, receipt_sha, raw_sha, graph_hash = pin
    receipt_path = repo / f"docs/research/milestones-20261102/navigation-{size // 1000}k/finished.json"
    receipt_bytes = receipt_path.read_bytes()
    require(hashlib.sha256(receipt_bytes).hexdigest() == receipt_sha, "source receipt hash mismatch")
    receipt = json.loads(receipt_bytes)
    require(receipt["sourceCommit"] == SOURCE_COMMIT and receipt["complete"] and receipt["failure"] is None,
            "unverified source receipt")
    require(receipt["efs"] == [128, 256] and receipt["queriesPerEf"] == 1000,
            "unexpected query configuration")
    require(receipt["result"]["originalRankingsMatched"] == 2000
            and receipt["result"]["originalExactRankingsMatched"] == 1000
            and receipt["result"]["rawSha256"] == raw_sha, "source verification incomplete")
    raw = Path("/Volumes/A/.hms-bench-cache") / archive / "navigation.ndjson.gz"
    digest = hashlib.sha256()
    cells = {ef: dict.fromkeys(COUNTERS, 0) for ef in (128, 256)}
    for ef, cell in cells.items():
        cell.update(ef=ef, pending_histogram=Counter())
    line_count = 0
    with gzip.open(raw, "rb") as stream:
        while line := stream.readline(128 * 1024 * 1024 + 1):
            require(len(line) <= 128 * 1024 * 1024, "oversized trace record")
            digest.update(line)
            row = json.loads(line)
            line_count += 1
            if line_count == 1:
                require(row["type"] == "start" and row["mode"] == mode and row["graphHash"] == graph_hash,
                        "header identity mismatch")
                require(row["indexBreakdown"]["original_ids_bytes"] == 0, "stored/original mapping present")
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
                shapes = [packet_shape(neighbors) for neighbors in adjacency]
                continue
            require(row["type"] == "navigation" and row["ef"] in cells, "unexpected record")
            require(line_count <= 2002 and row["ef"] == (128 if line_count <= 1002 else 256),
                    "unexpected cell order or record count")
            inspect_query(row, adjacency, shapes, cells[row["ef"]], size)
    require(line_count == 2002 and digest.hexdigest() == raw_sha, "incomplete or changed raw archive")
    for cell in cells.values():
        edges = cell["adjacency_edges"]
        require(cell["queries"] == 1000 and cell["expansions"] > 0 and edges > 0, "incomplete cell")
        require(edges == cell["eligible_edges"] + 8 * cell["conflicting_packets"] + cell["scalar_tail_edges"],
                "packet accounting mismatch")
        require(cell["fresh_edges"] == cell["fresh_eligible_edges"] + cell["fresh_conflicting_edges"]
                + cell["fresh_tail_edges"] == cell["offers"] - cell["seed_offers"], "fresh accounting mismatch")
        cell["eligible_edge_share"] = cell["eligible_edges"] / edges
        cell["conflicting_edge_share"] = 8 * cell["conflicting_packets"] / edges
        cell["scalar_tail_share"] = cell["scalar_tail_edges"] / edges
        cell["merge_group_share_ge8"] = cell["merge_groups_ge8"] / cell["merge_groups"]
        pending = cell["merge_pending_after_filter"]
        cell["merge_pending_share_ge8"] = cell["merge_pending_in_ge8_groups"] / pending if pending else None
        writes = cell["expansion_scalar_key_writes"]
        cell["optimistic_batch_to_scalar_key_write_ratio"] = cell["batch_key_write_lower_bound"] / writes if writes else None
        cell["pending_histogram"] = dict(sorted(cell["pending_histogram"].items()))
        cell["necessary_condition"] = "reject" if 10 * cell["eligible_edges"] < 9 * edges else "not_rejected"
    return dict(rows=size, archive=str(raw), raw_sha256=raw_sha, source_commit=SOURCE_COMMIT,
                receipt=str(receipt_path), receipt_sha256=receipt_sha, cells=list(cells.values()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--experiment", choices=("E21",), required=True)
    args = parser.parse_args()
    require(not args.output.exists() and args.ledger.is_file(), "output exists or ledger missing")
    vertex = args.repo / "src/core/qgraph/vertex.rs"
    require(hashlib.sha256(vertex.read_bytes()).hexdigest() == VERTEX_SHA256, "Marks/Pool source pin changed")
    archives = [inspect_archive(args.repo, pin) for pin in PINS]
    rejected = any(cell["necessary_condition"] == "reject" for archive in archives for cell in archive["cells"])
    decision = "reject" if rejected else "compose"
    value = dict(experiment=args.experiment, hypothesis="H-A-4", claim="local", rung=2,
                 question="Does fixed-eight packet eligibility meet its necessary90% edge-share condition in all frozen cells?",
                 prediction="at least one10k cell has eligible edge share<.90; no packet-size sweep",
                 decision=decision, necessary_condition="reject" if rejected else "not_rejected",
                 packet_size=8, visited_word_bits=64, threshold_numerator=9, threshold_denominator=10,
                 matched_gap_condition="UNPROVED", performance_acceptance=False,
                 source_vertex_sha256=VERTEX_SHA256,
                 script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), archives=archives,
                 limits="Exposed development traces only. No new search, profile, timing, holdout or comparator run. Key-write bound excludes sorting, retain movement, flags, comparisons, allocation and cache effects; its success cannot prove merge speedup. No mechanism gain or matched W/C accepted.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    summary = "; ".join(f"{archive['rows']}/ef{cell['ef']}:eligible={cell['eligible_edge_share']:.6f},tail={cell['scalar_tail_share']:.6f},merge_ge8={cell['merge_group_share_ge8']:.6f}"
                        for archive in archives for cell in archive["cells"])
    with args.ledger.open("a", newline="") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        csv.writer(stream, delimiter="\t").writerow([
            args.experiment, "experiment", "H-A-4", "static-development", 2, value["question"],
            value["prediction"], summary, decision, SOURCE_COMMIT, "fixed8;word64;10k/100k;ef128/256",
            platform.node(), str(args.output.resolve()),
        ])
        stream.flush()
        os.fsync(stream.fileno())
    print(f"{args.experiment} {value['necessary_condition']}: {summary}; matched gap UNPROVED; {args.output}")


if __name__ == "__main__":
    main()
