# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from itertools import combinations, product
import random
import unittest

from refine import (compile_method, linear_proof, refine, separating_withdrawal,
                    verify_variants, verify_witnesses)
from source_lift.probe import Record, deletion_sets, reference_space, surviving


class RefinementTests(unittest.TestCase):
    def test_proof_coefficients_reconstruct_the_target(self):
        rng = random.Random(710)
        rows = [Record(i, (1 + i % 4,), tuple(rng.randrange(17) for _ in range(5))) for i in range(12)]
        for values in product(range(2), repeat=5):
            proof = linear_proof(rows, values, 17)
            self.assertIsNotNone(proof)
            actual = tuple(sum(rows[index].values[j] * weight for index, weight in proof.items()) % 17 for j in range(5))
            self.assertEqual(actual, values)
        self.assertIsNone(linear_proof([], (1,), 7))
        self.assertEqual(linear_proof([], (0,), 7), {})

    def test_branching_agrees_with_exhaustive_admissible_withdrawals(self):
        rng = random.Random(711)
        for _ in range(60):
            rows = [Record(i, tuple(sorted(rng.sample(range(1, 6), rng.choice((1, 2))))),
                           tuple(rng.randrange(7) for _ in range(3))) for i in range(8)]
            for target in rows:
                others = [row for row in rows if row.ordinal != target.ordinal]
                for budget in (0, 1, 2):
                    actual = separating_withdrawal(others, target, 7, budget, {'proofSearchStates': 0})
                    expected = []
                    for deleted in deletion_sets(set(range(1, 6)) - set(target.sources), budget):
                        vectors = [row.values for row in surviving(others, deleted, budget)]
                        if reference_space(vectors, 3, 7) != reference_space([*vectors, target.values], 3, 7):
                            expected.append(deleted)
                    self.assertEqual(actual is not None, bool(expected))
                    if actual is not None:
                        self.assertIn(actual, expected)

    def test_rare_information_and_contradiction_withdrawal(self):
        rows = [Record(0, (1,), (1, 0, 2)), Record(1, (2,), (1, 0, 2)),
                Record(2, (2, 3), (0, 1, 5)), Record(3, (4,), (0, 0, 1)),
                Record(4, (1, 3), (1, 1, 7))]
        methods = ('provenance', 'union', 'union-refine', 'direct-refine', 'lift-refine')
        variants = {name: compile_method(name, rows, 3, 11, 2, 2) for name in methods}
        self.assertEqual(len(list(verify_variants(rows, variants, 3, 11, 2))), 11)
        for deleted in deletion_sets(range(1, 5), 2):
            expected = None
            for data in [rows, *(pair[0] for pair in variants.values())]:
                active = surviving(data, deleted, 2)
                solutions = [point for point in product(range(11), repeat=2)
                             if all(sum(a * b for a, b in zip(row.values[:-1], point)) % 11 == row.values[-1]
                                    for row in active)]
                if expected is None:
                    expected = solutions
                self.assertEqual(solutions, expected)

    def test_tight_cases_and_witness_tampering(self):
        rows = [Record(i, sources, (1,)) for i, sources in enumerate(combinations(range(1, 5), 2))]
        retained, details = refine(rows, 1, 11, 2, 2)
        self.assertEqual(len(retained), 6)
        verify_witnesses(retained, details, 1, 11, 2)
        details['witnesses'][0] = []
        with self.assertRaises(AssertionError):
            verify_witnesses(retained, details, 1, 11, 2)

    def test_refinement_keeps_no_redundant_rows_and_rejects_invalid_inputs(self):
        rows = [Record(i, (1 + i % 4,), (1, i % 3)) for i in range(12)]
        retained, details = refine(rows, 2, 11, 1, 1)
        verify_witnesses(retained, details, 2, 11, 1)
        self.assertLess(len(retained), len(rows))
        for count in (0, 1):
            surviving(retained, frozenset(range(1, count + 1)), 1)
        with self.assertRaises(ValueError):
            surviving(retained, frozenset((1, 2)), 1)
        with self.assertRaises(ValueError):
            compile_method('unknown', rows, 2, 11, 1, 1)
        with self.assertRaises(ValueError):
            refine([rows[0], rows[0]], 2, 11, 1, 1)


if __name__ == '__main__':
    unittest.main()
