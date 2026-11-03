"""Execute the preselected source-count/width grid with exact arithmetic."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys

from ambient_witness import MAX_GROUP, audit, family
from run_witness import digest, write


def main(root):
    source = Path(__file__).resolve().parent
    freeze = json.loads((root / 'freeze.json').read_text())
    def verify():
        for name, expected in freeze['files'].items():
            if digest(source / name) != expected:
                raise ValueError('frozen source changed: ' + name)
    verify()
    with (root / 'tests.log').open('xb') as log:
        subprocess.run([sys.executable, '-m', 'unittest', '-v', 'test_ambient_witness'],
                       cwd=source, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
    cases = []
    with (root / 'readouts.ndjson').open('x') as stream:
        def emit(value):
            stream.write(json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n')
        for group in range(1, MAX_GROUP + 1):
            for sources in range(group, 3 * group + 3):
                profiles = family(group, sources)
                write(root / f'family-{group}-{sources}.json', {'sources': sources, 'profiles': profiles})
                cases.append(audit(group, sources, emit))
    assert len(cases) == 32 and sum(case['comparisons'] for case in cases) == 37410
    write(root / 'summary.json', cases)
    verify()
    return {'complete': True, 'gatePass': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'cases': len(cases), 'readoutComparisons': 37410,
            'jointUnknownMasks': sum(case['jointUnknownMasks'] for case in cases),
            'distinguishingMasks': sum(case['distinguishingMasks'] for case in cases),
            'existingCheckerCases': sum(case['existingChecker'] is not None for case in cases),
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
