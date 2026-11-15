# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import unittest

import context_shapes as cs


def tenant(sessions):
    """sessions: list of (epoch, [texts]); returns the index and the fact ids in ingestion order."""
    idx, ids = cs.TenantIndex(), []
    for epoch, texts in sessions:
        for t in texts:
            fid = f'f{len(ids)}'
            idx.add(fid, t, epoch)
            ids.append(fid)
    return idx, ids


def hits_of(ids, order):
    return [{'id': ids[p], 'text': f't{p}', 'score': 1.0 - i / 100} for i, p in enumerate(order)]


class ContextShapes(unittest.TestCase):
    def test_runs_never_cross_sessions_and_obey_caps(self):
        idx, ids = tenant([(1000, list('abcd')), (2000, list('efgh'))])
        hits = hits_of(ids, [3, 4, 0, 1, 6])  # 3 and 4 are adjacent in ingestion order but in different sessions
        runs = cs.select_runs(hits, idx, max_items=10, max_turns=10, gap=0)
        self.assertEqual(sorted(map(tuple, runs)), [(0, 1), (3,), (4,), (6,)])
        self.assertEqual(len(cs.select_runs(hits, idx, max_items=2, max_turns=10, gap=0)), 2)
        self.assertLessEqual(sum(map(len, cs.select_runs(hits, idx, max_items=10, max_turns=3, gap=0))), 3)

    def test_gap_fill_joins_only_inside_a_session(self):
        idx, ids = tenant([(1000, list('abcd')), (2000, list('efgh'))])
        self.assertEqual(cs.layout([0, 2], idx, 1), [[0, 1, 2]])
        self.assertEqual(cs.layout([0, 2], idx, 0), [[0], [2]])
        self.assertEqual(cs.layout([2, 4], idx, 1), [[2], [4]])  # the middle turn belongs to the other session

    def test_merged_items_keep_dialogue_order_and_every_selected_turn(self):
        idx, ids = tenant([(1000, list('abcde'))])
        items = cs.merged_items(hits_of(ids, [3, 1, 2]), idx, 200, 300, gap=1)
        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]['memory'], 'b\nc\nd')

    def test_ordered_items_sort_by_dialogue_inside_a_session_and_keep_the_date(self):
        idx, ids = tenant([(1_700_000_000, list('abcde'))])
        items = cs.ordered_items(hits_of(ids, [4, 0, 2]), idx, 200)
        by_time = [i['id'] for i in sorted(items, key=lambda i: i['created_at'])]
        self.assertEqual(by_time, [ids[0], ids[2], ids[4]])
        self.assertEqual({i['created_at'][:10] for i in items}, {'2023-11-14'})


if __name__ == '__main__':
    unittest.main()
