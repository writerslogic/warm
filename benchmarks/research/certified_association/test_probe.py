# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import json
from pathlib import Path
import unittest

import numpy as np

from probe import Memory, exact, make_tensor, reconstruct


class AssociationTests(unittest.TestCase):
    def test_all_generated_controls_contract_without_a_full_tensor_copy(self):
        plan = json.loads(Path(__file__).with_name("PLAN.json").read_text())
        for scenario in plan["scenarios"]:
            tensor = make_tensor(plan, scenario, plan["seeds"][0])
            self.assertTrue(np.shares_memory(tensor, tensor.reshape(tensor.shape[0], -1)), scenario)

    def test_factorized_winner_matches_independent_scalar_contraction(self):
        tensor = np.einsum("i,j,k->ijk", np.arange(1, 9), np.arange(2, 10), [1., 2., 4., 8.])
        memory = Memory(tensor, rank=1, iterations=2)
        left = np.arange(1., 9.) / 36
        right = np.arange(8., 0., -1) / 36
        scalar = [sum(float(left[i] * right[j] * tensor[i, j, k]) for i in range(8) for j in range(8)) for k in range(4)]
        winner, certified, _ = memory.query(left, right)
        self.assertEqual(winner, int(np.argmax(scalar)))
        self.assertTrue(certified)
        np.testing.assert_allclose(exact(tensor, left, right), scalar, rtol=1e-14)

    def test_ties_and_tiny_gaps_require_exact_fallback(self):
        for gap in [0., 1e-14, 1e-8]:
            tensor = np.ones((4, 4, 3)) * np.array([1., 1. + gap, 0.])
            memory = Memory(tensor, rank=1, iterations=2)
            winner, certified, _ = memory.query(np.ones(4) / 4, np.ones(4) / 4)
            self.assertEqual(winner, 0 if gap == 0 else 1)
            self.assertEqual(certified, gap == 1e-8)

    def test_deleted_association_changes_rebuilt_winner_and_residual_is_bounded(self):
        tensor = np.ones((4, 4, 3)) * np.array([1., 2., 4.])
        memory = Memory(tensor, rank=1, iterations=2)
        tensor[:, :, 2] = 0
        rebuilt = Memory(tensor, rank=1, iterations=2)
        cue = np.ones(4) / 4
        self.assertEqual(memory.query(cue, cue)[0], 2)
        self.assertEqual(rebuilt.query(cue, cue)[0], 1)
        approximation = reconstruct(rebuilt.cores)
        approximation[tuple(rebuilt.positions)] += rebuilt.values
        self.assertLessEqual(float(np.max(np.abs(tensor - approximation))), rebuilt.residual_bound)
        self.assertTrue(np.all(rebuilt.tensor[:, :, 2] == 0))

    def test_invalid_shapes_values_and_cues_are_rejected(self):
        for shape in [(0, 2, 2), (257, 1, 1), (2, 2)]:
            with self.assertRaises(ValueError):
                Memory(np.zeros(shape))
        for value in [np.nan, np.inf, 1e6 + 1]:
            with self.assertRaises(ValueError):
                Memory(np.full((2, 2, 2), value))
        for size in [255, 256]:
            self.assertEqual(Memory(np.ones((size, 1, 1)), rank=1, iterations=1).tensor.shape[0], size)
        for value in [1e6 - 1, 1e6]:
            self.assertEqual(Memory(np.full((1, 1, 1), value), rank=1, iterations=1).tensor_max, value)
        memory = Memory(np.ones((2, 2, 2)), rank=1, iterations=1)
        for bad in [np.array([np.nan, 0]), np.array([-1., 2.]), np.array([1., 1.]), np.ones(3) / 3]:
            with self.assertRaises(ValueError):
                memory.query(bad, np.ones(2) / 2)


if __name__ == "__main__":
    unittest.main()
