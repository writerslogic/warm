from itertools import product
import unittest

from probe import Profile
from singleton_control import equivalent, signature, sufficient_signature


class SingletonTests(unittest.TestCase):
    def test_all_singleton_profiles_against_every_withdrawal(self):
        slots = [(outcome, 1 << source) for outcome in range(2) for source in range(3)]
        profiles = [Profile(3, 2, tuple((*slot, count) for slot, count in zip(slots, values, strict=True) if count))
                    for values in product((0, 1), repeat=6)]
        readouts = [tuple(profile.read(mask) for mask in range(8)) for profile in profiles]
        for first, left in enumerate(profiles):
            for second, right in enumerate(profiles):
                self.assertEqual(equivalent(left, right), readouts[first] == readouts[second])

    def test_constant_source_mixtures_and_different_mixing_weights(self):
        first = Profile(3, 2, ((0, 1, 1), (1, 1, 2), (0, 2, 7), (1, 2, 14)))
        same = Profile(3, 2, ((0, 1, 5), (1, 1, 10), (0, 2, 3), (1, 2, 6)))
        self.assertTrue(equivalent(first, same))
        self.assertNotEqual(first.structural_key(), same.structural_key())
        mixed = Profile(3, 2, ((0, 1, 1), (1, 2, 1)))
        changed = Profile(3, 2, ((0, 1, 2), (1, 2, 1)))
        self.assertFalse(equivalent(mixed, changed))
        with self.assertRaises(ValueError):
            signature(Profile(3, 2, ((0, 3, 1),)))
        with self.assertRaises(ValueError):
            signature(Profile(3, 2, ((0, 0, 1),)))

    def test_joint_constant_profiles_use_support_instead_of_raw_counts(self):
        left = Profile(3, 2, ((0, 1, 1), (1, 1, 2)))
        right = Profile(3, 2, ((0, 1, 7), (1, 1, 14), (0, 3, 2), (1, 3, 4)))
        self.assertEqual(sufficient_signature(left), sufficient_signature(right))
        for mask in range(8):
            self.assertEqual(left.read(mask), right.read(mask))
        different = Profile(3, 2, ((0, 3, 2), (1, 3, 4)))
        self.assertNotEqual(sufficient_signature(left), sufficient_signature(different))


if __name__ == '__main__':
    unittest.main()
