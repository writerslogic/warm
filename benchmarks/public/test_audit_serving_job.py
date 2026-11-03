# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import copy
import json
from pathlib import Path
import tempfile
import unittest

import numpy as np

from audit_serving_job import load, validate_cells, verify_cold, verify_phases
from adapters import payload
from measure import payload_digest, query_order
from oracle import truth


def plan():
    result = json.loads((Path(__file__).parent / 'matched_server/PLAN.json').read_text())
    result.update({'rows': 256, 'queries': 12, 'datasets': {'synthetic': 16}})
    return result


def envelope(policy):
    identity = {'engine': 'warm', 'dataset': 'synthetic', 'repetition': 0, 'hardware': 'development'}
    selected = {'build': {}, 'filters': {name: {'route': 'exact', 'ef': 512} for name in policy['filters']}}
    cells = [{**identity, 'state': state, 'filter': name, 'clients': clients,
              'configuration': selected['filters'][name], 'churn': state != 'baseline',
              'developmentOnly': True, 'evidenceVerified': False, 'isolationPassed': False}
             for state in policy['states'] for name in policy['filters'] for clients in policy['qualification']['concurrency']]
    return {'complete': True, 'failure': None, 'developmentOnly': True, **identity, 'cells': cells}, selected, identity


class ServingJobAuditTests(unittest.TestCase):
    def test_complete_cell_grid_rejects_missing_duplicate_order_identity_and_mode_tampering(self):
        policy = plan()
        result, selected, identity = envelope(policy)
        self.assertEqual(len(validate_cells(result, policy, selected, identity, True)), 32)
        mutants = []
        missing = copy.deepcopy(result); missing['cells'].pop(); mutants.append(missing)
        duplicate = copy.deepcopy(result); duplicate['cells'][-1] = duplicate['cells'][0]; mutants.append(duplicate)
        ordered = copy.deepcopy(result); ordered['cells'].reverse(); mutants.append(ordered)
        for name, value in [('clients', True), ('configuration', {'route': 'graph', 'ef': 512}),
                            ('repetition', False), ('churn', 0), ('isolationPassed', True)]:
            changed = copy.deepcopy(result); changed['cells'][0][name] = value; mutants.append(changed)
        mode = copy.deepcopy(result); mode['developmentOnly'] = False; mutants.append(mode)
        identity_changed = copy.deepcopy(result); identity_changed['repetition'] = False; mutants.append(identity_changed)
        for changed in mutants:
            with self.subTest(changed=changed['cells'][0]), self.assertRaises(ValueError):
                validate_cells(changed, policy, selected, identity, True)

    def test_lifecycle_receipts_and_resource_limits_are_cross_checked(self):
        policy = plan()
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            operations = [{'operation': 'replace' if i < 100 else 'delete', 'id': i, 'seconds': .01} for i in range(200)]
            for item in operations:
                (folder / f"{item['operation']}-{item['id']}.json").write_text(json.dumps(item))
            labels = ['build', 'baseline', 'churn', 'churn-before-maintenance', 'maintenance',
                      'churn-after-maintenance', 'reopen', 'final']
            phases = [{'phase': name, 'allocatedStoreBytes': 1024} for name in labels]
            phases[0].update(seconds=1., settings={'declared': 'build'})
            phases[4].update(seconds=.1, settings={'declared': 'maintenance'})
            phases[2].update(operations=operations, acknowledgedReplacementsChecked=100, acknowledgedDeletionsChecked=100)
            result = {'phases': phases, 'peakServerPssBytes': 100}
            samples = [{'monotonic': float(i), 'serverPssBytes': i * 50, 'load': [0., 0., 0.]} for i in (1, 2)]
            (folder / 'resources.ndjson').write_text(''.join(json.dumps(item) + '\n' for item in samples))
            self.assertEqual(verify_phases(folder, result, policy)['mutations'], 200)
            limit = policy['resources']['maxStoreBytes']
            for value in (limit - 1, limit, limit + 1):
                changed = copy.deepcopy(result); changed['phases'][-1]['allocatedStoreBytes'] = value
                if value > limit:
                    with self.assertRaisesRegex(ValueError, 'store size'):
                        verify_phases(folder, changed, policy)
                else:
                    verify_phases(folder, changed, policy)
            for value in (policy['resources']['mutationSeconds'] - 1, policy['resources']['mutationSeconds'], policy['resources']['mutationSeconds'] + 1):
                changed = copy.deepcopy(result); changed['phases'][2]['operations'][0]['seconds'] = value
                (folder / 'replace-0.json').write_text(json.dumps(changed['phases'][2]['operations'][0]))
                if value > policy['resources']['mutationSeconds']:
                    with self.assertRaisesRegex(ValueError, 'duration'):
                        verify_phases(folder, changed, policy)
                else:
                    verify_phases(folder, changed, policy)
            (folder / 'replace-0.json').write_text(json.dumps(operations[0]))
            (folder / 'delete-100.json').write_text(json.dumps({**operations[100], 'id': 101}))
            with self.assertRaisesRegex(ValueError, 'receipt differs'):
                verify_phases(folder, result, policy)

    def test_cold_open_validates_scores_membership_payload_and_latency(self):
        policy = plan()
        train = np.eye(16, dtype='<f4')[np.arange(256) % 16]
        queries = np.eye(16, dtype='<f4')[:12]
        expected = truth(train, queries, True)['all']
        ordinal = query_order('synthetic', 0, 12)[0]
        ids = expected[ordinal]['ids']
        for engine in ('warm', 'pgvector', 'opensearch'):
            returned = [{'id': str(identifier), 'score': score if engine != 'opensearch' else (1 + score) / 2}
                        for identifier, score in zip(ids, expected[ordinal]['scores'])]
            hits = [{**payload(int(item['id']), int(int(item['id']) < 100)), 'score': item['score']} for item in returned]
            cold = {'query': ordinal, 'expected': ids, 'startedAt': 1., 'readyAt': 2., 'receivedAt': 2.5,
                    'validatedAt': 3., 'readySeconds': 1., 'firstValidatedResponseSeconds': 2.,
                    'returned': returned, 'hits': sorted(ids), 'payloadSha256': payload_digest(hits),
                    'maximumScoreError': 0., 'recall': 1., 'unresolvedExactMembership': 0, 'pass': True}
            kwargs = dict(engine=engine, dataset='synthetic', repetition=0, configuration={'route': 'exact'},
                          queries=queries, train=train, expected=expected, limits=policy['resources'])
            self.assertTrue(verify_cold(cold, **kwargs)['qualityPassed'])
            mutants = []
            for key, value in [('readySeconds', .5), ('query', True), ('expected', []),
                               ('payloadSha256', '0' * 64), ('maximumScoreError', .01), ('recall', .9), ('pass', 1)]:
                changed = copy.deepcopy(cold); changed[key] = value; mutants.append(changed)
            changed = copy.deepcopy(cold); changed['returned'][0]['score'] += .1; mutants.append(changed)
            changed = copy.deepcopy(cold); changed['returned'][0]['id'] = '01'; mutants.append(changed)
            for changed in mutants:
                with self.assertRaises(ValueError):
                    verify_cold(changed, **kwargs)

    def test_evidence_reader_rejects_symlinks_and_allocation_overflow(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'record.json'
            path.write_text('{"ok":true}')
            size = path.stat().st_size
            for maximum in (size - 1, size, size + 1):
                if maximum < size:
                    with self.assertRaises(ValueError):
                        load(path, maximum)
                else:
                    self.assertEqual(load(path, maximum), {'ok': True})
            link = Path(directory) / 'linked.json'
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                load(link)


if __name__ == '__main__':
    unittest.main()
