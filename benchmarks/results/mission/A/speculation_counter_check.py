# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check staging state against independent finite expansion timelines."""

import argparse
from pathlib import Path

from speculation_counter import WINDOW, p99, sha256, simulate, within, write_json


def trace(adjacency, distances, ef):
    pool = []
    seen = set()
    expanded = set()
    events = []
    timeline = []

    def offer(vertex, parent):
        if vertex in seen:
            return
        seen.add(vertex)
        candidate = (distances[vertex], vertex)
        worst = pool[-1] if len(pool) == ef else None
        admitted = worst is None or candidate < worst
        events.append(dict(kind="offer", id=vertex, stored_id=vertex, parent=parent,
                           distance=distances[vertex], worst=list(worst) if worst is not None else None,
                           admitted=admitted, evicted=worst[1] if admitted and worst is not None else None))
        if admitted:
            pool.append(candidate)
            pool.sort()
            del pool[ef:]

    offer(0, None)
    while True:
        available = [vertex for _, vertex in pool if vertex not in expanded]
        if not available:
            break
        vertex = available[0]
        timeline.append(dict(frontier=available, vertex=vertex, offers=[]))
        expanded.add(vertex)
        events.append(dict(kind="expand", id=vertex))
        for neighbor in adjacency[vertex]:
            if neighbor not in seen:
                timeline[-1]["offers"].append(neighbor)
                offer(neighbor, vertex)
    return events, timeline, [vertex for _, vertex in pool]


def enumerate_staging(timeline, adjacency):
    cache = {0}
    offered = {0}
    estimates = 1
    reads = demand = buffered = prep_reads = 0
    groups = []
    for offset in range(0, len(timeline), 8):
        steps = timeline[offset:offset + 8]
        snapshot = steps[0]["frontier"][:8]
        prepared = []
        for vertex in snapshot:
            reads += len(adjacency[vertex])
            prep_reads += len(adjacency[vertex])
            for neighbor in adjacency[vertex]:
                if neighbor not in cache:
                    prepared.append(neighbor)
                    cache.add(neighbor)
                    estimates += 1
        committed = []
        for step in steps:
            vertex = step["vertex"]
            committed.append(vertex)
            if vertex in snapshot:
                buffered += len(adjacency[vertex])
            else:
                demand += 1
                reads += len(adjacency[vertex])
            for neighbor in step["offers"]:
                offered.add(neighbor)
                if neighbor not in cache:
                    cache.add(neighbor)
                    estimates += 1
        groups.append((snapshot, prepared, committed))
    return dict(groups=groups, estimates=estimates, index_reads=reads,
                prepare_index_reads=prep_reads, staged_buffer_reads=buffered,
                ideal_rounds=len(groups) + demand, demand_expansions=demand,
                physical_slot_reads=reads + buffered, unused=sorted(cache - offered))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert WINDOW == 8
    cases = []
    for count in (7, 8, 9):
        adjacency = [[vertex + 1] if vertex + 1 < count else [] for vertex in range(count)]
        cases.append((f"chain_{count}", adjacency, [float(count - vertex) for vertex in range(count)], count))
    adjacency = [list(range(1, 17))] + [[] for _ in range(32)]
    adjacency[1] = [17]
    adjacency[2] = [18]
    adjacency[3] = [19]
    adjacency[9] = [20, 21]
    adjacency[10] = [21, 22]
    distances = [100.0] + [float(vertex) for vertex in range(1, 33)]
    distances[17:20] = [0.25, 0.5, 0.75]
    cases.append(("new_best_during_window_and_duplicate_speculation", adjacency, distances, 24))
    adjacency = [list(range(1, 17))] + [[] for _ in range(32)]
    adjacency[8] = [17, 18, 19, 20, 21, 22, 23, 24]
    adjacency[15] = [25, 26]
    adjacency[16] = [26, 27]
    distances = [100.0] + [float(vertex) for vertex in range(1, 33)]
    distances[17:25] = [0.25] * 8
    cases.append(("prepared_then_evicted_and_unused_scores", adjacency, distances, 16))
    receipts = []
    for name, adjacency, distances, ef in cases:
        events, timeline, beam = trace(adjacency, distances, ef)
        expected = enumerate_staging(timeline, adjacency)
        actual = simulate(events, adjacency, len(adjacency), ef, 0, beam)
        for field in ("estimates", "index_reads", "prepare_index_reads", "staged_buffer_reads",
                      "ideal_rounds", "demand_expansions", "physical_slot_reads"):
            assert actual["counts"][field] == expected[field], (name, field, actual["counts"], expected)
        assert actual["unused_speculative_ids"] == expected["unused"], name
        assert len(actual["groups"]) == len(expected["groups"]), name
        for group, (snapshot, prepared, committed) in zip(actual["groups"], expected["groups"]):
            assert group["snapshot"] == snapshot and group["prepared_ids"] == prepared, name
            assert [event["id"] for event in group["expansions"]] == committed, name
        if name.startswith("chain_"):
            count = len(adjacency)
            assert actual["counts"]["expansions"] == count
            assert actual["counts"]["ideal_rounds"] == count
            assert actual["groups"][0]["snapshot"] == [0]
            assert actual["counts"]["groups"] == (count + 7) // 8
        if name == "prepared_then_evicted_and_unused_scores":
            assert actual["unused_speculative_ids"], "fixture did not exercise unused speculative scores"
            assert actual["counts"]["unused_snapshot_candidates"] > 0
        receipts.append(dict(case=name, expansions=len(timeline), counts=actual["counts"], passed=True))
    for cap_numerator, cap_denominator in ((1, 4), (5, 4), (3, 2)):
        denominator = cap_denominator * 10
        for numerator in (cap_numerator * 10 - 1, cap_numerator * 10, cap_numerator * 10 + 1):
            assert within(numerator, denominator, cap_numerator, cap_denominator) == (numerator <= cap_numerator * 10)
    assert p99(list(range(1000))) == 989
    own = Path(__file__).resolve().parent
    result = dict(status="pass", scope="Finite staging-state checks only; scalar beam oracle remains the pinned E21 parser.",
        method="Independent tuple-key scalar timelines, sliced into groups, plus explicit7/8/9 chain expectations.",
        cases=receipts, threshold_boundary_checks=9, percentile_check=True,
        source_hashes={name: sha256(own / name) for name in ("speculation_counter.py", "speculation_counter_check.py")})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    write_json(args.output, result)
    print(f"speculative staging check passed: {len(cases)} cases; {args.output}")


if __name__ == "__main__":
    main()
