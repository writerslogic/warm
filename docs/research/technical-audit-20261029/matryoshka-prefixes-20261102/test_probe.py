# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Independent scalar reference and small adversarial fixtures; no real corpus scoring."""

import math
import random
import unittest

import numpy as np

import probe


def scalar_cosine(row, query, dimension):
    x, y = list(map(float, row[:dimension])), list(map(float, query[:dimension]))
    nx = math.sqrt(math.fsum(v * v for v in x))
    ny = math.sqrt(math.fsum(v * v for v in y))
    return math.fsum((a / nx) * (b / ny) for a, b in zip(x, y))


class PrefixTests(unittest.TestCase):
    def test_dedup_keeps_first_identity_only_when_vector_bytes_match(self):
        rows = np.asarray([[1, 2], [3, 4], [1, 2]], dtype="<f2")
        self.assertEqual(probe.canonical_rows(["a", "b", "a"], rows), [0, 1])
        self.assertEqual(probe.canonical_rows(["a", "b", "c"], rows), [0, 1, 2])
        rows[2, 0] = 2
        with self.assertRaises(ValueError):
            probe.canonical_rows(["a", "b", "a"], rows)

    def test_fp16_rows_match_independent_scalar_reference_and_full_control(self):
        rng = random.Random(267811)
        rows = np.asarray([[rng.uniform(-1, 1) for _ in range(1024)] for _ in range(101)], dtype="<f2")
        query = np.asarray([rng.uniform(-1, 1) for _ in range(1024)], dtype="<f2")
        ids = [f"row-{100 - i:03}" for i in range(len(rows))]
        full_reference = [scalar_cosine(row, query, 1024) for row in rows]
        full = probe.cosine(rows, query, 1024)
        expected_full = sorted(range(len(ids)), key=lambda i: (-full_reference[i], ids[i]))[:10]
        for dimension in [2, 127, 128, 129, 255, 256, 257, 511, 512, 513, 1024]:
            reference = [scalar_cosine(row, query, dimension) for row in rows]
            scores = probe.cosine(rows, query, dimension)
            np.testing.assert_allclose(scores, reference, atol=2e-15, rtol=2e-15)
            expected_order = sorted(range(len(ids)), key=lambda i: (-reference[i], ids[i]))
            for limit in [19, 20, 21, 49, 50, 51, 99, 100, 101, 102]:
                with self.subTest(dimension=dimension, limit=limit):
                    shortlist, final = probe.retrieve(scores, full, ids, limit, 10)
                    self.assertEqual(shortlist, expected_order[:limit])
                    expected = sorted(expected_order[:limit], key=lambda i: (-full_reference[i], ids[i]))[:10]
                    self.assertEqual(final, expected)
                    if dimension == 1024:
                        self.assertEqual(final, expected_full)

    def test_prefix_renormalization_and_deterministic_ties(self):
        rows = np.asarray([[1, 0, 100], [2, 0, 0], [0, 1, 0]], dtype="<f2")
        query = np.asarray([1, 0, 0], dtype="<f2")
        scores = probe.cosine(rows, query, 2)
        np.testing.assert_array_equal(scores, [1, 1, 0])
        self.assertEqual(probe.order(scores, ["z", "a", "b"]), [1, 0, 2])
        self.assertLess(probe.cosine(rows, query, 3)[0], 0.011)

    def test_shortlist_can_miss_full_winner_and_reranking_cannot_restore_it(self):
        ids = ["prefix-winner", "full-winner", "third"]
        prefix = [1.0, 0.2, 0.3]
        full = [0.1, 1.0, 0.5]
        self.assertEqual(probe.retrieve(prefix, full, ids, 2, 1), ([0, 2], [2]))
        self.assertEqual(probe.retrieve(prefix, full, ids, 3, 1), ([0, 2, 1], [1]))
        for n in [9, 10, 11]:
            scores = list(range(n))
            row_ids = [str(i) for i in range(n)]
            for limit in [19, 20, 21]:
                shortlist, final = probe.retrieve(scores, scores, row_ids, limit, 10)
                self.assertEqual(len(shortlist), n)
                self.assertEqual(len(final), min(n, 10))

    def test_gold_session_coverage_deduplicates_returned_turns(self):
        sessions = {"a": "one", "b": "one", "c": "two", "d": "three"}
        metrics = probe.gold_metrics(["a", "b", "d"], sessions, ["a", "c"], ["one", "two"])
        self.assertEqual(metrics, {"turn_any": 1, "turn_all": 0, "turn_fraction": 0.5, "session_any": 1, "session_all": 0, "session_fraction": 0.5})
        self.assertEqual(probe.gold_metrics(["a", "c"], sessions, ["a", "c"], ["one", "two"])["session_all"], 1)
        assistant_only = probe.gold_metrics(["a"], sessions, [], ["one"])
        self.assertIsNone(assistant_only["turn_any"])
        self.assertEqual(assistant_only["session_any"], 1)
        self.assertEqual(probe.mean_metrics([metrics, assistant_only])["turn_fraction"], 0.5)
        with self.assertRaises(ValueError):
            probe.gold_metrics([], {}, [], [])

    def test_malformed_vectors_and_invalid_limits_fail(self):
        for dimension in [0, 4]:
            with self.assertRaises(ValueError):
                probe.cosine([[1, 2, 3]], [1, 2, 3], dimension)
        for rows, query in [([[0, 0]], [1, 0]), ([[1, 0]], [0, 0]), ([[1, math.nan]], [1, 1]), ([[1, 1]], [1, math.inf]), ([[1, 1]], [1])]:
            with self.assertRaises(ValueError):
                probe.cosine(rows, query, 2)
        for candidates, k in [(9, 10), (0, 0), (-1, 1)]:
            with self.assertRaises(ValueError):
                probe.retrieve([1], [1], ["a"], candidates, k)
        with self.assertRaises(ValueError):
            probe.order([1, 2], ["same", "same"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
