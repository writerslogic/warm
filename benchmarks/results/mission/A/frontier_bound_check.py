# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Independent finite checks of the Cauchy frontier rejection arithmetic."""

import argparse
import struct
from fractions import Fraction
from pathlib import Path

from frontier_bound_counter import (
    CURSOR_CAP, D, cauchy_decision, exact_frontier, f32_fraction, gamma_bound,
    inverse_key, norm_upper_bound, ordered_key, select_frontiers, sha256, write_json,
)


def independent_value(bits):
    value = struct.unpack(">f", struct.pack(">I", bits))[0]
    return Fraction(*value.as_integer_ratio())


def encode(value):
    return struct.unpack(">I", struct.pack(">f", value))[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    import numpy as np

    if D != 128 or CURSOR_CAP != 4096:
        raise AssertionError("registered dimension/cap changed")
    patterns = [0, 0x80000000, 1, 0x80000001, 0x007FFFFF, 0x807FFFFF,
                0x00800000, 0x80800000, 0x3F7FFFFF, 0x3F800000, 0x3F800001,
                0xBF800000, 0x7F7FFFFF, 0xFF7FFFFF]
    for bits in patterns:
        assert f32_fraction(bits) == independent_value(bits), f"decode {bits:08x}"
        assert inverse_key(ordered_key(bits)) == bits, f"round trip {bits:08x}"
    ordered = sorted(patterns, key=ordered_key)
    assert [independent_value(bits) for bits in ordered] == sorted(independent_value(bits) for bits in patterns)
    invalid_checks = 0
    for bits in (0x7F800000, 0xFF800000, 0x7FC00001, -1, 1 << 32):
        try:
            f32_fraction(bits)
        except ValueError:
            invalid_checks += 1
        else:
            raise AssertionError("invalid f32 accepted")

    values = list(range(-4097, 4098))
    permutation = values[::2] + values[1::2]
    one_column = np.array([encode(float(value)) for value in permutation], dtype=np.uint32)
    fixture = np.tile(one_column[:, None], (1, D))
    lower, upper = select_frontiers(fixture)
    ordered_values = sorted(Fraction(value) for value in permutation)
    assert all(independent_value(bits) == ordered_values[4096] == -1 for bits in lower)
    assert all(independent_value(bits) == ordered_values[len(values) - 4097] == 1 for bits in upper)
    assert ordered_values[4095:4098] == [Fraction(-2), Fraction(-1), Fraction(0)]
    assert ordered_values[len(values) - 4098:len(values) - 4095] == [Fraction(0), Fraction(1), Fraction(2)]
    signed_queries = [encode(1.0), encode(-1.0), 0, 0x80000000] * (D // 4)
    frontier, squared = exact_frontier(signed_queries, lower, upper)
    expected = sum((independent_value(q) * (independent_value(hi) if independent_value(q) >= 0 else independent_value(lo))
                    for q, lo, hi in zip(signed_queries, lower, upper)), Fraction())
    assert frontier == expected == 64 and squared == 64

    epsilon = Fraction(1, 1 << 149)
    decisions = [
        (Fraction(1) - epsilon, Fraction(1), Fraction(1), False),
        (Fraction(1), Fraction(1), Fraction(1), False),
        (Fraction(1) + epsilon, Fraction(1), Fraction(1), True),
        (Fraction(-2), Fraction(1), Fraction(1), False),
        (Fraction(0), Fraction(0), Fraction(0), False),
        (Fraction(2), Fraction(4), Fraction(1), False),
        (Fraction(2), Fraction(4) - epsilon, Fraction(1), True),
        (Fraction(2), Fraction(4) + epsilon, Fraction(1), False),
    ]
    for front, norm, maximum, expected in decisions:
        actual = cauchy_decision(front, norm, maximum)
        assert actual["rejected"] is expected
        margin = actual["squared_margin"]
        assert Fraction(int(margin["numerator"]), int(margin["denominator"])) == front * front - norm * maximum

    rows = [
        [0] * D,
        [1, 0x80000001] * (D // 2),
        [0x007FFFFF, 0x00800000] * (D // 2),
        [0x7F7FFFFF, 0xFF7FFFFF] * (D // 2),
        [encode(1.0)] + [encode(2.0 ** -27)] * (D - 1),
        (patterns * ((D + len(patterns) - 1) // len(patterns)))[:D],
    ]
    norm_receipts = []
    exact_norms = []
    for row in rows:
        exact = sum((independent_value(value) ** 2 for value in row), Fraction())
        upper_bound, receipt = norm_upper_bound(np.array([row], dtype=np.uint32))
        assert exact <= upper_bound
        expected_gamma = Fraction(127, (1 << 52) - 127)
        calculated = float.fromhex(receipt["maximum_computed_sum_hex"])
        assert upper_bound == Fraction(calculated) / (1 - expected_gamma)
        assert gamma_bound(calculated)[1] == expected_gamma
        exact_norms.append(exact)
        norm_receipts.append(receipt)
    combined, combined_receipt = norm_upper_bound(np.array(rows, dtype=np.uint32))
    assert max(exact_norms) <= combined
    for bad in (np.zeros((1, D - 1), dtype=np.uint32), np.full((1, D), 0x7F800000, dtype=np.uint32)):
        try:
            norm_upper_bound(bad)
        except ValueError:
            invalid_checks += 1
        else:
            raise AssertionError("unsupported norm input accepted")
    own = Path(__file__).resolve().parent
    result = dict(status="pass", decision="accept-frontier-integrity-checks", decoding_patterns=len(patterns), invalid_input_checks=invalid_checks,
        order_statistic_fixture_rows=len(values), fixed_rank_boundary_checks=6,
        signed_frontier_and_zero_checks=True, exact_decision_boundary_checks=len(decisions),
        norm_rows_checked=len(rows), norm_receipts=norm_receipts, combined_norm_receipt=combined_receipt,
        source_sha256={name: sha256(own / name) for name in ("frontier_bound_counter.py", "frontier_bound_check.py")},
        scope="Tiny synthetic arithmetic/order-statistic checker only; no benchmark inputs, labels, ANN search or performance measurement.")
    write_json(args.output, result)
    print(f"frontier bound check passed: {len(patterns)} decode,{len(decisions)} decision,{len(rows)} norm cases; {args.output}")


if __name__ == "__main__":
    main()
