# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from dataclasses import replace
from itertools import combinations
import random
import unittest

from probe import compile_certificate, packet, verify
from run import random_case, row
from symbolic_control import Diagram, symbolic_certificate


class SymbolicControlTests(unittest.TestCase):
    def test_diagram_truth_tables_and_minimum_withdrawal_cost(self):
        rng = random.Random(8101)
        assignments = [frozenset(values) for size in range(5) for values in combinations(range(1, 5), size)]
        for _ in range(80):
            diagram = Diagram()
            choices = [(diagram.variable(source), [source in assignment for assignment in assignments]) for source in range(1, 5)]
            for _ in range(12):
                left, truth_left = rng.choice(choices)
                right, truth_right = rng.choice(choices)
                operator = rng.choice(('and', 'or', 'not'))
                if operator == 'not':
                    root, truth = diagram.negate(left), [not value for value in truth_left]
                else:
                    root = diagram.apply(operator, left, right)
                    truth = [(a and b) if operator == 'and' else (a or b) for a, b in zip(truth_left, truth_right)]
                self.assertEqual([diagram.evaluate(root, assignment) for assignment in assignments], truth)
                expected = min((len(assignment) for assignment, value in zip(assignments, truth) if value), default=float('inf'))
                self.assertEqual(diagram.minimum[root], expected)
                choices.append((root, truth))

    def test_symbolic_packets_match_exhaustive_temporal_read_union(self):
        spec = {'records': 24, 'slots': 8, 'sources': 6}
        for seed in range(8110, 8140):
            records, programs = random_case(seed, spec)
            snapshots = [(0, 0), (2, 3), (4, 2), (7, 8)]
            for budget in (0, 1, 2):
                selected, _ = symbolic_certificate(records, programs, snapshots, budget)
                _, exhaustive = verify(records, programs, snapshots, budget, selected)
                self.assertEqual([item.ordinal for item in selected], exhaustive)
                traced, _ = compile_certificate(records, programs, snapshots, budget)
                self.assertEqual(packet(selected, programs, snapshots, budget), packet(traced, programs, snapshots, budget))

    def test_shared_dependencies_change_reachable_branches(self):
        guard = ('or', ('eq', 0, 1), ('or', ('eq', 1, 1), ('eq', 2, 1)))
        programs = [('if', guard, ('eq', 3, 1), ('eq', 4, 1))]
        records = [row(i, i, 1, 0, 0, (i + 1,)) for i in range(5)]
        snapshots = [(0, 0)]
        selected, _ = symbolic_certificate(records, programs, snapshots, 2)
        self.assertNotIn(4, [item.ordinal for item in selected])
        records[:3] = [replace(item, sources=(*item.sources, 9)) for item in records[:3]]
        selected, _ = symbolic_certificate(records, programs, snapshots, 2)
        self.assertIn(4, [item.ordinal for item in selected])
        verify(records, programs, snapshots, 2, selected)

    def test_node_allocation_limit_is_enforced(self):
        for limit in (3, 4, 5):
            diagram = Diagram(max_nodes=limit)
            for source in range(1, limit - 1):
                diagram.variable(source)
            self.assertEqual(len(diagram.nodes), limit)
            self.assertEqual(diagram.variable(1), 2)
            with self.assertRaisesRegex(ValueError, 'node bound exceeded'):
                diagram.variable(limit)


if __name__ == '__main__':
    unittest.main()
