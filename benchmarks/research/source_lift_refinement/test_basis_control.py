# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from itertools import product
import random
import unittest

from basis_control import proof_union
from refine import optimized_union, verify_variants
from source_lift.probe import Record


class BasisOracleTests(unittest.TestCase):
    def test_identical_union_under_single_and_joint_source_withdrawals(self):
        rng = random.Random(912)
        for fanin, budget, _ in product((1, 2), (0, 1, 2), range(20)):
            rows = [Record(i, tuple(sorted(rng.sample(range(1, 7), rng.randint(1, fanin)))),
                           tuple(rng.randrange(11) for _ in range(4))) for i in range(18)]
            expected = optimized_union(rows, 4, 11, budget)
            actual, details = proof_union(rows, 4, 11, budget, fanin)
            self.assertEqual(actual, expected)
            self.assertLessEqual(details['basisOracle']['basisBuilds'], details['basisOracle']['allPossibleWithdrawalSets'])

    def test_rank_loss_and_contradiction_are_not_pruned_as_irrelevant(self):
        rows = [Record(0, (1,), (1, 0, 2)), Record(1, (2,), (1, 0, 2)),
                Record(2, (2, 3), (0, 1, 5)), Record(3, (4,), (0, 0, 1)),
                Record(4, (1, 3), (1, 1, 7))]
        for minimize in (False, True):
            variant = proof_union(rows, 3, 11, 2, 2, minimize=minimize)
            states = list(verify_variants(rows, {'control': variant}, 3, 11, 2))
            self.assertEqual(len(states), 11)
            self.assertTrue(any(state['fullReferenceRequired'] for state in states))


if __name__ == '__main__':
    unittest.main()
