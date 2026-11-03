"""Execute the frozen certified arithmetic screen after local compilation ends."""
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import statistics
import subprocess
import time

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-certified-scan-20261103-v1')
INPUTS = Path('/Volumes/A/.hms-bench-cache/warm-performance-inputs-20261103-v1')
PREREQUISITE = Path('/Volumes/A/.hms-bench-cache/warm-android-qualification-20261103-v6/finished.json')
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
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


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    version = subprocess.check_output(['rustc', '-vV'], env=ENV, text=True)
    if version != freeze['rustcVersion']:
        raise ValueError('frozen Rust toolchain changed')
    def verify():
        for directory, files in ((ROOT / 'source', freeze['files']), (INPUTS, freeze['inputs'])):
            for name, expected in files.items():
                if digest(directory / name) != expected:
                    raise ValueError('frozen source or input changed: ' + name)
        for name, expected in freeze['artifacts'].items():
            if digest(ROOT / name) != expected:
                raise ValueError('frozen artifact changed: ' + name)
    verify()
    deadline = time.monotonic() + 7200
    while not PREREQUISITE.exists():
        if time.monotonic() >= deadline:
            raise TimeoutError('local Android job has not finished')
        time.sleep(30)
    prerequisite = json.loads(PREREQUISITE.read_text())
    write(ROOT / 'prerequisite.json', {'path': str(PREREQUISITE), 'sha256': digest(PREREQUISITE),
                                     'complete': prerequisite.get('complete'),
                                     'purpose': 'Avoid simultaneous local qualification load; no result reuse'})
    with (ROOT / 'native.log').open('xb') as log:
        command = ['cargo', 'test', '--release', '--locked', '--lib', '--features', 'security',
                   'core::documents::search::certified_probe::frozen_certified_scan_comparison',
                   '--', '--exact', '--ignored', '--nocapture', '--test-threads=1']
        process = subprocess.Popen(command, cwd=ROOT / 'source', env=ENV,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            if process.wait(timeout=7200):
                raise RuntimeError('native screen failed; log retained')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    if 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in (ROOT / 'native.log').read_text():
        raise ValueError('exactly one screen must execute')
    states = ('baseline-untrained', 'baseline-trained', 'churn-untrained', 'churn-trained')
    expected = {(dataset, state, name) for dataset in ('coco-i2i-512-angular', 'deep-image-96-angular')
                for state in states for name in ('all', '10pct', '1pct', 'empty')}
    expected |= {(f'separated-{count}-{dimensions}', 'baseline', '50pct')
                 for count in (8192, 32768) for dimensions in (2, 4)}
    ordinals = {(repetition, ordinal) for repetition in range(3) for ordinal in range(32)}
    cells = {}
    with (ROOT / 'output/queries.ndjson').open() as stream:
        for line in stream:
            if len(line) > 65536:
                raise ValueError('oversized query record')
            row = json.loads(line)
            key = row['dataset'], row['state'], row['filter']
            identity = row['repetition'], row['queryOrdinal']
            if key not in expected or identity not in ordinals or identity in cells.setdefault(key, {}):
                raise ValueError('undeclared or duplicate query')
            for field in ('baselineSeconds', 'previousSeconds', 'candidateSeconds'):
                value = row[field]
                if type(value) not in (int, float) or not math.isfinite(value) or value <= 0:
                    raise ValueError('invalid timing')
            if key[0].startswith('separated-'):
                eligible = int(key[0].split('-')[1]) // 2
                if type(row.get('prunedDots')) is not int or not 0 <= row['prunedDots'] < eligible:
                    raise ValueError('analytic pruning not retained')
            elif row.get('prunedDots') is not None:
                raise ValueError('unexpected analytic pruning count')
            owners, capacity = row.get('ownerCount'), row.get('ownerCapacity')
            if type(owners) is not int or type(capacity) is not int or not 0 <= owners <= 524288 or not owners <= capacity <= 1048576:
                raise ValueError('invalid owner membership allocation count')
            if key[0].startswith('separated-'):
                expected_owners = int(key[0].split('-')[1]) // 2
            elif key[1].endswith('-trained') and key[2] in ('10pct', '1pct'):
                expected_owners = (10000 if key[2] == '10pct' else 1000) - ((10 if key[2] == '10pct' else 1) if key[1].startswith('churn-') else 0)
            else:
                expected_owners = 0
            if owners != expected_owners or (owners == 0 and capacity != 0):
                raise ValueError('captured membership did not match the declared cohort')
            screened, canonical = row.get('screenedRows'), row.get('canonicalRows')
            if type(screened) is not int or type(canonical) is not int or not 0 <= canonical <= screened <= 100000:
                raise ValueError('invalid certified scan counters')
            if key[0].startswith('separated-') or key[2] in ('all', 'empty'):
                expected_screened = 0
            else:
                expected_screened = (10000 if key[2] == '10pct' else 1000) - ((10 if key[2] == '10pct' else 1) if key[1].startswith('churn-') else 0)
            if screened not in (0, expected_screened) or (key[1].endswith('-untrained') and screened != expected_screened):
                raise ValueError('certified scan did not match eligible rows')
            cells[key][identity] = row
    if set(cells) != expected or any(set(rows) != ordinals for rows in cells.values()):
        raise ValueError('incomplete screen cohort')
    summaries = []
    for key, rows in sorted(cells.items()):
        medians = {field: statistics.median(row[field + 'Seconds'] for row in rows.values())
                   for field in ('baseline', 'previous', 'candidate')}
        speedup = medians['baseline'] / medians['candidate']
        minimum = 1.25 if key[2] in ('10pct', '1pct') else 1 / 1.15
        repetitions = []
        for repetition in range(3):
            group = [row for (rep, _), row in rows.items() if rep == repetition]
            repetitions.append({field: statistics.median(row[field + 'Seconds'] for row in group)
                                for field in ('baseline', 'previous', 'candidate')})
        summaries.append({'dataset': key[0], 'state': key[1], 'filter': key[2],
                          'queries': len(rows), 'medianSeconds': medians, 'speedup': speedup,
                          'gatePass': speedup >= minimum, 'minimumSpeedup': minimum,
                          'combinedOverPreviousSpeedup': medians['previous'] / medians['candidate'],
                          'maxOwnerCount': max(row['ownerCount'] for row in rows.values()),
                          'maxOwnerCapacity': max(row['ownerCapacity'] for row in rows.values()),
                          'medianCanonicalFraction': statistics.median(
                              row['canonicalRows'] / row['screenedRows'] for row in rows.values() if row['screenedRows'])
                              if any(row['screenedRows'] for row in rows.values()) else None,
                          'totalScreenedRows': sum(row['screenedRows'] for row in rows.values()),
                          'totalCanonicalRows': sum(row['canonicalRows'] for row in rows.values()),
                          'repetitionMedians': repetitions})
    write(ROOT / 'summary.json', summaries)
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'cells': 36, 'queryRows': 3456, 'pairedComparisons': 6912,
            'developmentGatePass': all(case['gatePass'] for case in summaries),
            'rustcVersion': version,
            'qualification': False, 'productionChanged': False, 'freezeSha256': digest(ROOT / 'freeze.json'),
            'outputs': {name: digest(ROOT / name) for name in ('summary.json', 'native.log', 'output/queries.ndjson', 'prerequisite.json')}}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
