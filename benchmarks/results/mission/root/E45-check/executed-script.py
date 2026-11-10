# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check rounded observations and kth-bound transitions against exact score states."""

import argparse
import struct
import sys
from fractions import Fraction
from pathlib import Path

from frontier_threshold_counter import (
    D, E44_SOURCE_SHA, check_binary64, observed_kths, promote_signed, sequential_observations,
    require, sha256, threshold_decision, write_json,
)


def exact(bits):
    return Fraction(*struct.unpack(">f", struct.pack(">I", bits))[0].as_integer_ratio())


def encode(value):
    return struct.unpack(">I", struct.pack(">f", value))[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    import numpy as np

    require(not sys.flags.optimize, "checker assertions require non-optimized Python")
    check_binary64()
    patterns = [0, 0x80000000, 1, 0x80000001, 0x007FFFFF, 0x807FFFFF,
                0x00800000, 0x80800000, 0x3F800000, 0xBF800000, 0x7F7FFFFF, 0xFF7FFFFF]
    rows = [
        [0] * D,
        [0x7F7FFFFF, encode(1.0), 0xFF7FFFFF] + [0] * (D - 3),
        [0x7F7FFFFF, 0xFF7FFFFF] * (D // 2),
        [1, 0x80000001] * (D // 2),
        [0x00800000] * D,
        [0x007FFFFF, 0x80800000] * (D // 2),
        [encode(1.0)] + [encode(2.0 ** -27)] * (D - 1),
        (patterns * ((D + len(patterns) - 1) // len(patterns)))[:D],
    ]
    queries = [
        [encode(1.0)] * D,
        [0x7F7FFFFF] * D,
        [encode(1.0), encode(-1.0), 0, 0x80000000] * (D // 4),
        [1, 0x80000001] * (D // 2),
        [0] * D,
        [0x00800000, 0x807FFFFF] * (D // 2),
    ]
    base = promote_signed(np.array(rows, dtype=np.uint32))
    promoted_queries = promote_signed(np.array(queries, dtype=np.uint32))
    for source, promoted in ((rows, base), (queries, promoted_queries)):
        for source_row, promoted_row in zip(source, promoted):
            for bits, value in zip(source_row, promoted_row):
                assert Fraction(float(value)) == exact(bits)
                if bits == 0x80000000:
                    assert bool(np.signbit(value))
    observed = sequential_observations(base.T.copy(), promoted_queries)
    gamma = Fraction(127, (1 << 52) - 127)
    maximum = max(sum((exact(value) ** 2 for value in row), Fraction()) for row in rows)
    exact_scores = []
    enclosure_checks = 0
    unequal_observations = 0
    query_norms = []
    for query_index, query in enumerate(queries):
        norm = sum((exact(value) ** 2 for value in query), Fraction())
        query_norms.append(norm)
        truth = []
        for row_index, row in enumerate(rows):
            state = sum((exact(a) * exact(b) for a, b in zip(query, row)), Fraction())
            observation = Fraction(float(observed[query_index, row_index]))
            error = observation - state
            assert error * error <= gamma * gamma * norm * maximum, (query_index, row_index)
            unequal_observations += int(error != 0)
            enclosure_checks += 1
            truth.append(state)
        exact_scores.append(truth)
    assert unequal_observations > 0, "fixture did not distinguish exact state from rounded observation"
    assert exact_scores[0][1] == 1 and float(observed[0, 1]) == 0
    thresholds = observed_kths(observed.copy(), 3)
    kth_upper_checks = 0
    for ordinal, receipt in enumerate(thresholds):
        exact_kth = sorted(exact_scores[ordinal], reverse=True)[2]
        rounded_kth = Fraction(receipt["value"])
        difference = exact_kth - rounded_kth
        assert difference <= 0 or difference * difference <= gamma * gamma * query_norms[ordinal] * maximum
        kth_upper_checks += 1

    tie_rows = [[0] * D for _ in range(104)]
    tie_rows[0][0] = encode(1.0)
    tie_rows[1][0] = encode(1.0)
    tie_rows[2][0] = encode(-1.0)
    tie_queries = [[encode(1.0)] + [0] * (D - 1), [encode(-1.0)] + [0] * (D - 1)]
    tie_observations = sequential_observations(promote_signed(np.array(tie_rows, dtype=np.uint32)).T.copy(),
                                              promote_signed(np.array(tie_queries, dtype=np.uint32)))
    tie_receipts = observed_kths(tie_observations, 100)
    for query, receipt in zip(tie_queries, tie_receipts):
        states = [sum((exact(a) * exact(b) for a, b in zip(query, row)), Fraction()) for row in tie_rows]
        assert sorted(states, reverse=True)[99] == Fraction(receipt["value"])
    assert tie_receipts[0]["value"] == tie_receipts[1]["value"] == 0
    assert (tie_receipts[0]["scores_strictly_greater"], tie_receipts[0]["scores_at_least"]) == (2, 103)
    assert (tie_receipts[1]["scores_strictly_greater"], tie_receipts[1]["scores_at_least"]) == (1, 102)

    tiny = Fraction(1, 1 << 149)
    boundary_cases = [
        (Fraction(3) + Fraction(1, 4) - tiny, Fraction(3), Fraction(1), Fraction(1), Fraction(1, 4), False),
        (Fraction(3) + Fraction(1, 4), Fraction(3), Fraction(1), Fraction(1), Fraction(1, 4), False),
        (Fraction(3) + Fraction(1, 4) + tiny, Fraction(3), Fraction(1), Fraction(1), Fraction(1, 4), True),
        (Fraction(1), Fraction(3), Fraction(1), Fraction(1), Fraction(1, 4), False),
        (Fraction(3), Fraction(3), Fraction(1), Fraction(1), Fraction(1, 4), False),
        (Fraction(3) + tiny, Fraction(3), Fraction(1), Fraction(1), Fraction(0), True),
        (Fraction(3), Fraction(3), Fraction(1), Fraction(1), Fraction(0), False),
        (Fraction(0), Fraction(0), Fraction(0), Fraction(1), gamma, False),
    ]
    for frontier, observed_kth, norm, maximum_bound, error_factor, expected in boundary_cases:
        receipt = threshold_decision(frontier, observed_kth, norm, maximum_bound, error_factor)
        delta = frontier - observed_kth
        margin = receipt["exact_squared_comparison_margin"]
        assert receipt["rejected"] is expected
        assert Fraction(int(margin["numerator"]), int(margin["denominator"])) == delta * delta - error_factor * error_factor * norm * maximum_bound
    invalid_checks = 0
    for bad in (np.full((1, D), 0x7F800000, dtype=np.uint32), np.zeros((1, D - 1), dtype=np.uint32)):
        try:
            promote_signed(bad)
        except ValueError:
            invalid_checks += 1
        else:
            raise AssertionError("unsupported source accepted")
    for k in (0, 105):
        try:
            observed_kths(np.zeros((1, 104), dtype=np.float64), k)
        except ValueError:
            invalid_checks += 1
        else:
            raise AssertionError("invalid kth accepted")
    own = Path(__file__).resolve().parent
    assert sha256(own / "frontier_bound_counter.py") == E44_SOURCE_SHA
    result = dict(status="pass", decision="accept-threshold-integrity-checks",
        exact_state_vs_observation_enclosure_checks=enclosure_checks,
        nonidentical_rounded_observations=unequal_observations,
        signed_cancellation_witness=dict(exact_state="1", rounded_observation="0"),
        kth_monotonic_upper_checks=kth_upper_checks, k100_tie_checks=len(tie_receipts),
        delta_sign_equality_boundary_checks=len(boundary_cases), invalid_input_checks=invalid_checks,
        source_sha256={name: sha256(own / name) for name in ("frontier_threshold_counter.py", "frontier_threshold_check.py")},
        reused_E44_counter_sha256=E44_SOURCE_SHA,
        scope="Independent exact-Fraction tiny score states validate new signed sequential observations and kth upper transitions. No benchmark arrays, labels, E44 checker rerun or performance test.")
    write_json(args.output, result)
    print(f"threshold check passed: {enclosure_checks} enclosures,{kth_upper_checks} kth transitions,{len(boundary_cases)} boundaries; {args.output}")


if __name__ == "__main__":
    main()
