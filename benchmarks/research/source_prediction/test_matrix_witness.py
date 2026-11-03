from fractions import Fraction
import unittest

from matrix_witness import MAX_GROUP, audit, family, matrix_readout
from witness_tightness import normalized


class MatrixWitnessTests(unittest.TestCase):
    def test_one_group_matches_every_single_withdrawal_without_background(self):
        sources, profiles = family(1)
        self.assertEqual(sources, 3)
        self.assertEqual(profiles[0], ((0, 1, 4), (0, 2, 10), (0, 4, 16), (1, 1, 16), (1, 2, 10), (1, 4, 4)))
        self.assertEqual(profiles[1], ((0, 1, 1), (0, 2, 20), (0, 4, 19), (1, 1, 19), (1, 2, 20), (1, 4, 1)))
        for removed in (0, 1, 2, 4):
            left, right = matrix_readout(1, removed)
            self.assertEqual(normalized(left), normalized(right))
        self.assertEqual(tuple(map(normalized, matrix_readout(1, 6))),
                         ((Fraction(1, 5), Fraction(4, 5)), (Fraction(1, 20), Fraction(19, 20))))
        self.assertEqual(matrix_readout(1, 7), ((0, 0), (0, 0)))

    def test_composition_requires_two_withdrawals_in_every_group(self):
        result = audit(3)
        self.assertEqual(result['minimumWithdrawals'], 6)
        self.assertEqual(len(result['distinguishingMasks']), 8)
        self.assertEqual(result['unknownMasks'], 169)
        self.assertEqual(result['uniformDetectionProbability'], [1, 64])

    def test_width_and_withdrawal_validation_boundaries(self):
        for group in (MAX_GROUP - 1, MAX_GROUP):
            sources, _ = family(group)
            for mask in ((1 << sources) - 2, (1 << sources) - 1):
                matrix_readout(group, mask)
            with self.assertRaises(ValueError):
                matrix_readout(group, 1 << sources)
        for group in (0, MAX_GROUP + 1, True, 1.5):
            with self.assertRaises(ValueError):
                family(group)
        with self.assertRaises(ValueError):
            matrix_readout(1, True)


if __name__ == '__main__':
    unittest.main()
