"""Run the sealed comparison with existing host and guest observations."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-serving-qualification-20261103-v1')
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103/serving-qualification-v1'
HOST_RAW = Path('/Users/davidcondrey/.cache/warm-serving-isolation-20261103-v1')
GUEST_RAW = Path('/Users/davidcondrey/.cache/warm-serving-guest-isolation-20261103-v1')
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV['LIMA_HOME'] = '/Volumes/A/.hms-lima'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def run(name, args, timeout=120):
    with (ROOT / (name + '.log')).open('xb') as log:
        return subprocess.run(args, env=ENV, stdin=subprocess.DEVNULL,
                              stdout=log, stderr=subprocess.STDOUT, timeout=timeout)


def main():
    sealed = json.loads((ROOT / 'sealed.json').read_text())
    if not sealed['complete'] or digest(ROOT / 'configuration-seal.json') != sealed['configurationSealSha256']:
        raise ValueError('configuration seal changed or incomplete')
    core = Path('/Volumes/A/.hms-bench-cache/warm-core-qualification-20261103-v2/finished.json')
    dspy = Path('/Volumes/A/.hms-bench-cache/warm-dspy-qualification-20261103-v2/finished.json')
    if not all(json.loads(path.read_text())['complete'] for path in (core, dspy)):
        raise ValueError('functional qualification incomplete')
    prelude = json.loads((ROOT / 'prelude-reviewed.json').read_text())
    if prelude.get('readyToStart') is not True or prelude['configurationSealSha256'] != sealed['configurationSealSha256']:
        raise ValueError('physical-host prelude has not been reviewed')
    for folder in (HOST_RAW, GUEST_RAW):
        if time.time() - (folder / 'samples.ndjson').stat().st_mtime > 15:
            raise ValueError('observer is stale: ' + str(folder))
    inhibit = subprocess.Popen(['/usr/bin/caffeinate', '-isu', '-t', '86400', '-w', str(os.getpid())],
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    terminal_received = False
    try:
        run('transfer-host-admission', ['limactl', 'copy', str(ROOT / 'host-observation-started.json'),
                                      'warm-arm64:' + GUEST + '/host-observation-started.json']).check_returncode()
        write(ROOT / 'execution-started.json', {
            'startedAt': datetime.now(timezone.utc).isoformat(), 'pid': os.getpid(),
            'configurationSealSha256': sealed['configurationSealSha256'],
            'launcherSha256': digest(Path(__file__)), 'preludeSha256': digest(ROOT / 'prelude-reviewed.json'),
            'isolationPassed': False, 'claimQualified': False})
        completed = run('execution', ['limactl', 'shell', 'warm-arm64', '--', 'python3',
                                     GUEST + '/linux-serving-qualification.py'], timeout=82800)
        run('fetch-execution-finished', ['limactl', 'copy', 'warm-arm64:' + GUEST + '/finished.json',
                                        str(ROOT / 'finished.json')]).check_returncode()
        receipt = json.loads((ROOT / 'finished.json').read_text())
        terminal_received = True
        if completed.returncode or receipt.get('complete') is not True:
            raise RuntimeError('serving queue failed; raw evidence retained')
        return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
                'finishedSha256': digest(ROOT / 'finished.json'), 'claimQualified': False,
                'pending': ['Raw serving audits', 'Whole-run host/guest isolation review',
                            'Actual settings and resource review', 'Second physical host', 'Comparison gates']}
    finally:
        try:
            if terminal_received:
                try:
                    stop = run('stop-guest-observer', ['limactl', 'shell', 'warm-arm64', '--', 'touch',
                                                     GUEST + '/stop-guest-observation'])
                    write(ROOT / 'observer-stop-requested.json', {
                        'requestedAt': datetime.now(timezone.utc).isoformat(), 'guestExitCode': stop.returncode})
                    stop.check_returncode()
                finally:
                    (ROOT / 'stop-host-observation').touch(exist_ok=False)
            else:
                write(ROOT / 'observer-left-running.json', {
                    'recordedAt': datetime.now(timezone.utc).isoformat(),
                    'reason': 'No guest terminal receipt; remote execution must be inspected before stopping observations.'})
        finally:
            if inhibit.poll() is None:
                inhibit.terminate()
            inhibit.wait(timeout=30)


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'execution-launcher-finished.json', result)
