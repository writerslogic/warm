"""Recalibrate only the changed WARM artifact on the pre-exposed query cohort."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tarfile

ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103')
DRIVER = ROOT / 'matched-driver-v7'
BINARY = ROOT / 'dense-product-v2/warm-server'
OUTPUT = ROOT / 'evidence/warm-recalibration-v3'
ENV = {name: value for name, value in os.environ.items() if name in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV.update({'WARM_BENCH_WARM_SERVER': str(BINARY), 'OPENBLAS_NUM_THREADS': '1', 'OMP_NUM_THREADS': '1'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    OUTPUT.mkdir()
    archive = ROOT / 'driver-v7.tar'
    if digest(archive) != 'fba17b05dc9c207c7acf896c2e7f35cda844754f7006fb008175a35a5639ac50':
        raise ValueError('frozen driver archive changed')
    files = {}
    with tarfile.open(archive) as packed:
        for member in packed.getmembers():
            if not member.isfile():
                continue
            name = Path(member.name)
            if name.is_absolute() or '..' in name.parts:
                raise ValueError('invalid driver member')
            files[str(name)] = hashlib.sha256(packed.extractfile(member).read()).hexdigest()
    def verify():
        if digest(BINARY) != 'b61f4e70bcddb3673ef551ea921903124d364b6fb34dc48bb01af8642bd58709':
            raise ValueError('qualified development server changed')
        for name, expected in files.items():
            if digest(DRIVER / name) != expected:
                raise ValueError('frozen calibration driver changed: ' + name)
    verify()
    cases = []
    for dataset in ('coco-i2i-512-angular', 'deep-image-96-angular'):
        output = ROOT / ('evidence/calibration-warm-' + dataset + '-v3')
        command = ['taskset', '-c', '0-3', str(ROOT / 'tools/uv/bin/uv'), 'run', '--project',
                   'benchmarks/public/matched_server', '--frozen', 'python',
                   'benchmarks/public/matched_server/calibrate.py', '--engine', 'warm', '--dataset', dataset,
                   '--inputs', str(ROOT / 'performance-calibration-inputs'), '--output', str(output),
                   '--installation-evidence', str(ROOT / 'comparators/evidence')]
        with (OUTPUT / (dataset + '.log')).open('xb') as stream:
            process = subprocess.Popen(command, cwd=DRIVER, env=ENV, stdout=stream,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=14400)
                if code:
                    raise RuntimeError(dataset + ' calibration exited ' + str(code))
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=30)
        selection = json.loads((output / 'selection.json').read_text())
        if selection.get('complete') is not True or selection.get('eligible') is not True:
            raise ValueError(dataset + ' has no eligible selection')
        cases.append({'dataset': dataset, 'selection': str(output / 'selection.json'),
                      'selectionSha256': digest(output / 'selection.json')})
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(), 'cases': cases,
            'binarySha256': digest(BINARY), 'driverArchiveSha256': digest(archive)}


if __name__ == '__main__':
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        OUTPUT.mkdir(exist_ok=True)
        with (OUTPUT / 'finished.json').open('x') as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write('\n')
