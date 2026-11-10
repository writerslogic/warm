# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check that malformed review identities and incomplete coverage cannot pass provenance verification."""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import sys

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--sources', required=True, type=Path)
p.add_argument('--report', required=True, type=Path)
p.add_argument('--output', required=True, type=Path)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
original = json.loads(a.report.read_text())
cases = {}
duplicate = copy.deepcopy(original)
duplicate['reviewed'].append(copy.deepcopy(duplicate['reviewed'][0]))
cases['duplicate'] = duplicate
identity = copy.deepcopy(original)
identity['reviewed'][0]['id'] = 'locomo_0_qa999999'
cases['wrong_identity'] = identity
missing = copy.deepcopy(original)
missing['reviewed'].pop()
cases['missing_claim'] = missing
results = {}
for name, report in cases.items():
    path = a.output / (name + '.json')
    path.write_text(json.dumps(report) + '\n')
    command = [sys.executable, str(Path(__file__).with_name('verify_audit_pointers.py')), '--source', str(a.sources / 'locomo10.json'), '--reports', str(path), '--expected-type', 'HALLUCINATION', '--external-sources', str(a.sources), '--output', str(a.output / (name + '-result.json')), '--ledger', str(a.output / 'isolated-ledger.tsv'), '--experiment', 'E0']
    run = subprocess.run(command, capture_output=True, text=True)
    (a.output / (name + '.log')).write_text(run.stdout + run.stderr)
    results[name] = {'exit_code': run.returncode, 'result_created': (a.output / (name + '-result.json')).exists()}
    if run.returncode == 0 or results[name]['result_created']:
        raise AssertionError('malformed review accepted: ' + name)
(a.output / 'summary.json').write_text(json.dumps(results, indent=2) + '\n')
print('3 malformed review cases rejected before evidence publication')
