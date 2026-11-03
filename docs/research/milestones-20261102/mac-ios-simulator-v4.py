"""Retry the verified iOS package with simulator-readable internal build products."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tarfile

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-ios-simulator-qualification-20261103-v4')
PRIOR = Path('/Volumes/A/.hms-bench-cache/warm-ios-simulator-qualification-20261103-v3')
INSTALLED = Path('/Users/davidcondrey/.cache/warm-ios-simulator-consumer-v4')
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def run(name, command, cwd, timeout=600):
    path = ROOT / (name + '.log')
    with path.open('xb') as stream:
        process = subprocess.Popen([str(value) for value in command], cwd=cwd, env=ENV,
                                   stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            if process.wait(timeout=timeout):
                raise RuntimeError(name + ' failed; log retained')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    return path.read_text()


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    def verify():
        if digest(PRIOR / 'freeze.json') != freeze['priorFreezeSha256']:
            raise ValueError('prior freeze changed')
        for directory, files in ((PRIOR / 'source', freeze['files']), (PRIOR, freeze['priorArtifacts'])):
            for name, expected in files.items():
                if digest(directory / name) != expected:
                    raise ValueError('frozen source or package changed: ' + name)
        if digest(ROOT / 'run.py') != freeze['runnerSha256']:
            raise ValueError('runner changed')
    verify()
    prior_result = json.loads((PRIOR / 'finished.json').read_text())
    if prior_result.get('complete') is not False:
        raise ValueError('expected the recorded prior simulator launch failure')
    old_simulator = json.loads((PRIOR / 'simulator.json').read_text())['udid']
    devices = json.loads(run('prior-simulator-state', ['xcrun', 'simctl', 'list', 'devices', '-j'], ROOT, 60))
    if any(item['udid'] == old_simulator and item['state'] != 'Shutdown'
           for values in devices['devices'].values() for item in values):
        raise ValueError('prior owned simulator has not shut down')
    package = ROOT / 'warm-swift-ios-sim.tar.gz'
    shutil.copyfile(PRIOR / package.name, package)
    INSTALLED.mkdir(exist_ok=False)
    with tarfile.open(package) as packed:
        packed.extractall(INSTALLED, filter='data')
    shutil.copytree(PRIOR / 'source/bindings/swift/Tests', INSTALLED / 'Tests')
    files = {str(path.relative_to(INSTALLED)): digest(path)
             for path in sorted(INSTALLED.rglob('*')) if path.is_file()}
    write(ROOT / 'installed-files.json', files)
    runtime = 'com.apple.CoreSimulator.SimRuntime.iOS-27-0'
    simulator = run('simulator-create', ['xcrun', 'simctl', 'create', 'WARMQualification20261103v4',
                    'com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro', runtime], ROOT, 60).strip()
    if not re.fullmatch(r'[A-Fa-f0-9]{8}(?:-[A-Fa-f0-9]{4}){3}-[A-Fa-f0-9]{12}', simulator):
        raise ValueError('invalid owned simulator identity')
    write(ROOT / 'simulator.json', {'udid': simulator, 'runtime': runtime, 'owned': True})
    run('simulator-boot', ['xcrun', 'simctl', 'boot', simulator], ROOT, 120)
    try:
        run('simulator-ready', ['xcrun', 'simctl', 'bootstatus', simulator, '-b'], ROOT)
        run('swift-consumer-tests', ['xcodebuild', '-scheme', 'WarmMemory', '-destination',
            'platform=iOS Simulator,id=' + simulator, '-derivedDataPath', ROOT / 'derived-data',
            '-resultBundlePath', ROOT / 'tests.xcresult', '-parallel-testing-enabled', 'NO',
            '-jobs', '2', 'CODE_SIGNING_ALLOWED=NO', 'SYMROOT=' + str(INSTALLED / 'products'), 'test'], INSTALLED, 900)
        summary = json.loads(run('xcresult-summary', ['xcrun', 'xcresulttool', 'get',
            'test-results', 'summary', '--path', ROOT / 'tests.xcresult', '--compact'], ROOT, 60))
        expected = {'totalTestCount': 7, 'passedTests': 7, 'failedTests': 0, 'skippedTests': 0, 'expectedFailures': 0}
        if any(summary.get(key) != value for key, value in expected.items()):
            raise ValueError('Swift simulator consumer did not pass exactly seven tests without skips')
        write(ROOT / 'test-summary.json', summary)
    finally:
        run('simulator-shutdown', ['xcrun', 'simctl', 'shutdown', simulator], ROOT, 120)
    for name, expected in files.items():
        if digest(INSTALLED / name) != expected:
            raise ValueError('installed consumer changed: ' + name)
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(ROOT / 'freeze.json'), 'swiftTests': 7,
            'swiftArchiveSha256': digest(package), 'installed': str(INSTALLED),
            'installedFilesSha256': digest(ROOT / 'installed-files.json'),
            'simulator': simulator, 'runtime': runtime,
            'logs': {path.name: digest(path) for path in sorted(ROOT.glob('*.log'))},
            'testSummarySha256': digest(ROOT / 'test-summary.json'),
            'scope': 'iOS 27.0 arm64 simulator lifecycle only; no physical-device, legacy migration or process-death claim'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
