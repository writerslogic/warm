# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from fractions import Fraction
import unittest

from probe import Basis, Compiler, contrasts, masks


class CompilerTests(unittest.TestCase):
    def test_row_deletion_keeps_the_gauge_cell_deletion_reveals_it(self):
        left, right = (8, 8, 4, 0), (1, 1, 4, 0)
        row = Compiler(contrasts(2, 2), masks(2, 2, 'row'))
        cell = Compiler(contrasts(2, 2), masks(2, 2, 'cell'))
        self.assertEqual(row.encode(left), row.encode(right))
        self.assertNotEqual(cell.encode(left), cell.encode(right))
        self.assertEqual(len(row.basis.rows), 2)
        self.assertEqual(len(cell.basis.rows), 4)
        self.assertEqual(row.update(row.encode(left), 0), row.encode((0, 0, 4, 0)))
        self.assertEqual(cell.update(cell.encode(left), 0), cell.encode((0, 8, 4, 0)))

    def test_fractional_basis_and_inaccessible_observation(self):
        basis = Basis(3)
        self.assertTrue(basis.insert([Fraction(1, 2), 1, 0]))
        self.assertFalse(basis.insert([1, 2, 0]))
        self.assertEqual(basis.coordinates([1, 2, 0]), (1,))
        with self.assertRaises(ValueError):
            basis.coordinates([0, 0, 1])
        basis.insert([0, 0, 2])
        self.assertEqual(basis.coordinates([3, 6, 5]), (3, 5))

    def test_dimension_and_mask_boundaries(self):
        for width in [0, 1, 255, 256, 257]:
            if 1 <= width <= 256:
                Basis(width)
            else:
                with self.assertRaises(ValueError):
                    Basis(width)
        for mask in [[], [1], [1, 1, 1], [1, -1], [1, 2], [1, True]]:
            with self.assertRaises(ValueError):
                Compiler([(1, -1)], [mask])
        with self.assertRaises(ValueError):
            masks(2, 2, 'unspecified')


if __name__ == '__main__':
    unittest.main()
