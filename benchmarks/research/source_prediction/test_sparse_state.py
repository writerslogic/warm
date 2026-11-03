import random
import unittest

from state import Model, minimize as small_minimize
from sparse_state import canonical, minimize, quotient, validate, withdraw


class SparseTests(unittest.TestCase):
    def test_random_models_against_exhaustive_withdrawal_refinement(self):
        rng = random.Random(70231)
        for _ in range(100):
            states, actions, sources = 5, 3, 4
            records = [tuple([rng.randrange(states), rng.randrange(actions), rng.randrange(states), rng.randrange(sources), rng.randrange(1, 6)]) for _ in range(rng.randrange(1, 30))]
            model = Model(sources, actions, (0,) * states, tuple((state, action, target, 1 << source, count) for state, action, target, source, count in records))
            result = minimize(states, actions, sources, records)['partition']
            self.assertEqual(result, small_minimize(model, 'exhaustive')['partition'])
            compact = quotient(states, actions, sources, records, result)
            for mask in range(1 << sources):
                removed = {source for source in range(sources) if mask & (1 << source)}
                fresh = minimize(states, actions, sources, withdraw(records, removed))['partition']
                repaired = minimize(max(result) + 1, actions, sources, withdraw(compact, removed))['partition']
                self.assertEqual(fresh, canonical(repaired[block] for block in result))

    def test_source_weights_and_unknown_are_not_dropped(self):
        records = [(0, 0, 2, 0, 1), (1, 0, 2, 1, 1)]
        self.assertNotEqual(minimize(3, 1, 2, records)['partition'], minimize(3, 1, 2, records, current=True)['partition'])
        with self.assertRaises(ValueError):
            quotient(3, 1, 2, records, (0, 0, 1))
        self.assertEqual(minimize(3, 1, 2, [])['partition'], (0, 0, 0))

    def test_record_bounds(self):
        validate(3, 2, 128, [(2, 1, 2, 126, 2147483646), (2, 1, 2, 127, 2147483647)])
        for row in [(3, 1, 2, 127, 1), (2, 2, 2, 127, 1), (2, 1, 2, 128, 1), (2, 1, 2, 127, 2147483648), (2, 1, 2, 127, 0), (True, 1, 2, 127, 1)]:
            with self.assertRaises(ValueError):
                validate(3, 2, 128, [row])


if __name__ == '__main__':
    unittest.main()
