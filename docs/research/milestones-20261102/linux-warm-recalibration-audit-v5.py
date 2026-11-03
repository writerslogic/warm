"""Audit the changed WARM selections after their exposed-data calibration finishes."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103')
SOURCE = ROOT / 'calibration-audit-driver-v2'
OUTPUT = ROOT / 'evidence/warm-recalibration-audit-v5'
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV.update({'OPENBLAS_NUM_THREADS': '1', 'OMP_NUM_THREADS': '1'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    OUTPUT.mkdir()
    manifest = SOURCE / 'source.json'
    if digest(manifest) != 'bae6c148f79f8ecc2f7bc1c007dd8db31410f0e2563a52546869ed0b29976a79':
        raise ValueError('auditor source manifest changed')
    files = json.loads(manifest.read_text())['files']
    def verify():
        for name, expected in files.items():
            if digest(SOURCE / name) != expected:
                raise ValueError('frozen calibration auditor changed: ' + name)
    verify()
    terminal = ROOT / 'evidence/warm-recalibration-v5/finished.json'
    deadline = time.monotonic() + 28800
    while not terminal.exists():
        if time.monotonic() > deadline:
            raise TimeoutError('recalibration did not finish within eight hours')
        time.sleep(30)
    result = json.loads(terminal.read_text())
    if result.get('complete') is not True:
        raise ValueError('recalibration failed; no audit started')
    cases = []
    for dataset in ('coco-i2i-512-angular', 'deep-image-96-angular'):
        calibration = ROOT / ('evidence/calibration-warm-' + dataset + '-v5')
        output = OUTPUT / dataset
        command = [str(ROOT / 'tools/uv/bin/uv'), 'run', '--project',
                   'benchmarks/public/matched_server', '--frozen', 'python',
                   'benchmarks/public/audit_calibration.py', '--calibration', str(calibration),
                   '--inputs', str(ROOT / 'performance-calibration-inputs'), '--output', str(output)]
        with (OUTPUT / (dataset + '.log')).open('xb') as stream:
            subprocess.run(command, cwd=SOURCE, env=ENV, stdout=stream,
                           stderr=subprocess.STDOUT, check=True, timeout=3600)
        audit = json.loads((output / 'finished.json').read_text())
        if audit.get('complete') is not True or audit.get('heldOutVectorsOpened') is not False:
            raise ValueError('incomplete or incorrectly scoped audit')
        cases.append({'dataset': dataset, 'cells': len(audit['cells']),
                      'queries': sum(cell['queries'] for cell in audit['cells']),
                      'maximumScoreError': max(cell['maximumScoreError'] for cell in audit['cells']),
                      'oracleMaximumNumericalDifference': audit['oracleMaximumNumericalDifference'],
                      'receiptSha256': digest(output / 'finished.json')})
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'cases': cases, 'heldOutVectorsOpened': False,
            'calibrationTerminalSha256': digest(terminal), 'sourceManifestSha256': digest(manifest)}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        with (OUTPUT / 'finished.json').open('x') as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write('\n')
