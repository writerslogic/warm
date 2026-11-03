"""Run the frozen filtered-query phase and ordered-dot diagnostic."""
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import statistics
import subprocess

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-filtered-dense-phase-20261103-v1')
INPUTS = Path('/Volumes/A/.hms-bench-cache/warm-performance-inputs-20261103-v1')
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
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


def summarize():
    expected = {(dataset, state, name) for dataset in ('coco-i2i-512-angular', 'deep-image-96-angular')
                for state in ('baseline', 'churn-maintained') for name in ('10pct', '1pct')}
    ordinals = {(repetition, ordinal) for repetition in range(3) for ordinal in range(32)}
    cells = {}
    fields = ('prepare', 'capture', 'rank', 'read', 'release', 'total', 'scalar', 'interleaved')
    with (ROOT / 'output/queries.ndjson').open() as stream:
        for line in stream:
            if len(line) > 65536:
                raise ValueError('oversized query record')
            row = json.loads(line)
            key = row['dataset'], row['state'], row['filter']
            identity = row['repetition'], row['queryOrdinal']
            if key not in expected or identity not in ordinals or identity in cells.setdefault(key, {}):
                raise ValueError('undeclared or duplicate query')
            count = (10000 if key[2] == '10pct' else 1000) - ((10 if key[2] == '10pct' else 1) if key[1] != 'baseline' else 0)
            if type(row['dotComparisons']) is not int or row['dotComparisons'] != count:
                raise ValueError('kernel comparison cohort changed')
            for field in fields:
                value = row[field + 'Seconds']
                if type(value) not in (int, float) or not math.isfinite(value) or value <= 0:
                    raise ValueError('invalid timing: ' + field)
            if not math.isclose(row['totalSeconds'], sum(row[field + 'Seconds'] for field in fields[:5]), rel_tol=1e-10, abs_tol=1e-12):
                raise ValueError('phase times do not cover the query')
            cells[key][identity] = row
    if set(cells) != expected or any(set(rows) != ordinals for rows in cells.values()):
        raise ValueError('incomplete diagnostic cohort')
    summaries = []
    for key, rows in sorted(cells.items()):
        values = list(rows.values())
        medians = {field: statistics.median(row[field + 'Seconds'] for row in values) for field in fields}
        speedup = medians['scalar'] / medians['interleaved']
        summaries.append({'dataset': key[0], 'state': key[1], 'filter': key[2], 'queries': len(values),
                          'medianSeconds': medians, 'kernelSpeedup': speedup, 'kernelGatePass': speedup >= 2,
                          'medianPhaseFractions': {field: statistics.median(row[field + 'Seconds'] / row['totalSeconds'] for row in values) for field in fields[:5]},
                          'dotComparisons': sum(row['dotComparisons'] for row in values)})
    write(ROOT / 'summary.json', summaries)
    return summaries


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    def verify():
        for directory, files in ((ROOT / 'source', freeze['files']), (INPUTS, freeze['inputs'])):
            for name, expected in files.items():
                if digest(directory / name) != expected:
                    raise ValueError('frozen source or input changed: ' + name)
        if digest(ROOT / 'run.py') != freeze['runnerSha256'] or digest(ROOT / 'preflight.log') != freeze['preflightSha256']:
            raise ValueError('frozen runner or preflight changed')
    verify()
    command = ['cargo', 'test', '--release', '--locked', '--lib', '--features', 'security',
               'core::documents::search::phase_probe::frozen_filtered_phase_diagnostic',
               '--', '--ignored', '--exact', '--nocapture', '--test-threads=1']
    with (ROOT / 'native.log').open('xb') as stream:
        process = subprocess.Popen(command, cwd=ROOT / 'source', env=ENV, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            if process.wait(timeout=7200):
                raise RuntimeError('native phase diagnostic failed; log retained')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    if 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in (ROOT / 'native.log').read_text():
        raise ValueError('exactly one diagnostic must execute')
    cases = summarize()
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'queryRows': 768, 'cells': len(cases), 'dotComparisons': sum(case['dotComparisons'] for case in cases),
            'kernelGatePass': all(case['kernelGatePass'] for case in cases),
            'qualification': False, 'productionChanged': False, 'freezeSha256': digest(ROOT / 'freeze.json'),
            'outputs': {name: digest(ROOT / name) for name in ('summary.json', 'native.log', 'output/queries.ndjson')}}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
