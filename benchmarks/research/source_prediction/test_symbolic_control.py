from fractions import Fraction
import random
import unittest

from probe import Profile, equivalent, multiplied_profile
from symbolic_control import BudgetExceeded, Diagrams


class DiagramTests(unittest.TestCase):
    def test_exact_probabilities_and_identity_in_both_orders(self):
        rng = random.Random(302)
        for _ in range(50):
            left, right = [Profile(4, 3, tuple((rng.randrange(3), rng.randrange(15), rng.randint(1, 5))
                                             for _ in range(10))) for _ in range(2)]
            for descending in (False, True):
                diagrams = Diagrams(descending)
                first, second = diagrams.profile(left), diagrams.profile(right)
                self.assertEqual(first == second, equivalent(left, right)['equivalent'])
                for mask in range(16):
                    self.assertEqual(diagrams.read(first, mask), left.read(mask))
                    self.assertEqual(diagrams.read(second, mask), right.read(mask))

    def test_factor_cancellation_and_unknown(self):
        left, right = multiplied_profile(6, 4)
        for descending in (False, True):
            diagrams = Diagrams(descending)
            first, second = diagrams.profile(left), diagrams.profile(right)
            self.assertEqual(first, second)
            self.assertIsNone(diagrams.read(first, 63))
            self.assertEqual(diagrams.read(first, 62), (Fraction(1), Fraction(0), Fraction(0), Fraction(0)))
        empty = Profile(6, 4, ())
        self.assertNotEqual(first, diagrams.profile(empty))

    def test_node_budget_boundary(self):
        profile = Profile(1, 1, ((0, 1, 1),))
        for limit in (3, 4):
            diagrams = Diagrams(node_limit=limit)
            diagrams.profile(profile)
            self.assertEqual(diagrams.stats()['nodes'] + diagrams.stats()['terminals'], 3)
        with self.assertRaises(BudgetExceeded):
            Diagrams(node_limit=2).profile(profile)
        for limit in (0, 100001, True):
            with self.assertRaises(ValueError):
                Diagrams(node_limit=limit)
        diagrams = Diagrams(state_limit=1)
        with self.assertRaises(BudgetExceeded):
            diagrams.profile(Profile(2, 2, ((0, 1, 1), (1, 2, 2))))

    def test_constant_distribution_does_not_enumerate_weighted_subsets(self):
        profile = Profile(20, 2, tuple((0, 1 << source, 1 << source) for source in range(20)))
        for descending in (False, True):
            diagrams = Diagrams(descending, node_limit=100, state_limit=100)
            roots = diagrams.profile(profile)
            self.assertLessEqual(diagrams.stats()['nodes'], 40)
            self.assertEqual(diagrams.read(roots, 0), (Fraction(1), Fraction(0)))
            self.assertIsNone(diagrams.read(roots, (1 << 20) - 1))


if __name__ == '__main__':
    unittest.main()
