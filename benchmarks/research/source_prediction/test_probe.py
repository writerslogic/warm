from fractions import Fraction
from itertools import product
import random
import unittest

import probe
from probe import Profile, antichain, equivalent, multiplied_profile, witness


class PredictionTests(unittest.TestCase):
    def test_shared_sources_and_unknown_are_preserved(self):
        left = Profile(4, 1, tuple((0, 1 << source, 1) for source in (0, 1, 2)))
        right = Profile(4, 1, tuple((0, 1 << source, 1) for source in (0, 1, 3)))
        for mask in range(16):
            if mask.bit_count() <= 2:
                self.assertEqual(left.read(mask), right.read(mask))
        self.assertTrue(equivalent(left, right, check_support=False)['equivalent'])
        result = equivalent(left, right)
        self.assertEqual(result['reason'], 'unknown-support')
        self.assertGreater(result['withdrawn'].bit_count(), 2)
        self.assertNotEqual(left.read(result['withdrawn']), right.read(result['withdrawn']))

    def test_boolean_factor_and_structural_control(self):
        left, right = multiplied_profile(5, 3)
        self.assertNotEqual(left.structural_key(), right.structural_key())
        self.assertTrue(equivalent(left, right)['equivalent'])
        for mask in range(32):
            self.assertEqual(left.read(mask), right.read(mask))
        self.assertIsNone(left.read(31))
        self.assertEqual(left.read(30), (Fraction(1), Fraction(0), Fraction(0)))

    def test_random_exact_equivalence_and_witness(self):
        rng = random.Random(301)
        for _ in range(100):
            profiles = [Profile(4, 3, tuple((rng.randrange(3), rng.randrange(15), rng.randrange(1, 5))
                                            for _ in range(12))) for _ in range(2)]
            left, right = profiles
            result = equivalent(left, right)
            self.assertEqual(result['equivalent'], all(left.read(mask) == right.read(mask) for mask in range(16)))
            if not result['equivalent']:
                self.assertNotEqual(left.read(result['withdrawn']), right.read(result['withdrawn']))
                if result['reason'] == 'distribution':
                    self.assertLessEqual(result['withdrawn'].bit_count(), 2 * probe.MAX_WIDTH)

    def test_witness_degree_bound_exhaustively(self):
        for coefficients in product((-1, 0, 1), repeat=4):
            polynomial = {mask: coefficient for mask, coefficient in enumerate(coefficients) if coefficient}
            if not polynomial:
                continue
            removed = witness(polynomial)
            self.assertNotEqual(sum(value for mask, value in polynomial.items() if not mask & removed), 0)
            self.assertLessEqual(removed.bit_count(), max(mask.bit_count() for mask in polynomial))
        self.assertEqual(antichain([3, 1, 6, 2, 0]), (0,))

    def test_input_and_work_boundaries(self):
        for sources in (probe.MAX_SOURCES - 1, probe.MAX_SOURCES):
            Profile(sources, 1, ((0, 1 << (sources - 1), 1),))
        for count in (probe.MAX_COUNT - 1, probe.MAX_COUNT):
            Profile(1, 1, ((0, 1, count),))
        for records in (probe.MAX_RECORDS - 1, probe.MAX_RECORDS):
            Profile(1, 1, ((0, 1, 1),) * records)
        for outcomes in (probe.MAX_OUTCOMES - 1, probe.MAX_OUTCOMES):
            Profile(1, outcomes, ((outcomes - 1, 1, 1),))
        invalid = [(33, 1, ()), (1, 33, ()), (True, 1, ()), (1, 1, ((0, True, 1),)),
                   (1, 1, ((0, 1, True),)), (1, 1, ((0, 1, probe.MAX_COUNT + 1),)),
                   (1, 1, ((0, 2, 1),)), (4, 1, ((0, 15, 1),)),
                   (1, 1, ((0, 1, 1),) * (probe.MAX_RECORDS + 1))]
        for args in invalid:
            with self.assertRaises(ValueError):
                Profile(*args)
        for used in (probe.MAX_PAIRS - 3, probe.MAX_PAIRS - 2):
            probe.cross_difference({1: 1}, {1: 1}, {1: 1}, {1: 1}, {'pairs': used})
        with self.assertRaises(ValueError):
            probe.cross_difference({1: 1}, {1: 1}, {1: 1}, {1: 1}, {'pairs': probe.MAX_PAIRS - 1})
        with self.assertRaises(ValueError):
            Profile(2, 1, ()).read(4)


if __name__ == '__main__':
    unittest.main()
