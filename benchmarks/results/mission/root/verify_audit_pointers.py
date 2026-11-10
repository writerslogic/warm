# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Verify source pointers in semantic review artifacts without accepting their conclusions."""
import argparse
import csv
import hashlib
import json
import re
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--source', type=Path, required=True)
p.add_argument('--reports', nargs='+', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--ledger', type=Path, required=True)
p.add_argument('--experiment', required=True)
p.add_argument('--expected-type')
p.add_argument('--external-sources', type=Path)
a = p.parse_args()
if a.output.exists() or not a.experiment[1:].isdigit() or not a.experiment.startswith('E'):
    p.error('unique output and numeric experiment ID required')
source = json.loads(a.source.read_text())
sha = hashlib.sha256(a.source.read_bytes()).hexdigest()
def resolve(pointer):
    value = source
    if not pointer.startswith('/'):
        raise ValueError('absolute JSON pointer required')
    for part in pointer[1:].split('/'):
        key = part.replace('~1', '/').replace('~0', '~')
        value = value[int(key)] if isinstance(value, list) else value[key]
    return value
receipts = []
seen = set()
for path in a.reports:
    report = json.loads(path.read_text())
    if report['source_sha256'] != sha:
        raise ValueError('source hash mismatch')
    refs = 0
    for item in report['reviewed']:
        identity = re.fullmatch(r'locomo_(\d+)_qa(\d+)', item['id'])
        if not identity or item['id'] in seen or item['official_pointer'] != f'/{int(identity[1])}/qa/{int(identity[2])}':
            raise ValueError('duplicate or inconsistent claim identity')
        seen.add(item['id'])
        if resolve(item['official_pointer']) != item['official']:
            raise ValueError('official question mismatch: ' + item['id'])
        for identity, evidence in item['evidence'].items():
            turn = resolve(evidence['pointer'])
            if any(turn[key] != evidence[key] for key in ('speaker', 'text')) or turn['dia_id'] != identity:
                raise ValueError('source evidence mismatch: ' + item['id'])
            if resolve(evidence['session_date_pointer']) != evidence['session_date']:
                raise ValueError('source date mismatch: ' + item['id'])
            refs += 1
    receipts.append({'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'claims': len(report['reviewed']), 'source_references': refs})
if a.expected_type:
    if a.external_sources is None:
        p.error('--expected-type requires --external-sources')
    expected = {item['question_id'] for index in range(10) for item in json.loads((a.external_sources / f'errors_conv_{index}.json').read_text()) if item['error_type'] == a.expected_type}
    if expected != seen:
        raise ValueError('external claim coverage mismatch')
result = {'experiment': a.experiment, 'hypothesis': 'H-C-1', 'source_sha256': sha, 'receipts': receipts, 'claim': 'local structural verification only', 'accepted_semantic_corrections': 0, 'official_score': None, 'audited_score': None}
a.output.write_text(json.dumps(result, indent=2) + '\n')
with a.ledger.open('a') as f:
    csv.writer(f, delimiter='\t', lineterminator='\n').writerow([a.experiment, 'experiment', 'H-C-1', 'LoCoMo-inventory', 2, 'Do reviewed claims preserve exact source questions, speaker text and dates?', 'All pointers match; no semantic acceptance inferred', f"{sum(r['claims'] for r in receipts)} claims and {sum(r['source_references'] for r in receipts)} references match", 'compose', 'source pins in receipt', 'exact JSON pointer comparison', 'owned-local', str(a.output.resolve())])
print(f"{a.experiment}: {sum(r['claims'] for r in receipts)} claims structurally verified; semantic conclusions remain proposals")
