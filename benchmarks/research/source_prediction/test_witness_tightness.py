from fractions import Fraction
import unittest

from witness_tightness import MAX_GROUP, audit, count, family, normalized


class WitnessTightnessTests(unittest.TestCase):
    def test_positive_width_one_can_hide_until_two_withdrawals(self):
        sources, (left, right) = family(1)
        for removed in [0, 1, 2]:
            self.assertEqual(normalized(count(left, removed, sources)), normalized(count(right, removed, sources)))
        self.assertEqual(normalized(count(left, 3, sources)), (Fraction(2, 3), Fraction(1, 3)))
        self.assertEqual(normalized(count(right, 3, sources)), (Fraction(5, 9), Fraction(4, 9)))
        self.assertTrue(all(value > 0 for profile in (left, right) for value in count(profile, 3, sources)))

    def test_anchor_withdrawal_is_joint_unknown_and_other_withdrawals_remain_exact(self):
        result = audit(2, True)
        self.assertEqual(result['minimumWithdrawals'], 4)
        self.assertEqual(result['unknownMasks'], 16)
        self.assertEqual(result['maximumSupportWidth'], 3)

    def test_declared_width_and_withdrawal_boundaries(self):
        for group in [MAX_GROUP - 1, MAX_GROUP]:
            sources, profiles = family(group)
            for mask in [(1 << sources) - 2, (1 << sources) - 1]:
                count(profiles[0], mask, sources)
            with self.assertRaises(ValueError):
                count(profiles[0], 1 << sources, sources)
        for group in [0, MAX_GROUP + 1, True, 1.5]:
            with self.assertRaises(ValueError):
                family(group)
        with self.assertRaises(ValueError):
            family(1, 1)
        with self.assertRaises(ValueError):
            count((), True, 1)


if __name__ == '__main__':
    unittest.main()
