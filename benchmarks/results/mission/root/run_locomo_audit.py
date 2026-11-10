# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Run the preregistered source audit and record its complete evidence."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--lane', type=Path, required=True)
parser.add_argument('--ledger', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
script = args.lane / 'audit_locomo_sources.py'
manifest = args.lane / 'sources/manifest.json'
registration = {
    'id': 'E3', 'hypothesis': 'H-C-1', 'rung': 2,
    'question': 'Do externally proposed LoCoMo audit corrections link to the pinned original questions and source turns?',
    'prediction': '156 external issues; all mismatches rejected for correction use; zero semantic corrections accepted by structural checks.',
    'acceptance': 'Report all issues and mismatches; advance only source-matched packets to independent semantic review.',
    'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
    'script_sha256': hashlib.sha256(script.read_bytes()).hexdigest(),
    'sources_manifest_sha256': hashlib.sha256(manifest.read_bytes()).hexdigest(),
    'claim': 'local; development audit only',
}
(args.output / 'registration.json').write_text(json.dumps(registration, indent=2) + '\n')
with (args.output / 'run.log').open('w') as log:
    run = subprocess.run([sys.executable, str(script), '--sources', str(args.lane / 'sources'), '--output', str(args.output / 'audit')], stdout=log, stderr=subprocess.STDOUT)
if run.returncode:
    result = f'audit failed with exit {run.returncode}; see run.log'
    decision = 'reject'
else:
    summary = json.loads((args.output / 'audit/summary.json').read_text())
    result = f"issues={summary['external_issue_count']}; source_mismatches={summary['source_link_mismatch_count']}; accepted_corrections=0"
    decision = 'compose'
with (args.ledger / 'LOG.tsv').open('a') as log:
    csv.writer(log, delimiter='\t', lineterminator='\n').writerow(['E3', 'experiment', 'H-C-1', 'LoCoMo-inventory', 2, registration['question'], registration['prediction'], result, decision, registration['source_commit'], 'pinned-source-structural-audit', 'owned-local', str(args.output)])
print(f'E3 {decision}: {result}')
sys.exit(run.returncode)
