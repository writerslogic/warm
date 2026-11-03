import copy
import random
import unittest

from fiber import compile_model, expand
from state import Model, minimize


def check(test, states, actions, sources, records):
    packet = compile_model(states, actions, sources, records)
    expanded = expand(packet, actions, sources)
    combined = tuple((s, a, t, 1 << source, count) for s, a, t, source, count in records)
    combined += tuple((s + states, a, t + states, 1 << source, count) for s, a, t, source, count in expanded)
    model = Model(sources, actions, (0,) * (states + len(packet['instances'])), combined)
    partition = minimize(model, 'exhaustive')['partition']
    for state, mapped in enumerate(packet['stateMap']):
        test.assertEqual(partition[state], partition[states + mapped])
    return packet


class FiberTests(unittest.TestCase):
    def test_shared_control_keeps_distinct_source_bindings_and_terminal(self):
        packet = check(self, 5, 2, 2, [(0, 0, 1, 0, 2), (1, 1, 4, 0, 3),
                                     (2, 0, 3, 1, 5), (3, 1, 4, 1, 7)])
        self.assertEqual(packet['controlCount'], 3)
        self.assertEqual(len(packet['rules']), 2)
        self.assertEqual(packet['fallback'], [])
        self.assertNotEqual(packet['stateMap'][0], packet['stateMap'][2])

    def test_cross_source_successors_and_shared_state_propagate_fallback(self):
        packet = check(self, 5, 2, 3, [(0, 0, 1, 0, 1), (1, 1, 2, 1, 1),
                                     (2, 0, 4, 1, 1), (2, 1, 4, 2, 1), (3, 0, 4, 0, 1)])
        self.assertEqual(len(packet['fallback']), 4)
        self.assertEqual(len(packet['rules']), 1)

    def test_random_models_and_cycles_against_all_withdrawals(self):
        rng = random.Random(901134)
        for _ in range(100):
            records = [(rng.randrange(6), rng.randrange(3), rng.randrange(6), rng.randrange(3), rng.randrange(1, 8))
                       for _ in range(rng.randrange(25))]
            check(self, 6, 3, 3, records)
        check(self, 4, 1, 2, [(0, 0, 1, 0, 1), (1, 0, 0, 0, 1), (2, 0, 3, 1, 1), (3, 0, 2, 1, 1)])

    def test_count_boundaries_and_packet_rejection(self):
        for value in (2147483646, 2147483647):
            check(self, 3, 1, 1, [(0, 0, 1, 0, value), (0, 0, 1, 0, value), (0, 0, 2, 0, 1)])
        with self.assertRaises(ValueError):
            compile_model(3, 1, 1, [(0, 0, 1, 0, 2147483648)])
        packet = compile_model(3, 1, 2, [(0, 0, 2, 0, 1), (1, 0, 2, 1, 1)])
        for field, value in [('controlCount', -1), ('controlCount', 50001), ('controlCount', True),
                             ('stateMap', []), ('stateMap', [len(packet['instances'])]),
                             ('instances', [[0, 2]]), ('rules', [[0, 0, 0, 0]]),
                             ('fallback', [[0, 0, 0, 0, 1]])]:
            changed = copy.deepcopy(packet)
            changed[field] = value
            with self.assertRaises(ValueError):
                expand(changed, 1, 2)


if __name__ == '__main__':
    unittest.main()
