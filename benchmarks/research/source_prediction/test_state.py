from fractions import Fraction
from itertools import product
import random
import unittest

from state import Model, canonical, minimize, quotient


def partitions(length):
    def extend(prefix):
        if len(prefix) == length:
            yield tuple(prefix)
            return
        for block in range(max(prefix, default=-1) + 2):
            yield from extend([*prefix, block])
    yield from extend([])


def raw_distribution(model, state, action, removed):
    counts = [0] * len(model.labels)
    for origin, step, target, dependencies, count in model.records:
        if origin == state and step == action and not dependencies & removed:
            counts[target] += count
    total = sum(counts)
    return tuple(Fraction(count, total) for count in counts) if total else None


def brute_valid(model, partition):
    for left in range(len(model.labels)):
        for right in range(left):
            if partition[left] != partition[right]:
                continue
            if model.labels[left] != model.labels[right]:
                return False
            for removed in range(1 << model.sources):
                for action in range(model.actions):
                    first = raw_distribution(model, left, action, removed)
                    second = raw_distribution(model, right, action, removed)
                    if (first is None) != (second is None):
                        return False
                    if first is not None:
                        for block in set(partition):
                            if sum(value for index, value in enumerate(first) if partition[index] == block) != sum(
                                    value for index, value in enumerate(second) if partition[index] == block):
                                return False
    return True


def trace_distribution(model, origin, actions, removed):
    paths = {((model.labels[origin],), origin): Fraction(1)}
    for action in actions:
        updated = {}
        for (trace, state), weight in paths.items():
            row = None if state is None else raw_distribution(model, state, action, removed)
            if row is None:
                key = (*trace, None), None
                updated[key] = updated.get(key, 0) + weight
            else:
                for target, probability in enumerate(row):
                    if probability:
                        key = (*trace, model.labels[target]), target
                        updated[key] = updated.get(key, 0) + weight * probability
        paths = updated
    result = {}
    for (trace, _), weight in paths.items():
        result[trace] = result.get(trace, 0) + weight
    return result


def correlated_model():
    records = []
    for state in range(4):
        for action in range(2):
            for source in range(3):
                target = 2 * ((source + action) % 2) + state % 2
                base = (state, action, target, 1 << source, 1 << source)
                if state % 2:
                    records.extend((*base[:3], base[3] | (1 << other), base[4]) for other in range(3))
                else:
                    records.append(base)
    return Model(3, 2, (0, 0, 1, 1), tuple(records))


class StateTests(unittest.TestCase):
    def test_quotient_preserves_multistep_distributions_after_every_withdrawal(self):
        model = correlated_model()
        candidate = minimize(model)['partition']
        self.assertEqual(candidate, (0, 0, 1, 1))
        for method in ('diagram', 'exhaustive'):
            self.assertEqual(minimize(model, method)['partition'], candidate)
        self.assertNotEqual(minimize(model, 'structural')['partition'], candidate)
        reduced = quotient(model, candidate)
        for removed in range(8):
            for actions in product(range(2), repeat=3):
                for origin in range(4):
                    self.assertEqual(trace_distribution(model, origin, actions, removed),
                                     trace_distribution(reduced, candidate[origin], actions, removed))

    def test_independent_partition_enumeration(self):
        rng = random.Random(303)
        models = [correlated_model()]
        for _ in range(30):
            models.append(Model(3, 2, tuple(rng.randrange(2) for _ in range(4)),
                                tuple((rng.randrange(4), rng.randrange(2), rng.randrange(4), rng.randrange(8), rng.randint(1, 3))
                                      for _ in range(12))))
        for model in models:
            valid = [partition for partition in partitions(len(model.labels)) if brute_valid(model, partition)]
            minimum = min(len(set(partition)) for partition in valid)
            optimal = [partition for partition in valid if len(set(partition)) == minimum]
            self.assertEqual(len(optimal), 1)
            self.assertEqual(minimize(model)['partition'], optimal[0])
            self.assertEqual(minimize(model, 'diagram')['partition'], optimal[0])

    def test_current_only_merging_breaks_after_withdrawal(self):
        model = Model(2, 1, (0, 0, 1, 2),
                      ((0, 0, 2, 1, 1), (0, 0, 3, 2, 1), (1, 0, 3, 1, 1), (1, 0, 2, 2, 1)))
        self.assertEqual(minimize(model, 'current')['partition'], (0, 0, 1, 2))
        self.assertEqual(minimize(model)['partition'], (0, 1, 2, 3))
        with self.assertRaises(ValueError):
            quotient(model, (0, 0, 1, 2))

    def test_sequential_withdrawal_and_recompaction_matches_fresh_model(self):
        for model in (correlated_model(), Model(2, 1, (0, 0, 1),
                      ((0, 0, 2, 1, 1), (0, 0, 0, 2, 1), (1, 0, 1, 1, 1), (1, 0, 1, 2, 1)))):
            mapping = minimize(model)['partition']
            reduced = quotient(model, mapping)
            original = model
            for source in range(model.sources):
                original = original.withdraw(1 << source)
                reduced = reduced.withdraw(1 << source)
                next_partition = minimize(reduced)['partition']
                mapping = canonical(tuple(next_partition[block] for block in mapping))
                reduced = quotient(reduced, next_partition)
                self.assertEqual(mapping, minimize(original)['partition'])
                for state in range(len(model.labels)):
                    self.assertEqual(trace_distribution(original, state, (0, 0, 0), 0),
                                     trace_distribution(reduced, mapping[state], (0, 0, 0), 0))

    def test_model_boundaries_and_invalid_partitions(self):
        for states in (31, 32):
            Model(1, 1, (0,) * states, ())
        for actions in (7, 8):
            Model(1, actions, (0,), ())
        for args in ((1, 9, (0,), ()), (1, 1, (0,) * 33, ()),
                     (1, 1, (True,), ()), (1, 1, (0,), ((0, 1, 0, 0, 1),)),
                     (1, 1, (0,), ((0, 0, 1, 0, 1),))):
            with self.assertRaises(ValueError):
                Model(*args)
        model = correlated_model()
        for invalid in ((0, 0), (0, 0, 2, 2), (0, False, 1, 1)):
            with self.assertRaises(ValueError):
                model.profiles(invalid)
        with self.assertRaises(ValueError):
            quotient(model, (0, 0, 0, 0))
        with self.assertRaises(ValueError):
            model.withdraw(8)


if __name__ == '__main__':
    unittest.main()
