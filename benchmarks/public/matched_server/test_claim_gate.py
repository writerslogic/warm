# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import copy
from itertools import product
import json
from pathlib import Path
import unittest

from claim_gate import evaluate, quality


class ClaimGateTests(unittest.TestCase):
    def setUp(self):
        self.plan = json.loads((Path(__file__).parent / 'PLAN.json').read_text())
        self.rows = []
        for hardware, dataset, repetition, state, filter_name, clients, engine in product(
            self.plan['resources']['physicalHosts'], self.plan['datasets'], range(3), self.plan['states'],
            self.plan['filters'], (1, 8), ('warm', 'pgvector', 'opensearch')):
            warm = engine == 'warm'
            self.rows.append({'hardware': hardware, 'dataset': dataset, 'repetition': repetition,
                'state': state, 'filter': filter_name, 'clients': clients, 'engine': engine,
                'complete': True, 'evidenceVerified': True, 'resourcesPassed': True, 'isolationPassed': True,
                'developmentOnly': False, 'resourceAccountingAvailable': True,
                'queries': 1000, 'completed': 1000, 'errors': 0, 'unresolvedExactMembership': 0,
                'meanRecall': .99, 'worstRecall': .9, 'median': .8 if warm else 1.,
                'p95': 1.6 if warm else 2., 'p99': 3., 'queriesPerSecond': 200. if warm else 100.})

    def test_every_hardware_workload_repetition_is_required(self):
        result = evaluate(self.rows, self.plan)
        self.assertTrue(result['complete'])
        self.assertTrue(result['latencyPassed'])
        self.assertTrue(result['advantagePassed'])
        self.assertEqual(result['advantageTargetCells'], 12)
        for rows in (self.rows[:-1], self.rows + [self.rows[0]]):
            result = evaluate(rows, self.plan)
            self.assertFalse(result['complete'])
            self.assertFalse(result['latencyPassed'])
            self.assertFalse(result['advantagePassed'])

    def test_one_bad_tail_or_filter_rejects_latency_without_pooling(self):
        rows = copy.deepcopy(self.rows)
        row = next(row for row in rows if row['engine'] == 'warm' and row['filter'] == '1pct' and row['repetition'] == 2)
        row['p99'] = 3.151
        result = evaluate(rows, self.plan)
        self.assertFalse(result['latencyPassed'])
        self.assertTrue(result['advantagePassed'])
        row['p99'] = 3.15
        self.assertTrue(evaluate(rows, self.plan)['latencyPassed'])

    def test_strongest_comparator_and_twofold_boundary(self):
        rows = copy.deepcopy(self.rows)
        warm = next(row for row in rows if row['engine'] == 'warm' and row['state'] == 'churn-after-maintenance'
                    and row['filter'] == 'all' and row['clients'] == 8)
        for throughput in (199.999, 200., 200.001):
            warm['queriesPerSecond'] = throughput
            self.assertEqual(evaluate(rows, self.plan)['advantagePassed'], throughput >= 200.)
        warm['queriesPerSecond'] = 210.
        competitor = next(row for row in rows if row['engine'] == 'opensearch' and
                          all(row[name] == warm[name] for name in ('hardware', 'dataset', 'repetition', 'state', 'filter', 'clients')))
        competitor['queriesPerSecond'] = 110.
        self.assertFalse(evaluate(rows, self.plan)['advantagePassed'])

    def test_quality_requires_raw_validation_resources_and_isolation(self):
        for field, value in [('evidenceVerified', False), ('resourcesPassed', False), ('isolationPassed', False),
                             ('developmentOnly', True), ('resourceAccountingAvailable', False), ('errors', 1),
                             ('unresolvedExactMembership', 1), ('queries', 999), ('completed', 999),
                             ('meanRecall', .98999), ('meanRecall', 1.00001), ('worstRecall', .89999),
                             ('worstRecall', 1.01), ('errors', False), ('p95', float('nan'))]:
            row = {**self.rows[0], field: value}
            self.assertFalse(quality(row, self.plan), field)
        rows = copy.deepcopy(self.rows)
        for row in rows:
            if row['engine'] != 'warm':
                row['evidenceVerified'] = False
        result = evaluate(rows, self.plan)
        self.assertFalse(result['latencyPassed'])
        self.assertFalse(result['advantagePassed'])


if __name__ == '__main__':
    unittest.main()
