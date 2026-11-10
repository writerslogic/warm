# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check the finite-prefix optimizer against independent exhaustive enumeration."""

import argparse
import hashlib
import itertools
import json
from pathlib import Path

from effort_allocation import meets_frontier, optimize_prefixes


def exhaustive(options, budget, gold_count):
    candidates = []
    for selection in itertools.product(*options):
        misses = sum(gold_count - option[0] for option in selection)
        if misses <= budget:
            candidates.append((sum(option[1] for option in selection), misses))
    return min(candidates) if candidates else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    cases = {
        "nonconcave_unlock": (2, [[(0, 0, 0), (1, 100, 1), (2, 101, 2)],
                                  [(0, 0, 0), (1, 60, 1), (2, 120, 2)]]),
        "unattainable_perfect": (2, [[(0, 1, 0), (1, 4, 1)], [(0, 1, 0), (2, 6, 1)]]),
        "equal_cost_more_hits": (2, [[(0, 0, 0), (1, 0, 1), (2, 0, 2)]]),
        "skipped_hit_counts": (3, [[(0, 1, 0), (2, 5, 1), (3, 12, 2)],
                                   [(1, 1, 0), (3, 7, 1)], [(0, 1, 0), (3, 9, 1)]]),
        "duplicate_hits": (2, [[(0, 1, 0), (1, 4, 1), (1, 7, 2), (2, 8, 3)]]),
    }
    checks = []
    for name, (gold_count, options) in cases.items():
        for budget in range(gold_count * len(options) + 1):
            expected = exhaustive(options, budget, gold_count)
            actual = optimize_prefixes(options, budget, gold_count)
            assert actual["feasible"] == (expected is not None), (name, budget, expected, actual)
            if expected is None:
                assert actual["cost"] is None and actual["option_indices"] is None
            else:
                assert (actual["cost"], actual["missed"]) == expected, (name, budget, expected, actual)
                selection = [query[index] for query, index in zip(options, actual["option_indices"])]
                assert len(selection) == len(options)
                assert sum(option[1] for option in selection) == expected[0]
                assert sum(gold_count - option[0] for option in selection) == expected[1]
            checks.append(dict(case=name, missed_budget=budget, expected=expected, passed=True))
    assert optimize_prefixes(cases["nonconcave_unlock"][1], 2, 2)["cost"] == 101
    read_options = [[(0, 0, 0), (1, 1, 1)], [(0, 0, 0), (1, 9, 1)]]
    estimate_options = [[(0, 0, 0), (1, 9, 1)], [(0, 0, 0), (1, 1, 1)]]
    read = optimize_prefixes(read_options, 3, 2)
    estimate = optimize_prefixes(estimate_options, 3, 2)
    assert read["option_indices"] == [1, 0] and estimate["option_indices"] == [0, 1]
    for cost, baseline, expected in [(33, 98, False), (33, 99, True), (33, 100, True),
                                      (32, 99, True), (34, 99, False)]:
        assert meets_frontier(cost, baseline) is expected
    for bad_budget in (-1, 3):
        try:
            optimize_prefixes([[(0, 0, 0), (2, 1, 1)]], bad_budget, 2)
        except ValueError:
            pass
        else:
            raise AssertionError("out-of-range budget accepted")
    own = Path(__file__).resolve().parent
    value = dict(status="pass", method="independent Cartesian-product enumeration of every feasible combination",
                 optimizer_cases=checks, independent_objective_witness_check=True,
                 exact_threefold_boundary_checks=5, invalid_budget_checks=2,
                 source_hashes={name: hashlib.sha256((own / name).read_bytes()).hexdigest()
                                for name in ("effort_allocation.py", "effort_allocation_check.py")},
                 scope="Tiny synthetic optimizer verification only; no archive read, native run or classifier.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
    print(f"finite-prefix optimizer check passed: {len(checks)} exhaustive cases; {args.output}")


if __name__ == "__main__":
    main()
