# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Execute one registered local counter and append its evidence to the mission log."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import subprocess
import sys

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--registration', type=Path, required=True)
p.add_argument('--ledger', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
r = json.loads(a.registration.read_text())
a.output.mkdir(parents=True, exist_ok=False)
r['source_commit'] = subprocess.check_output(['git', '-C', r['cwd'], 'rev-parse', 'HEAD'], text=True).strip()
r['script_sha256'] = hashlib.sha256(Path(r['script']).read_bytes()).hexdigest()
(a.output / 'registration.json').write_text(json.dumps(r, indent=2) + '\n')
with (a.output / 'run.log').open('w') as log:
    run = subprocess.run(r['argv'], cwd=r['cwd'], stdout=log, stderr=subprocess.STDOUT)
if run.returncode:
    result = {'exit_code': run.returncode, 'decision': 'reject-execution-failure'}
else:
    result = json.loads(Path(r['result']).read_text())
    (a.output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
summary = {k: result[k] for k in r['summary_fields'] if k in result}
summary['decision'] = result['decision']
with (a.ledger / 'LOG.tsv').open('a') as log:
    csv.writer(log, delimiter='\t', lineterminator='\n').writerow([r['id'], 'experiment', r['hypothesis'], r['row'], 2, r['question'], r['prediction'], json.dumps(summary, sort_keys=True), 'reject' if result['decision'].startswith('reject') else 'compose', r['source_commit'], r['configuration'], 'owned-local', str(a.output)])
print(r['id'] + ' ' + json.dumps(summary, sort_keys=True))
sys.exit(run.returncode)
