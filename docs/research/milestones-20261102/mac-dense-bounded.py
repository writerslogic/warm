"""Run the frozen exposed-data bounded-tree development experiment."""

from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import statistics
import subprocess

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-filtered-dense-bounded-20261103-v1')
INPUTS = Path('/Volumes/A/.hms-bench-cache/warm-performance-inputs-20261103-v1')
ENV = {name: value for name, value in os.environ.items() if name in
       ('PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV.update({'CARGO_TARGET_DIR': '/Users/davidcondrey/.cache/warm-filtered-dense-bounded-target',
            'CARGO_BUILD_JOBS': '2', 'RAYON_NUM_THREADS': '2',
            'WARM_DENSE_DIAGNOSTIC_INPUT': str(INPUTS),
            'WARM_DENSE_DIAGNOSTIC_OUTPUT': str(ROOT / 'output')})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def verify(freeze):
    actual = {str(path.relative_to(ROOT / 'source')) for path in (ROOT / 'source').rglob('*') if path.is_file()}
    if actual != set(freeze['files']):
        raise ValueError('source file set changed')
    for name, expected in freeze['files'].items():
        if digest(ROOT / 'source' / name) != expected:
            raise ValueError('source changed: ' + name)
    for name, expected in freeze['inputs'].items():
        if digest(INPUTS / name) != expected:
            raise ValueError('exposed input changed: ' + name)
    for name, expected in freeze['preflight'].items():
        if digest(ROOT / name) != expected:
            raise ValueError('preflight evidence changed: ' + name)
    if digest(ROOT / 'run.py') != freeze['runnerSha256']:
        raise ValueError('runner changed')


def summarize():
    expected_cells = {(dataset, state, filter_name)
                      for dataset in ('coco-i2i-512-angular', 'deep-image-96-angular')
                      for state in ('baseline', 'churn-maintained')
                      for filter_name in ('all', '10pct', '1pct', 'empty')}
    expected_cells |= {(f'separated-{count}-{dimensions}', 'baseline', '50pct')
                       for count in (8192, 32768) for dimensions in (2, 4)}
    expected_queries = {(repetition, ordinal) for repetition in range(3) for ordinal in range(32)}
    cells = {}
    path = ROOT / 'output' / 'queries.ndjson'
    with path.open() as stream:
        for line in stream:
            if len(line) > 65536:
                raise ValueError('oversized query record')
            row = json.loads(line)
            key = (row['dataset'], row['state'], row['filter'])
            if key not in expected_cells:
                raise ValueError('undeclared cell')
            query = (row['repetition'], row['queryOrdinal'])
            if query not in expected_queries or query in cells.setdefault(key, {}):
                raise ValueError('invalid or repeated query')
            for field in ('originalSeconds', 'boundedSeconds', 'scanSeconds'):
                value = row[field]
                if type(value) not in (float, int) or not math.isfinite(value) or value <= 0:
                    raise ValueError('invalid timing')
            if row['boundedRoute'] not in ('tree', 'exhausted-then-scan', 'scan', 'unlimited-tree'):
                raise ValueError('unknown route')
            cells[key][query] = row
    if set(cells) != expected_cells or any(set(values) != expected_queries for values in cells.values()):
        raise ValueError('incomplete cohort')
    summaries, failures = [], []
    for key, queries in sorted(cells.items()):
        values = list(queries.values())
        medians = {route: statistics.median(row[route + 'Seconds'] for row in values)
                   for route in ('original', 'bounded', 'scan')}
        scan_ratio = medians['bounded'] / medians['scan']
        original_ratio = medians['bounded'] / medians['original']
        cell_failures = []
        if key[0].startswith('separated-'):
            if original_ratio > 1.15:
                cell_failures.append('bounded/original median > 1.15')
            if any(row['boundedRoute'] != 'tree' or type(row['completedTreeDots']) is not int
                   or not 0 <= row['completedTreeDots'] < row['eligibleChunks'] for row in values):
                cell_failures.append('useful pruning not retained on every analytic query')
        elif key[2] in ('1pct', '10pct'):
            if scan_ratio > 1.15:
                cell_failures.append('bounded/scan median > 1.15')
            if key[2] == '1pct' and original_ratio > 0.125:
                cell_failures.append('bounded/original median > 0.125')
        repetitions = []
        for repetition in range(3):
            group = [row for row in values if row['repetition'] == repetition]
            repetitions.append({'repetition': repetition, 'medianSeconds': {
                route: statistics.median(row[route + 'Seconds'] for row in group)
                for route in ('original', 'bounded', 'scan')}})
        summary = {'dataset': key[0], 'state': key[1], 'filter': key[2], 'queries': len(values),
                   'medianSeconds': medians, 'boundedOverScan': scan_ratio,
                   'boundedOverOriginal': original_ratio,
                   'routes': dict(Counter(row['boundedRoute'] for row in values)),
                   'repetitions': repetitions, 'gateFailures': cell_failures}
        summaries.append(summary)
        if cell_failures:
            failures.append({'cell': key, 'reasons': cell_failures})
    write(ROOT / 'summary.json', summaries)
    return {'pass': not failures, 'failures': failures}


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    verify(freeze)
    version = subprocess.check_output(['rustc', '--version'], env=ENV, text=True).strip()
    if not version.startswith('rustc 1.96.0 '):
        raise ValueError('unexpected Rust toolchain')
    with (ROOT / 'native.log').open('xb') as stream:
        process = subprocess.Popen(['cargo', 'test', '--release', '--locked', '--lib', '--features', 'security',
                                    'core::documents::search::tests::frozen_bounded_dense_comparison',
                                    '--', '--ignored', '--exact', '--nocapture', '--test-threads=1'],
                                   cwd=ROOT / 'source', env=ENV, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=14400)
            if code:
                raise RuntimeError('native comparison failed with status ' + str(code))
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    if 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in (ROOT / 'native.log').read_text():
        raise ValueError('exactly one explicit comparison must execute')
    gate = summarize()
    verify(freeze)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(), 'rust': version,
            'queryRows': 1920, 'pairedComparisons': 3840, 'cells': 20, 'developmentGate': gate,
            'qualification': False, 'freezeSha256': digest(ROOT / 'freeze.json'),
            'outputs': {name: digest(ROOT / name) for name in ('native.log', 'summary.json', 'output/queries.ndjson')}}


if __name__ == '__main__':
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(ROOT / 'finished.json', result)
