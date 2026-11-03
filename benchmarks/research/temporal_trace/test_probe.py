# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from dataclasses import replace
import random
import unittest

from probe import Certificate, Index, Record, compile_certificate, execute, expression_slots, reference, validate, verify


def row(ordinal, slot=0, value=1, start=0, end=None, known=0, sources=(1,)):
    return Record(ordinal, slot, value, start, end, known, sources, f'Evidence {ordinal}.')


class TemporalTraceTests(unittest.TestCase):
    def test_false_guard_resurrection_and_contradiction(self):
        records = [row(0, sources=(1,)), row(1, value=0, start=1, known=1, sources=(2,)),
                   row(2, slot=1, sources=(3,))]
        programs, snapshots = [('eq', 0, 1), ('eq', 1, 1)], [(2, 2)]
        bad, _ = compile_certificate(records, programs, snapshots, 1, 'positive-only')
        self.assertEqual(execute(Index(bad), programs, snapshots[0], frozenset())[0][0], (False, True))
        self.assertEqual(reference(records, programs, snapshots[0], frozenset({2}))[0], (True, True))
        with self.assertRaisesRegex(AssertionError, 'counterfactual changed'):
            verify(records, programs, snapshots, 1, bad)
        selected, _ = compile_certificate(records, programs, snapshots, 1)
        self.assertEqual(verify(records, programs, snapshots, 1, selected), (4, [0, 1, 2]))

    def test_half_open_validity_known_time_and_tie_break(self):
        records = [row(0), row(1, value=2, start=1, end=3, known=2, sources=(2,)),
                   row(2, value=0, start=1, end=3, known=2, sources=(3,))]
        programs = [('eq', 0, value) for value in range(3)]
        snapshots = [(0, 3), (1, 1), (1, 2), (2, 3), (3, 3)]
        expected = [(False, True, False), (False, True, False), (True, False, False),
                    (True, False, False), (False, True, False)]
        for snapshot, answer in zip(snapshots, expected):
            self.assertEqual(reference(records, programs, snapshot, frozenset())[0], answer)
        selected, _ = compile_certificate(records, programs, snapshots, 2)
        verify(records, programs, snapshots, 2, selected)
        self.assertEqual(reference(records, programs, (2, 3), frozenset({3}))[0], (False, False, True))

    def test_random_joint_sources_nonlinear_programs_and_exact_trace_union(self):
        rng = random.Random(7101)
        for _ in range(80):
            records = [row(i, slot=rng.randrange(4), value=rng.randrange(3), start=rng.randrange(4),
                           known=rng.randrange(5), sources=tuple(sorted(rng.sample(range(1, 7), rng.randint(1, 2)))))
                       for i in range(16)]
            programs = [('if', ('eq', 0, 1), ('not', ('eq', 1, 2)),
                         ('or', ('eq', 2, 0), ('and', ('eq', 3, 1), ('eq', 1, 1))))]
            snapshots = [(0, 1), (2, 2), (4, 5)]
            for budget in (0, 1, 2):
                selected, _ = compile_certificate(records, programs, snapshots, budget)
                _, exhaustive = verify(records, programs, snapshots, budget, selected)
                self.assertEqual([item.ordinal for item in selected], exhaustive)
                heads, _ = compile_certificate(records, programs, snapshots, budget, 'heads')
                verify(records, programs, snapshots, budget, heads)
                self.assertTrue(set(exhaustive).issubset(item.ordinal for item in heads))

    def test_bounded_reader_refuses_scope_expansion_and_uses_only_certificate(self):
        programs, snapshots = [('eq', 0, 1)], [(0, 0)]
        records = [row(i, sources=(i + 1,)) for i in range(4)]
        selected, _ = compile_certificate(records, programs, snapshots, 2)
        reader = Certificate(selected, programs, snapshots, 2)
        for size in (1, 2):
            deleted = frozenset(range(1, size + 1))
            self.assertEqual(reader.evaluate((0, 0), deleted), reference(records, programs, (0, 0), deleted))
        for deleted in (frozenset({1, 2, 3}), frozenset({True}), {1}, frozenset({0})):
            with self.assertRaises(ValueError):
                reader.evaluate((0, 0), deleted)
        for snapshot in ((0, 1), (False, 0), [0, 0]):
            with self.assertRaises(ValueError):
                reader.evaluate(snapshot, frozenset())
        records.clear()
        self.assertTrue(reader.evaluate((0, 0), frozenset())[0][0])

    def test_input_boundaries_and_record_tampering(self):
        programs, snapshots = [('eq', 0, 1)], [(0, 0)]
        for count in (4095, 4096):
            validate([row(i) for i in range(count)], programs, snapshots, 2)
        with self.assertRaises(ValueError):
            validate([row(i) for i in range(4097)], programs, snapshots, 2)
        for count in (255, 256):
            validate([row(i, sources=(i + 1,)) for i in range(count)], programs, snapshots, 2)
        with self.assertRaises(ValueError):
            validate([row(i, sources=(i + 1,)) for i in range(257)], programs, snapshots, 2)
        leaf = ('eq', 0, 1)
        tree = leaf
        for _ in range(7):
            tree = ('and', tree, tree)
        for expression in (tree, ('not', tree)):
            self.assertEqual(expression_slots(expression), {0})
        with self.assertRaises(ValueError):
            expression_slots(('not', ('not', tree)))
        for bad in (replace(row(0), value=True), replace(row(0), valid_to=0),
                    replace(row(0), sources=(1, 1)), replace(row(0), sources=(True,))):
            with self.assertRaises(ValueError):
                validate([bad], programs, snapshots, 2)
        with self.assertRaises(ValueError):
            expression_slots(([], leaf))
        with self.assertRaisesRegex(AssertionError, 'changed an original'):
            verify([row(0)], programs, snapshots, 2, [replace(row(0), text='Changed')])


if __name__ == '__main__':
    unittest.main()
