# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from fractions import Fraction
from itertools import product
import unittest

from probe import Winner, reconstruct


class IdentifiabilityTests(unittest.TestCase):
    def check_reconstruction(self, table, bound):
        oracle = Winner(table, bound)
        recovered = reconstruct(lambda mask, cue: oracle(mask, cue), len(table), len(table[0]), bound)
        self.assertEqual(recovered, tuple(Fraction(value, table[0][0]) for row in table for value in row))
        return recovered

    def test_two_by_two_bridge_covers_both_signs_and_equality(self):
        for a, b in ((1, 3), (3, 1), (3, 3)):
            self.check_reconstruction(((a, b), (2, 4)), 4)
        for bound in (1, 2, 3):
            self.check_reconstruction(((bound, bound), (bound, bound)), bound)
        self.check_reconstruction(((1, 31), (30, 2)), 31)
        self.check_reconstruction(((31, 1), (2, 30)), 31)

    def test_connected_ratio_graph_reconstructs_rectangular_tables(self):
        self.check_reconstruction(((3, 1, 2), (1, 3, 1)), 3)
        self.check_reconstruction(((3, 1), (2, 1), (3, 2)), 3)
        self.check_reconstruction(((1, 2, 3), (3, 3, 1), (2, 1, 2)), 3)

    def test_scale_is_unobservable_but_common_row_offsets_are_exposed(self):
        table = ((1, 2, 3), (3, 1, 2))
        scaled = tuple(tuple(2 * value for value in row) for row in table)
        left, right = Winner(table, 3), Winner(scaled, 6)
        self.assertEqual(self.check_reconstruction(table, 3), self.check_reconstruction(scaled, 6))
        for bits in product((0, 1), repeat=6):
            mask = tuple(i for i, present in enumerate(bits) if present)
            for cue in product(range(3), repeat=2):
                if any(cue):
                    self.assertEqual(left(mask, cue), right(mask, cue))
        a, b = Winner(((8, 8), (4, 4)), 8), Winner(((1, 1), (4, 4)), 8)
        for i in range(11):
            cue = (Fraction(i, 10), Fraction(10 - i, 10))
            self.assertEqual(a(range(4), cue), b(range(4), cue))
        self.assertEqual(a((0, 3), (1, 1)), 0)
        self.assertEqual(b((0, 3), (1, 1)), 1)

    def test_one_row_counterexample_and_input_boundaries(self):
        left, right = Winner(((1, 2, 3),), 4), Winner(((1, 2, 4),), 4)
        for bits in product((0, 1), repeat=3):
            mask = tuple(i for i, present in enumerate(bits) if present)
            self.assertEqual(left(mask, (1,)), right(mask, (1,)))
        for rows in (1, 65):
            with self.assertRaises(ValueError):
                reconstruct(left, rows, 3, 4)
        for value in (0, -1, 6, True, 1.5):
            with self.assertRaises(ValueError):
                Winner(((1, value), (2, 3)), 5)
        oracle = Winner(((1, 2), (3, 4)), 5)
        for cell in (-1, 4, 5):
            with self.assertRaises(ValueError):
                oracle((cell,), (1, 1))
        for cue in ((0, 0), (1., 1), (-1, 2), (1,)):
            with self.assertRaises(ValueError):
                oracle((0, 3), cue)


if __name__ == '__main__':
    unittest.main()
