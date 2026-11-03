"""Run the isolated same-store tree/scan comparison after queued local checks."""

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import statistics
import subprocess
import time


ROOT = Path('/Volumes/A/.hms-bench-cache/warm-filtered-dense-diagnostic-20261103-v3')
WAIT = Path('/Volumes/A/.hms-bench-cache/warm-c-family-qualification-20261103-v2/finished.json')
INPUTS = Path('/Volumes/A/.hms-bench-cache/warm-performance-inputs-20261103-v1')
ENV = {name: value for name, value in os.environ.items() if name in ('PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV.update({'CARGO_TARGET_DIR': '/Users/davidcondrey/.cache/warm-filtered-dense-diagnostic-target',
            'CARGO_BUILD_JOBS': '2', 'RAYON_NUM_THREADS': '2',
            'WARM_DENSE_DIAGNOSTIC_INPUT': str(INPUTS),
            'WARM_DENSE_DIAGNOSTIC_OUTPUT': str(ROOT / 'output')})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def verify(freeze):
    for name, expected in freeze['files'].items():
        if digest(ROOT / 'source' / name) != expected:
            raise ValueError('frozen source changed: ' + name)
    for name, expected in freeze['inputs'].items():
        if digest(INPUTS / name) != expected:
            raise ValueError('exposed input changed: ' + name)
    if digest(ROOT / 'run.py') != freeze['runnerSha256']:
        raise ValueError('runner changed')


def main():
    deadline = time.monotonic() + 129600
    while not WAIT.exists():
        if time.monotonic() > deadline:
            raise TimeoutError('prior local checks still pending after 36 hours')
        time.sleep(60)
    write(ROOT / 'prior-work-terminal.json', json.loads(WAIT.read_text()))
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    verify(freeze)
    work = ROOT / 'work'
    shutil.copytree(ROOT / 'source', work)
    version = subprocess.check_output(['rustc', '--version'], env=ENV, text=True).strip()
    if not version.startswith('rustc 1.96.0 '):
        raise ValueError('unexpected Rust toolchain')
    with (ROOT / 'native.log').open('xb') as stream:
        process = subprocess.Popen(['cargo', 'test', '--release', '--locked', '--lib', '--features', 'security',
                                    'core::documents::dense::tests::frozen_filtered_dense_counterfactual', '--', '--ignored', '--exact',
                                    '--nocapture', '--test-threads=1'],
                                   cwd=work, env=ENV, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=14400)
            if code:
                raise RuntimeError('native diagnostic failed with status ' + str(code))
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    log = (ROOT / 'native.log').read_text()
    if 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in log:
        raise ValueError('exactly one explicit diagnostic must execute')
    rows, summary = 0, []
    output = ROOT / 'output'
    for path in sorted(output.glob('*.ndjson')):
        cells = {}
        with path.open() as stream:
            for line in stream:
                if len(line) > 65536:
                    raise ValueError('oversized diagnostic result')
                row = json.loads(line)
                cells.setdefault((row['state'], row['filter']), []).append(row)
                rows += 1
        for (state, filter_name), values in sorted(cells.items()):
            if len(values) != 96 or {(v['repetition'], v['queryOrdinal']) for v in values} != {(r, q) for r in range(3) for q in range(32)}:
                raise ValueError('diagnostic query cohort mismatch')
            summary.append({'dataset': path.stem, 'state': state, 'filter': filter_name, 'queries': len(values),
                            'treeMedian': statistics.median(v['treeSeconds'] for v in values),
                            'scanMedian': statistics.median(v['scanSeconds'] for v in values),
                            'medianPredicateVisits': statistics.median(v['metadataPredicateVisits'] for v in values),
                            'medianVectorDots': statistics.median(v['vectorDots'] for v in values),
                            'allRowsVisitedQueries': sum(v['metadataPredicateVisits'] == v['indexRows'] for v in values)})
    if rows != 1536 or len(summary) != 16:
        raise ValueError('missing diagnostic cells')
    write(ROOT / 'summary.json', summary)
    verify(freeze)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(), 'rust': version,
            'pairedComparisons': rows, 'cells': len(summary), 'freezeSha256': digest(ROOT / 'freeze.json'),
            'outputs': {str(path.relative_to(ROOT)): digest(path) for path in sorted(output.glob('*.ndjson'))}}


if __name__ == '__main__':
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(ROOT / 'finished.json', result)
