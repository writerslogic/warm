"""Reproduce the positive-evidence tightness construction without model calls."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from witness_tightness import audit, family


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, data):
    with path.open('x') as stream:
        json.dump(data, stream, indent=2, sort_keys=True)
        stream.write('\n')


def main(root):
    freeze = json.loads((root / 'freeze.json').read_text())
    source = Path(__file__).resolve().parent
    def verify():
        for name, expected in freeze['files'].items():
            if digest(source / name) != expected:
                raise ValueError('source changed: ' + name)
    verify()
    with (root / 'tests.log').open('xb') as log:
        subprocess.run([sys.executable, '-m', 'unittest', '-v', 'test_witness_tightness'],
                       cwd=source, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
    cases = []
    with (root / 'readouts.ndjson').open('x') as stream:
        def emit(value):
            stream.write(json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n')
        for anchored, groups in [(False, range(1, 7)), (True, range(1, 6))]:
            for group in groups:
                sources, profiles = family(group, anchored)
                write(root / f'family-{group}-{int(anchored)}.json', {'sources': sources, 'profiles': profiles})
                cases.append(audit(group, anchored, emit))
    write(root / 'summary.json', cases)
    verify()
    return {'complete': True, 'gatePass': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'cases': len(cases), 'readoutComparisons': sum(case['comparisons'] for case in cases),
            'existingCheckerCases': sum(case['existingChecker'] is not None for case in cases),
            'freezeSha256': digest(root / 'freeze.json'),
            'outputs': {path.name: digest(path) for path in sorted(root.iterdir())
                        if path.is_file() and (path.name.startswith('family-') or path.name in ('tests.log', 'readouts.ndjson', 'summary.json'))}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = args.output.resolve()
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main(root)
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(root / 'finished.json', result)
