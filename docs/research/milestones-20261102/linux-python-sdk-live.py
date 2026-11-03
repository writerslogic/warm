"""Install frozen SDK wheels and run live lifecycle/recovery after calibration."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import xml.etree.ElementTree as ET


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def run(command, log, cwd, environment, timeout=900):
    with log.open('xb') as stream:
        child = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT,
                                 cwd=cwd, env=environment, start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
            if code != 0:
                raise RuntimeError(f'{log.name} exited with status {code}')
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=30)


def verify_source(root, source):
    for name, expected in source['files'].items():
        if digest(root / name) != expected:
            raise RuntimeError('frozen SDK qualification source changed: ' + name)


def main(args):
    root = Path(__file__).resolve().parent
    source = json.loads((root / 'source.json').read_text())
    verify_source(root, source)
    deadline = time.monotonic() + 86400
    while not args.wait_for.is_file():
        if time.monotonic() > deadline:
            raise TimeoutError('calibration audit did not finish within twenty-four hours')
        time.sleep(60)
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = (entry / 'cmdline').read_bytes()
        except FileNotFoundError:
            continue
        if b'benchmarks/public/matched_server/calibrate.py' in command:
            raise RuntimeError('calibration process remains active')
    if digest(args.server) != source['serverSha256']:
        raise RuntimeError('server differs from the qualified-source freeze')
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(('WARM_', 'HMS_', 'PYTHON')) and key.upper() not in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY')}
    environment.update({'WARM_SERVER_BIN': str(args.server), 'RAYON_NUM_THREADS': '2', 'NO_PROXY': '127.0.0.1,localhost'})
    consumers = []
    for tag, python in (('311', '3.11.14'), ('314', '3.14.7')):
        consumer = root / ('consumer-' + tag)
        result = args.output / ('consumer-' + tag)
        result.mkdir()
        run([str(args.uv), 'sync', '--project', str(consumer), '--locked', '--python', python],
            result / 'install.log', consumer, environment)
        executable = consumer / '.venv/bin/python'
        run([str(executable), '-I', str(root / 'check_installed.py'), str(root / 'dist/warm_sdk-0.1.3-py3-none-any.whl'),
             str(result / 'installed.json'), python], result / 'installed.log', consumer, environment)
        run([str(executable), '-I', '-m', 'pytest', '-q', '-rs', '-p', 'no:cacheprovider',
             '--junitxml=' + str(result / 'live.xml'), str(root / 'tests')], result / 'live.log', consumer, environment)
        suites = list(ET.parse(result / 'live.xml').getroot().iter('testsuite'))
        counts = {name: sum(int(suite.attrib.get(name, 0)) for suite in suites)
                  for name in ('tests', 'failures', 'errors', 'skipped')}
        if counts != {'tests': 12, 'failures': 0, 'errors': 0, 'skipped': 0}:
            raise RuntimeError('live SDK suite did not pass all twelve cases without skips')
        run([str(executable), '-I', str(root / 'python_sdk_recovery.py'), '--binary', str(args.server),
             '--output', str(result / 'recovery')], result / 'recovery.log', consumer, environment)
        recovery = json.loads((result / 'recovery/finished.json').read_text())
        if recovery['complete'] is not True or len(recovery['cases']) != 2:
            raise RuntimeError('both recovery cases must complete')
        consumers.append({'python': python, 'live': counts, 'recoveryCases': 2,
                          'installed': json.loads((result / 'installed.json').read_text()),
                          'lockSha256': digest(consumer / 'uv.lock')})
    verify_source(root, source)
    if digest(args.server) != source['serverSha256']:
        raise RuntimeError('server changed during the SDK checks')
    return {'complete': True, 'scope': 'Linux arm64 installed HTTP SDK, live temporal/document APIs and two process-recovery cases per interpreter',
            'consumers': consumers, 'sourceSha256': digest(root / 'source.json'), 'benchmarkClaim': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--uv', required=True, type=Path)
    parser.add_argument('--server', required=True, type=Path)
    parser.add_argument('--wait-for', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'Driver did not finish'}}
    try:
        result = main(args)
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(args.output / 'finished.json', result)
