# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import copy
import unittest

import numpy as np

from adapters import payload, validate_hits
from oracle import summarize, truth
from calibrate import configurations, select_configuration


class ProtocolTests(unittest.TestCase):
    def test_selection_cannot_hide_a_failed_or_unmeasured_state(self):
        candidates = [{'route': 'fast'}, {'route': 'eligible'}]
        states = ['baseline', 'changed', 'maintained', 'reopen']
        cells = [{'configuration': config, 'state': state, 'pass': True, 'median': 1. if config['route'] == 'fast' else 2.}
                 for config in candidates for state in states]
        self.assertEqual(select_configuration(cells, candidates, states)['configuration'], candidates[0])
        cells[1]['pass'] = False
        self.assertEqual(select_configuration(cells, candidates, states)['configuration'], candidates[1])
        self.assertIsNone(select_configuration(cells[:-1], candidates, states))
        duplicate = cells[:-1] + [cells[-2]]
        self.assertIsNone(select_configuration(duplicate, candidates, states))

    def test_calibration_grid_respects_public_limits_and_filter_routes(self):
        import json
        from pathlib import Path
        plan = json.loads((Path(__file__).parent / 'PLAN.json').read_text())
        for engine in ['warm', 'pgvector', 'opensearch']:
            for build in plan['calibration'][engine]['builds']:
                for name in ['all', '10pct', '1pct', 'empty']:
                    configs = list(configurations(engine, build, name, plan))
                    self.assertEqual(len(configs), len({json.dumps(c, sort_keys=True) for c in configs}))
                    self.assertEqual(configs[0], {'route': 'exact'})
                    if engine == 'warm' and name != 'all':
                        self.assertEqual(len(configs), 1)
                    for config in configs:
                        if 'probes' in config:
                            self.assertLessEqual(config['probes'], build['lists'])
                        if engine == 'pgvector' and 'ef' in config:
                            self.assertLessEqual(config['ef'], 1000)

    def test_invalid_payload_membership_boundaries(self):
        hits = [{**payload(i), 'score': .5} for i in range(10)]
        self.assertEqual(validate_hits(hits, 1024, False, 'all'), set(range(10)))
        for index in [9, 10, 11]:
            invalid = copy.deepcopy(hits)
            invalid[-1] = {**payload(index), 'score': .5}
            if index == 9:
                validate_hits(invalid, 1024, False, '10pct')
            else:
                with self.assertRaises(ValueError):
                    validate_hits(invalid, 1024, False, '10pct')
        for invalid_id in ['-1', '1024', '1025', '00', '١', '1' * 13, 4]:
            invalid = copy.deepcopy(hits)
            invalid[-1]['id'] = invalid_id
            with self.assertRaises(ValueError):
                validate_hits(invalid, 1024, False, 'all')
        for count in [9, 11]:
            with self.assertRaises(ValueError):
                validate_hits((hits + [hits[0]])[:count], 1024, False, 'all')
        for key, value in [('text', ''), ('metadata', {}), ('score', float('nan')), ('id', '0')]:
            invalid = copy.deepcopy(hits)
            invalid[1][key] = value
            with self.assertRaises(ValueError):
                validate_hits(invalid, 1024, False, 'all')
        for ordinal in [99, 100, 199, 200]:
            changed = [{**payload(i, 1), 'score': .5} for i in range(9)]
            changed.append({**payload(ordinal, int(ordinal < 100)), 'score': .5})
            if 100 <= ordinal < 200:
                with self.assertRaises(ValueError):
                    validate_hits(changed, 1024, True, 'all')
            else:
                validate_hits(changed, 1024, True, 'all')

    def test_oracle_mutations_filters_and_scalar_agreement(self):
        rng = np.random.default_rng(701)
        train = rng.normal(size=(1024, 16)).astype('f4')
        queries = train[[0, 99, 100, 199, 200]].copy()
        for churn in [False, True]:
            result = truth(train, queries, churn)
            changed = train.astype(float)
            if churn:
                changed[:100] = changed[1:101].copy()
            for name in result:
                ids = [i for i in range(1024) if not (churn and 100 <= i < 200)
                       and (name == 'all' or name == '10pct' and i % 100 < 10 or name == '1pct' and i % 100 == 0)]
                for q, row in zip(queries, result[name]):
                    scores = {i: sum(float(a) * float(b) for a, b in zip(q, changed[i])) /
                              (sum(float(v) ** 2 for v in q) * sum(float(v) ** 2 for v in changed[i])) ** .5 for i in ids}
                    expected = sorted(ids, key=lambda i: (-scores[i], i))[:10]
                    self.assertEqual(row['ids'], expected)
        train[:] = 1
        self.assertEqual(truth(train, queries[:1])['all'][0]['ids'], list(range(10)))

    def test_quality_gate_does_not_waive_missing_errors_or_exact_membership(self):
        rows = [{'query': i, 'seconds': .1, 'hits': list(range(10)), 'expected': list(range(10))} for i in range(100)]
        self.assertTrue(summarize(rows, 100, True)['pass'])
        with self.assertRaises(ValueError):
            summarize(rows[:-1], 100, False)
        with self.assertRaises(ValueError):
            summarize(rows[:-1] + [rows[0]], 100, False)
        for missing in [0, 1, 2]:
            altered = copy.deepcopy(rows)
            altered[0]['hits'] = list(range(10 - missing))
            self.assertEqual(summarize(altered, 100, False)['pass'], missing <= 1)
            self.assertEqual(summarize(altered, 100, True)['pass'], missing == 0)
        altered = copy.deepcopy(rows)
        altered[0]['error'] = 'wrong payload'
        self.assertFalse(summarize(altered, 100, False)['pass'])


if __name__ == '__main__':
    unittest.main()
