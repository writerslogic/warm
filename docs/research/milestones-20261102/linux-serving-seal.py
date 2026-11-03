"""Check the exact serving artifact and seal selections without held-out vectors."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess

BASE = Path('/home/davidcondrey.guest/warm-milestones-20261103')
ROOT = BASE / 'serving-qualification-v1'
SOURCE = ROOT / 'source'
INPUTS = BASE / 'performance-calibration-inputs'
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV.update({'WARM_BENCH_WARM_SERVER': str(BASE / 'dense-product-v2/warm-server'),
            'WARM_BENCH_PG_BIN': str(BASE / 'comparators/postgres/bin'),
            'WARM_MATCHED_OPENSEARCH_HOME': str(BASE / 'comparators/opensearch-3.9.0'),
            'WARM_MATCHED_OPENSEARCH_ARCHIVE': str(BASE / 'comparators/downloads/opensearch-3.9.0-linux-arm64.tar.gz'),
            'OPENBLAS_NUM_THREADS': '1', 'OMP_NUM_THREADS': '1'})
PYTHON = [str(BASE / 'tools/uv/bin/uv'), 'run', '--project',
          'benchmarks/public/matched_server', '--frozen', '--python', '3.12.12', 'python']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def verify():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    for name, expected in freeze['files'].items():
        if digest(SOURCE / name) != expected:
            raise ValueError('frozen source changed: ' + name)
    for name, expected in freeze['artifacts'].items():
        if digest(ROOT / name) != expected:
            raise ValueError('frozen artifact changed: ' + name)
    if digest(Path(ENV['WARM_BENCH_WARM_SERVER'])) != freeze['warmBinarySha256']:
        raise ValueError('serving binary changed')
    return freeze


def run(name, arguments, timeout=7200):
    with (ROOT / (name + '.log')).open('xb') as log:
        process = subprocess.Popen(arguments, cwd=SOURCE, env=ENV,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    if code:
        raise RuntimeError(name + ' failed with exit ' + str(code))


def main():
    verify()
    if any(INPUTS.glob('*/test.f32')):
        raise ValueError('preflight inputs unexpectedly contain held-out vectors')
    run('warm-recovery', ['taskset', '-c', '0-3', *PYTHON,
        'benchmarks/public/matched_server/check_adapters.py', '--engine', 'warm',
        '--output', str(ROOT / 'warm-recovery')])
    recovery = json.loads((ROOT / 'warm-recovery/finished.json').read_text())
    if recovery.get('complete') is not True or len(recovery['cells']) != 16:
        raise ValueError('exact-artifact recovery check incomplete')
    if not all(row['pass'] for row in recovery['cells']):
        raise ValueError('exact-artifact synthetic quality failed')
    mapping = {}
    for dataset in ('coco-i2i-512-angular', 'deep-image-96-angular'):
        mapping[dataset] = {
            engine: str(BASE / 'evidence' / ('calibration-' + engine + '-' + dataset + '-v' + version))
            for engine, version in [('warm', '5'), ('pgvector', '1' if dataset.startswith('coco') else '2'),
                                    ('opensearch', '2')]}
    write(ROOT / 'calibrations.json', mapping)
    run('seal', [*PYTHON, 'benchmarks/public/matched_server/seal_qualification.py',
        '--calibrations', str(ROOT / 'calibrations.json'), '--inputs', str(INPUTS),
        '--output', str(ROOT / 'selection'), '--hardware', 'Apple M4 / Linux arm64 Lima'])
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'heldOutVectorsOpened': False, 'freezeSha256': digest(ROOT / 'freeze.json'),
            'configurationSealSha256': digest(ROOT / 'selection/configuration-seal.json'),
            'recoverySha256': digest(ROOT / 'warm-recovery/finished.json'),
            'scope': 'Exact-artifact recovery and selection/component seal; performance and isolation pending.'}


if __name__ == '__main__':
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'sealed.json', result)
