"""Run the frozen wholly withdrawable matrix-witness cohort."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys

from matrix_witness import MAX_GROUP, audit, family
from run_witness import digest, write


def main(root):
    freeze = json.loads((root / 'freeze.json').read_text())
    source = Path(__file__).resolve().parent
    def verify():
        for name, expected in freeze['files'].items():
            if digest(source / name) != expected:
                raise ValueError('frozen source changed: ' + name)
    verify()
    with (root / 'tests.log').open('xb') as log:
        subprocess.run([sys.executable, '-m', 'unittest', '-v', 'test_matrix_witness'],
                       cwd=source, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
    cases = []
    with (root / 'readouts.ndjson').open('x') as stream:
        def emit(value):
            stream.write(json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n')
        for group in range(1, MAX_GROUP + 1):
            sources, profiles = family(group)
            write(root / f'family-{group}.json', {'sources': sources, 'profiles': profiles})
            cases.append(audit(group, emit))
    assert sum(case['comparisons'] for case in cases) == 37448
    assert sum(case['unknownMasks'] for case in cases) == 17841
    assert sum(len(case['distinguishingMasks']) for case in cases) == 62
    write(root / 'summary.json', cases)
    verify()
    return {'complete': True, 'gatePass': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'cases': len(cases), 'readoutComparisons': 37448, 'jointUnknownMasks': 17841,
            'distinguishingMasks': 62, 'existingCheckerCases': 3,
            'freezeSha256': digest(root / 'freeze.json'),
            'outputs': {path.name: digest(path) for path in sorted(root.iterdir())
                        if path.is_file() and (path.name.startswith('family-') or path.name in ('tests.log', 'readouts.ndjson', 'summary.json'))}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = args.output.resolve()
    result = {'complete': False}
    try:
        result = main(root)
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(root / 'finished.json', result)
