# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import copy
import json
from pathlib import Path
import unittest

from seal_qualification import validate_selection


class SelectionSealTests(unittest.TestCase):
    def setUp(self):
        self.plan = json.loads((Path(__file__).parent / 'PLAN.json').read_text())
        self.source = {'adapters.py': 'adapter-digest'}
        self.binaries = {'warmBinarySha256': 'binary-digest',
                         'postgresBinarySha256': 'postgres-digest',
                         'pgvectorBinarySha256': 'pgvector-digest'}
        self.freeze = {'engine': 'warm', 'dataset': 'deep-image-96-angular', 'plan': self.plan,
                       'sources': {'/frozen/adapters.py': 'adapter-digest'}, **self.binaries}
        build = self.plan['calibration']['warm']['builds'][0]
        filters = {}
        for name in self.plan['filters']:
            configuration = {'route': 'exact'}
            states = [{'state': state, 'filter': name, 'configuration': configuration, 'queries': 1000,
                       'pass': True, 'errors': 0, 'unresolvedExactMembership': 0, 'meanRecall': 1.,
                       'worstRecall': 1., 'median': 1., 'p95': 2., 'p99': 3.} for state in self.plan['states']]
            filters[name] = {'configuration': configuration, 'states': states, 'median': 1.}
        chosen = {'build': build, 'filters': filters, 'objective': 1., 'directory': '/calibration/build-0'}
        self.selection = {'complete': True, 'eligible': True, 'selectedBeforeHeldOutQueries': True,
                          'failures': [], 'candidates': [chosen], 'selected': copy.deepcopy(chosen)}

    def check(self, selection=None, freeze=None):
        return validate_selection(selection or self.selection, freeze or self.freeze, self.plan,
                                  'warm', 'deep-image-96-angular', self.source, self.binaries)

    def test_source_binary_policy_and_completeness_are_binding(self):
        self.assertEqual(self.check(), self.selection['selected'])
        for field, value in [('complete', False), ('eligible', False), ('selectedBeforeHeldOutQueries', False),
                             ('failures', ['failed build'])]:
            with self.assertRaises(ValueError):
                self.check({**self.selection, field: value})
        for field, value in [('warmBinarySha256', 'different'), ('sources', {'/new/adapters.py': 'different'}),
                             ('dataset', 'different'), ('plan', {})]:
            with self.assertRaises(ValueError):
                self.check(freeze={**self.freeze, field: value})

    def test_no_failed_missing_duplicate_or_undeclared_selected_cell(self):
        for mutation in ('failed', 'missing', 'duplicate', 'unknown', 'bad-objective', 'incomplete-queries'):
            selection = copy.deepcopy(self.selection)
            chosen = selection['candidates'][0]['filters']['all']
            if mutation == 'failed':
                chosen['states'][1]['worstRecall'] = .8
            elif mutation == 'missing':
                chosen['states'].pop()
            elif mutation == 'duplicate':
                chosen['states'][-1] = copy.deepcopy(chosen['states'][0])
            elif mutation == 'unknown':
                chosen['configuration'] = {'route': 'graph', 'ef': 9999}
            elif mutation == 'bad-objective':
                chosen['median'] = .5
            else:
                chosen['states'][0]['queries'] = 999
            selection['selected'] = copy.deepcopy(selection['candidates'][0])
            with self.assertRaises(ValueError, msg=mutation):
                self.check(selection)

    def test_rebuilding_one_engine_preserves_other_engines_calibration(self):
        requirements = {'warm': ('warmBinarySha256',),
                        'pgvector': ('postgresBinarySha256', 'pgvectorBinarySha256'),
                        'opensearch': ()}
        for engine, required in requirements.items():
            selection = copy.deepcopy(self.selection)
            selection['candidates'][0]['build'] = self.plan['calibration'][engine]['builds'][0]
            selection['selected'] = copy.deepcopy(selection['candidates'][0])
            freeze = {**self.freeze, 'engine': engine}
            for name in self.binaries:
                changed = {**self.binaries, name: 'rebuilt-artifact'}
                arguments = (selection, freeze, self.plan, engine, 'deep-image-96-angular', self.source, changed)
                if name in required:
                    with self.assertRaises(ValueError, msg=engine + ':' + name):
                        validate_selection(*arguments)
                    missing = {key: value for key, value in self.binaries.items() if key != name}
                    with self.assertRaises(ValueError):
                        validate_selection(selection, freeze, self.plan, engine, 'deep-image-96-angular', self.source, missing)
                else:
                    self.assertEqual(validate_selection(*arguments), selection['selected'])

    def test_a_slower_eligible_build_cannot_replace_the_selected_control(self):
        selection = copy.deepcopy(self.selection)
        fast = selection['candidates'][0]
        fast['build'] = self.plan['calibration']['pgvector']['builds'][0]
        slow = copy.deepcopy(fast)
        slow['build'] = self.plan['calibration']['pgvector']['builds'][1]
        for chosen in slow['filters'].values():
            chosen['median'] = 2.
            for row in chosen['states']:
                row['median'], row['p95'], row['p99'] = 2., 4., 6.
        slow['objective'] = 2.
        selection['candidates'].append(slow)
        selection['selected'] = copy.deepcopy(slow)
        freeze = {**self.freeze, 'engine': 'pgvector'}
        with self.assertRaises(ValueError):
            validate_selection(selection, freeze, self.plan, 'pgvector', 'deep-image-96-angular', self.source, self.binaries)
        selection['selected'] = copy.deepcopy(fast)
        self.assertEqual(validate_selection(selection, freeze, self.plan, 'pgvector', 'deep-image-96-angular',
                                            self.source, self.binaries), fast)


if __name__ == '__main__':
    unittest.main()
