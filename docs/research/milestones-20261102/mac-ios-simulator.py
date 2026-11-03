"""Qualify a fresh source-built Swift consumer on an owned iOS simulator."""
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

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-ios-simulator-qualification-20261103-v1')
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV.update({'CARGO_TARGET_DIR': '/Users/davidcondrey/.cache/warm-ios-simulator-qualification-target',
            'CARGO_BUILD_JOBS': '2', 'RAYON_NUM_THREADS': '2', 'SLICES': 'ios-sim'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def run(name, command, cwd, timeout=7200):
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


def normalized(path):
    return '\n'.join(line.rstrip() for line in path.read_text().splitlines())


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    def verify():
        for name, expected in freeze['files'].items():
            if digest(ROOT / 'source' / name) != expected:
                raise ValueError('frozen source changed: ' + name)
        if digest(ROOT / 'run.py') != freeze['runnerSha256']:
            raise ValueError('frozen runner changed')
    verify()
    work = ROOT / 'work'
    shutil.copytree(ROOT / 'source', work)
    for name, command, expected in [('rust', ['rustc', '--version'], 'rustc 1.96.0'),
                                    ('swift', ['swift', '--version'], 'Swift version 6.4.2')]:
        if expected not in run('version-' + name, command, work, timeout=60):
            raise ValueError('toolchain changed: ' + name)
    run('version-xcode', ['xcodebuild', '-version'], work, timeout=60)
    run('swift-xcframework', ['bash', work / 'bindings/swift/scripts/build-xcframework.sh'], work)
    swift = work / 'bindings/swift'
    if normalized(swift / 'Sources/WarmMemory/WarmMobile.swift') != normalized(ROOT / 'source/bindings/swift/Sources/WarmMemory/WarmMobile.swift'):
        raise ValueError('generated Swift interface changed')
    package = ROOT / 'warm-swift-ios-sim.tar.gz'
    with tarfile.open(package, 'w:gz') as packed:
        for name in ('Package.swift', 'Sources', 'Frameworks'):
            packed.add(swift / name, arcname=name)
    installed = ROOT / 'swift-consumer'
    installed.mkdir()
    with tarfile.open(package) as packed:
        packed.extractall(installed, filter='data')
    shutil.copytree(ROOT / 'source/bindings/swift/Tests', installed / 'Tests')
    installed_files = {str(path.relative_to(installed)): digest(path)
                       for path in sorted(installed.rglob('*')) if path.is_file()}
    runtime = 'com.apple.CoreSimulator.SimRuntime.iOS-27-0'
    runtimes = json.loads(run('simulator-runtimes', ['xcrun', 'simctl', 'list', 'runtimes', '-j'], work, 60))
    if not any(item['identifier'] == runtime and item['isAvailable'] for item in runtimes['runtimes']):
        raise ValueError('declared iOS runtime unavailable')
    simulator = run('simulator-create', ['xcrun', 'simctl', 'create', 'WARMQualification20261103v1',
                    'com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro', runtime], work, 60).strip()
    if not re.fullmatch(r'[A-Fa-f0-9]{8}(?:-[A-Fa-f0-9]{4}){3}-[A-Fa-f0-9]{12}', simulator):
        raise ValueError('invalid owned simulator identity')
    write(ROOT / 'simulator.json', {'udid': simulator, 'runtime': runtime, 'owned': True})
    run('simulator-boot', ['xcrun', 'simctl', 'boot', simulator], work, 120)
    try:
        run('simulator-ready', ['xcrun', 'simctl', 'bootstatus', simulator, '-b'], work, 600)
        run('swift-consumer-tests', ['xcodebuild', '-scheme', 'WarmMemory', '-destination',
            'platform=iOS Simulator,id=' + simulator, '-derivedDataPath', ROOT / 'derived-data',
            '-resultBundlePath', ROOT / 'tests.xcresult', '-parallel-testing-enabled', 'NO',
            '-jobs', '2', 'CODE_SIGNING_ALLOWED=NO', 'test'], installed, 1800)
        summary = json.loads(run('xcresult-summary', ['xcrun', 'xcresulttool', 'get',
            'test-results', 'summary', '--path', ROOT / 'tests.xcresult', '--compact'], work, 60))
        if summary.get('passedTests') != 7 or summary.get('failedTests') != 0 or summary.get('skippedTests') != 0:
            raise ValueError('Swift simulator consumer did not pass exactly seven tests without skips')
        write(ROOT / 'test-summary.json', summary)
    finally:
        run('simulator-shutdown', ['xcrun', 'simctl', 'shutdown', simulator], work, 120)
    for name in ('bindings/uniffi/Cargo.lock', 'bindings/uniffi/bindgen/Cargo.lock'):
        if digest(work / name) != freeze['files'][name]:
            raise ValueError('build changed a frozen Cargo lock')
    for name, expected in installed_files.items():
        if digest(installed / name) != expected:
            raise ValueError('installed consumer changed: ' + name)
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(ROOT / 'freeze.json'), 'swiftTests': 7,
            'swiftArchiveSha256': digest(package), 'installedFiles': installed_files,
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
