# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from itertools import combinations, product
from math import comb
import random
import unittest

from probe import (Record, checked_case, deletion_sets, exterior, lift_certificate,
                   packed, plain_basis, quotient_repairs, reference_space, surviving)


class SourceLiftTests(unittest.TestCase):
    def test_source_polynomial_annihilates_withdrawn_rows(self):
        for source in range(1, 5):
            code = exterior((source,), 2, 1, 11)
            for left, right in combinations(range(1, 5), 2):
                coefficients = (left * right, -left - right, 1)
                self.assertEqual(sum(a * b for a, b in zip(code, coefficients)) % 11,
                                 (source - left) * (source - right) % 11)

    def test_joint_provenance_preserves_actual_solutions_and_contradictions(self):
        rows = [Record(0, (1,), (1, 0, 2)), Record(1, (2, 3), (0, 1, 4)),
                Record(2, (1, 4), (1, 1, 6)), Record(3, (4,), (0, 0, 1)),
                Record(4, (2,), (1, 0, 2)), Record(5, (3,), (0, 1, 4))]
        for fanin in (2, 3):
            for budget in (0, 1, 2):
                result = checked_case(rows, 3, 11, budget, fanin)
                retained = [row for row in rows if row.ordinal in result['retained']['lift']]
                for deleted in deletion_sets(range(1, 5), budget):
                    solutions = []
                    for data in (rows, retained):
                        active = surviving(data, deleted, budget)
                        solutions.append([point for point in product(range(11), repeat=2)
                                          if all(sum(a * b for a, b in zip(row.values[:-1], point)) % 11 == row.values[-1]
                                                 for row in active)])
                    self.assertEqual(*solutions)

    def test_tight_bound_and_budget_refusal(self):
        for budget, fanin in ((0, 1), (1, 1), (2, 2), (4, 3)):
            rows = [Record(i, support, (1,)) for i, support in enumerate(combinations(range(1, budget + fanin + 1), fanin))]
            result = checked_case(rows, 1, 257, budget, fanin)
            self.assertEqual(len(result['retained']['lift']), comb(budget + fanin, fanin))
        rows = [Record(i, (i + 1,), (1,)) for i in range(3)]
        retained = lift_certificate(rows, 1, 7, 1, 1)
        self.assertEqual(len(retained), 2)
        for count in (0, 1):
            surviving(retained, frozenset(range(1, count + 1)), 1)
        with self.assertRaisesRegex(ValueError, 'budget'):
            surviving(retained, frozenset((1, 2)), 1)
        self.assertTrue(surviving(rows, frozenset((1, 2)), 2))

    def test_plain_basis_loses_conclusions_after_a_source_withdrawal(self):
        rows = [Record(0, (1,), (1, 0)), Record(1, (1,), (0, 1)),
                Record(2, (2,), (1, 0)), Record(3, (3,), (0, 1))]
        reduced = plain_basis(rows, 7)
        self.assertEqual(surviving(reduced, frozenset((1,)), 1), [])
        result = checked_case(rows, 2, 7, 1, 1)
        self.assertEqual(len(result['retained']['lift']), 4)

    def test_quotient_repair_control_and_merge_composition(self):
        rng = random.Random(43)
        rows = [Record(i, (1 + i % 5,), tuple(rng.randrange(17) for _ in range(4))) for i in range(40)]
        repair = quotient_repairs(rows, 17)
        self.assertLessEqual(len(repair), 2 * len(reference_space([row.values for row in rows], 4, 17)))
        checked_case(rows, 4, 17, 1, 1, variants={'lift': repair})
        for budget in (0, 1, 2):
            left = lift_certificate(rows[:20], 4, 17, budget, 1)
            right = lift_certificate(rows[20:], 4, 17, budget, 1)
            merged = lift_certificate(left + right, 4, 17, budget, 1)
            checked_case(rows, 4, 17, budget, 1, variants={'lift': merged})

    def test_input_and_allocation_boundaries(self):
        row = Record(0, (1,), (0, 6))
        packed([row], 2, 7, 1, 1)
        for source in (4, 5):
            lift_certificate([Record(0, (source,), (1,))], 1, 7, 1, 1)
        for source in (0, 6, 7, True, 1.5):
            with self.assertRaises(ValueError):
                lift_certificate([Record(0, (source,), (1,))], 1, 7, 1, 1)
        for value in (-1, 7, 8, True, 1.5):
            with self.assertRaises(ValueError):
                lift_certificate([Record(0, (1,), (value,))], 1, 7, 1, 1)
        for records in ([row, row], [Record(0, (1, 1), (1, 0))], [Record(0, (2, 1), (1, 0))]):
            with self.assertRaises(ValueError):
                lift_certificate(records, 2, 7, 1, 2)
        for prime in (1, 4, 9, 65522):
            with self.assertRaises(ValueError):
                lift_certificate([], 2, prime, 1, 1)
        for width in (0, 65):
            with self.assertRaises(ValueError):
                lift_certificate([], width, 7, 1, 1)
        with self.assertRaises(ValueError):
            lift_certificate([], 2, 7, 5, 1)
        for count in (4095, 4096):
            lift_certificate([Record(i, (1,), (0,)) for i in range(count)], 1, 7, 1, 1)
        with self.assertRaises(ValueError):
            lift_certificate([Record(i, (1,), (0,)) for i in range(4097)], 1, 7, 1, 1)


if __name__ == '__main__':
    unittest.main()
