"""Finish Swift host qualification after correcting the macOS strip setting."""
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
import xml.etree.ElementTree as ET

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-mobile-host-qualification-20261103-v2')
PRIOR = Path('/Volumes/A/.hms-bench-cache/warm-mobile-host-qualification-20261103-v1')
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV.update({'CARGO_TARGET_DIR': '/Users/davidcondrey/.cache/warm-mobile-host-qualification-target/jvm-host',
            'CARGO_BUILD_JOBS': '2', 'RAYON_NUM_THREADS': '2', 'SLICES': 'macos'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def run(name, command, cwd, timeout=7200):
    with (ROOT / (name + '.log')).open('xb') as stream:
        process = subprocess.Popen([str(value) for value in command], cwd=cwd, env=ENV,
                                   stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            if process.wait(timeout=timeout):
                raise RuntimeError(name + ' failed; log retained')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)


def normalized(path):
    return '\n'.join(line.rstrip() for line in path.read_text().splitlines())


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    previous = json.loads((PRIOR / 'freeze.json').read_text())
    def verify():
        if digest(PRIOR / 'freeze.json') != freeze['priorFreezeSha256']:
            raise ValueError('prior source receipt changed')
        for directory, files in ((ROOT / 'source', freeze['files']), (PRIOR / 'source', previous['files']),
                                 (PRIOR, freeze['reusedArtifacts'])):
            for name, expected in files.items():
                if digest(directory / name) != expected:
                    raise ValueError('frozen source or reused evidence changed: ' + name)
        if digest(ROOT / 'run.py') != freeze['runnerSha256']:
            raise ValueError('runner changed')
    verify()
    changed = {name for name in freeze['files'].keys() | previous['files'].keys()
               if freeze['files'].get(name) != previous['files'].get(name)}
    if changed != {'bindings/swift/scripts/build-xcframework.sh'}:
        raise ValueError('prior UniFFI/Kotlin results do not cover these source changes')
    tests = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', (PRIOR / 'uniffi-tests.log').read_text())
    if sum(int(row[0]) for row in tests) != 18 or any(int(row[1]) or int(row[2]) for row in tests):
        raise ValueError('prior UniFFI tests did not pass')
    report = ET.parse(PRIOR / 'jvm-consumer/build/test-results/test/TEST-WarmStoreTest.xml').getroot()
    if {key: int(report.get(key, '0')) for key in ('tests', 'failures', 'errors', 'skipped')} != dict(tests=7, failures=0, errors=0, skipped=0):
        raise ValueError('prior fresh Kotlin consumer did not pass')
    for variant in ('jvm', 'android'):
        generated = PRIOR / ('generated-' + variant + '/com/writerslogic/warm/warm_mobile.kt')
        if normalized(generated) != normalized(ROOT / ('source/bindings/kotlin/' + variant + '/src/main/kotlin/com/writerslogic/warm/warm_mobile.kt')):
            raise ValueError('prior Kotlin wrapper does not match')
    work = ROOT / 'work'
    shutil.copytree(ROOT / 'source', work)
    for name, command, expected in [('rust', ['rustc', '--version'], 'rustc 1.96.0'),
                                     ('swift', ['swift', '--version'], 'Swift version 6.4.2')]:
        run('version-' + name, command, work, timeout=60)
        if expected not in (ROOT / ('version-' + name + '.log')).read_text():
            raise ValueError('toolchain changed: ' + name)
    run('swift-xcframework', ['bash', work / 'bindings/swift/scripts/build-xcframework.sh'], work)
    swift = work / 'bindings/swift'
    if normalized(swift / 'Sources/WarmMemory/WarmMobile.swift') != normalized(ROOT / 'source/bindings/swift/Sources/WarmMemory/WarmMobile.swift'):
        raise ValueError('generated Swift interface changed')
    package = ROOT / 'warm-swift-macos.tar.gz'
    with tarfile.open(package, 'w:gz') as packed:
        for name in ('Package.swift', 'Sources', 'Frameworks'):
            packed.add(swift / name, arcname=name)
    installed = ROOT / 'swift-consumer'
    installed.mkdir()
    with tarfile.open(package) as packed:
        packed.extractall(installed, filter='data')
    shutil.copytree(ROOT / 'source/bindings/swift/Tests', installed / 'Tests')
    run('swift-consumer-tests', ['swift', 'test', '--jobs', '2'], installed)
    log = (ROOT / 'swift-consumer-tests.log').read_text()
    if not re.search(r'Executed 7 tests, with 0 failures', log) or re.search(r'\b[1-9]\d* tests? skipped\b', log):
        raise ValueError('Swift consumer did not pass all seven tests')
    for name in ('bindings/uniffi/Cargo.lock', 'bindings/uniffi/bindgen/Cargo.lock'):
        if digest(work / name) != freeze['files'][name]:
            raise ValueError('build changed a frozen Cargo lock')
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(ROOT / 'freeze.json'), 'uniffiTests': 18, 'kotlinTests': 7,
            'swiftTests': 7, 'reusedPriorUniFFIAndKotlinEvidence': True,
            'jvmArchiveSha256': digest(PRIOR / 'jvm-consumer/libs/warm-jvm.jar'),
            'swiftArchiveSha256': digest(package),
            'swiftInstalledFiles': {str(p.relative_to(installed)): digest(p) for p in sorted(installed.rglob('*'))
                                    if p.is_file() and '.build' not in p.relative_to(installed).parts},
            'logs': {p.name: digest(p) for p in sorted(ROOT.glob('*.log'))},
            'scope': 'macOS arm64 host lifecycle; other platforms, legacy migration and process-death recovery remain open'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
