from itertools import combinations
import random
import unittest

import probe
from ambient_witness import MAX_GROUP, MAX_SOURCES, audit, family, scalar_readout, sizes
from matrix_witness import record_readout
from witness_tightness import normalized


class AmbientWitnessTests(unittest.TestCase):
    def test_all_three_ambient_source_regimes(self):
        for group, sources, first in ((2, 2, 0), (2, 3, 1), (2, 5, 3), (2, 6, 4), (2, 7, 4), (2, 8, 4)):
            result = audit(group, sources)
            self.assertEqual(result['minimumWithdrawals'], first)
            self.assertIsNotNone(result['existingChecker'])

    def test_unknown_support_is_not_covered_by_distribution_bound(self):
        rows = tuple((outcome, 1 << source, 1) for source in range(5) for outcome in range(2))
        left, right = probe.Profile(5, 2, rows), probe.Profile(5, 2, rows[:-2])
        result = probe.equivalent(left, right)
        self.assertEqual(result['reason'], 'unknown-support')
        self.assertEqual(result['withdrawn'].bit_count(), 4)
        self.assertTrue(probe.equivalent(left, right, check_support=False)['equivalent'])

    def test_upper_bound_on_small_random_positive_common_support_profiles(self):
        randomizer = random.Random(117309)
        for group in (1, 2, 3):
            for sources in range(group, 8):
                masks = [sum(1 << bit for bit in bits) for bits in combinations(range(sources), group)]
                left, right = [tuple((outcome, mask, randomizer.randint(1, 9))
                                     for mask in masks for outcome in range(2)) for _ in range(2)]
                witnesses = [mask.bit_count() for mask in range(1 << sources)
                             if normalized(record_readout(left, mask, sources)) != normalized(record_readout(right, mask, sources))]
                if witnesses:
                    self.assertLessEqual(min(witnesses), min(2 * group, sources - group))

    def test_input_bounds_reject_invalid_types_and_allocations(self):
        for group in (MAX_GROUP - 1, MAX_GROUP):
            for sources in (group, MAX_SOURCES - 1, MAX_SOURCES):
                self.assertEqual(sum(sizes(group, sources)), sources)
                rows = family(group, sources)
                self.assertTrue(all(mask.bit_count() == group for row in rows for _, mask, _ in row))
                for mask in ((1 << sources) - 2, (1 << sources) - 1):
                    scalar_readout(group, sources, mask)
                with self.assertRaises(ValueError):
                    scalar_readout(group, sources, 1 << sources)
        for group, sources in ((0, 1), (MAX_GROUP + 1, MAX_SOURCES), (True, 3), (2, 1),
                               (1, MAX_SOURCES + 1), (1, True), (1, 1.5)):
            with self.assertRaises(ValueError):
                family(group, sources)
        with self.assertRaises(ValueError):
            scalar_readout(1, 1, True)


if __name__ == '__main__':
    unittest.main()
