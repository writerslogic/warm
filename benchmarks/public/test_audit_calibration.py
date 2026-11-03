# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import copy
import json
from pathlib import Path
import tempfile
import unittest

import numpy as np

from audit_calibration import audit_cell, audit_reference, legacy_digest, raw_cell_present
from adapters import payload
from oracle import summarize, truth


class CalibrationAuditTests(unittest.TestCase):
    def test_missing_raw_requires_explicit_rejection_and_never_qualifies_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'cell.ndjson'
            rejected = {'pass': False, 'error': {'type': 'ValueError', 'message': 'incorrect hit count'}}
            self.assertFalse(raw_cell_present(path, rejected))
            for summary in ({'pass': True}, {**rejected, 'pass': True}, {'pass': False},
                            {**rejected, 'error': {'type': 'ValueError', 'message': ''}},
                            {**rejected, 'error': {'type': 'ValueError', 'message': 3}}):
                with self.assertRaises(ValueError):
                    raw_cell_present(path, summary)
            path.write_text('')
            self.assertTrue(raw_cell_present(path, {'pass': True}))

    def test_actual_legacy_orders_scores_churn_and_empty_results(self):
        rng = np.random.default_rng(919)
        train = rng.normal(size=(1024, 6)).astype('f4')
        queries = train[[0, 201, 400]].copy()
        order = ('epoch', 'eligible1', 'ordinal', 'eligible10', 'bucket100')
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'cell.ndjson'
            for engine in ('warm', 'pgvector', 'opensearch'):
                for churn in (False, True):
                    expected = truth(train, queries, churn)
                    for name in ('all', '10pct', '1pct', 'empty'):
                        rows = []
                        for index, reference in enumerate(expected[name]):
                            hits = [{**payload(identifier, int(churn and identifier < 100)),
                                     'score': (1 + score) / 2 if engine == 'opensearch' else score}
                                    for identifier, score in zip(reference['ids'], reference['scores'])]
                            rows.append({'query': index, 'seconds': .001 + .0001 * index, 'error': None,
                                         'expected': reference['ids'], 'hits': sorted(reference['ids']),
                                         'returned': [{'id': hit['id'], 'score': hit['score']} for hit in hits],
                                         'payloadSha256': legacy_digest(hits, order)})
                        path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
                        summary = {'filter': name, 'state': 'reopen' if churn else 'baseline',
                                   'configuration': {'route': 'exact'}, **summarize(rows, 3, True)}
                        result = audit_cell(path, summary, engine=engine, train=train, queries=queries, expected=expected[name])
                        self.assertTrue(result['recomputed']['pass'])
                        self.assertEqual(result['metadataOrder'], order if name != 'empty' else None)

    def test_payload_type_reference_score_count_and_summary_tampering(self):
        rng = np.random.default_rng(920)
        train = rng.normal(size=(1024, 6)).astype('f4')
        query = train[[0]]
        expected = truth(train, query)['all']
        hits = [{**payload(i), 'score': score} for i, score in zip(expected[0]['ids'], expected[0]['scores'])]
        order = tuple(hits[0]['metadata'])
        row = {'query': 0, 'seconds': .01, 'error': None, 'expected': expected[0]['ids'], 'hits': sorted(expected[0]['ids']),
               'returned': [{'id': hit['id'], 'score': hit['score']} for hit in hits], 'payloadSha256': legacy_digest(hits, order)}
        summary = {'filter': 'all', 'state': 'baseline', 'configuration': {'route': 'exact'}, **summarize([row], 1, True)}
        bad_payload = copy.deepcopy(hits)
        bad_payload[0]['metadata']['epoch'] = False
        invalid = []
        changed = copy.deepcopy(row)
        changed['payloadSha256'] = legacy_digest(bad_payload, order)
        invalid.append([changed])
        changed = copy.deepcopy(row)
        changed['returned'][0]['score'] -= .01
        invalid.append([changed])
        changed = copy.deepcopy(row)
        changed['expected'] = list(reversed(changed['expected']))
        invalid.append([changed])
        changed = copy.deepcopy(row)
        changed['seconds'] *= 2
        invalid.extend([[changed], [row, row], []])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'cell.ndjson'
            for rows in invalid:
                path.write_text(''.join(json.dumps(item) + '\n' for item in rows))
                with self.assertRaises(ValueError):
                    audit_cell(path, summary, engine='warm', train=train, queries=query, expected=expected)

    def test_reference_roundoff_never_waives_membership(self):
        original = {'baseline': {'all': [{'ids': [0], 'scores': [0.], 'eligible': 2, 'cutoff': 0.}]}}
        for delta in (0., .999e-12, 1e-12):
            changed = copy.deepcopy(original)
            changed['baseline']['all'][0]['scores'][0] += delta
            self.assertEqual(audit_reference(original, changed), delta)
        for delta in (1.001e-12, float('nan')):
            changed = copy.deepcopy(original)
            changed['baseline']['all'][0]['scores'][0] += delta
            with self.assertRaises(ValueError):
                audit_reference(original, changed)
        changed = copy.deepcopy(original)
        changed['baseline']['all'][0]['ids'] = [1]
        with self.assertRaises(ValueError):
            audit_reference(original, changed)


if __name__ == '__main__':
    unittest.main()
