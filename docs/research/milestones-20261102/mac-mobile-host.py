"""Check source-built mobile bindings through fresh macOS host consumers."""
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
import time
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-mobile-host-qualification-20261103-v1')
TARGET = Path('/Users/davidcondrey/.cache/warm-mobile-host-qualification-target')
JAVA = Path('/Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home')
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103/evidence/warm-recalibration-audit-v5/finished.json'
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV.update({'JAVA_HOME': str(JAVA), 'PATH': str(JAVA / 'bin') + ':' + ENV['PATH'],
            'ANDROID_HOME': '/Volumes/A/android-sdk', 'LIMA_HOME': '/Volumes/A/.hms-lima',
            'CARGO_TARGET_DIR': str(TARGET), 'CARGO_BUILD_JOBS': '2', 'RAYON_NUM_THREADS': '2'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def run(name, command, cwd, extra=None, timeout=7200):
    with (ROOT / (name + '.log')).open('xb') as stream:
        process = subprocess.Popen([str(value) for value in command], cwd=cwd,
                                   env={**ENV, **(extra or {})}, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            if process.wait(timeout=timeout):
                raise RuntimeError(name + ' failed; retained log: ' + name + '.log')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)


def wait_for_calibration():
    deadline = time.monotonic() + 28800
    while time.monotonic() < deadline:
        check = subprocess.run(['limactl', 'shell', 'warm-arm64', 'test', '-f', GUEST],
                               env=ENV, capture_output=True, timeout=30)
        if check.returncode == 0:
            value = subprocess.check_output(['limactl', 'shell', 'warm-arm64', 'cat', GUEST],
                                            env=ENV, timeout=30)
            write(ROOT / 'prior-audit-terminal.json', json.loads(value))
            return
        if check.returncode != 1:
            raise RuntimeError('cannot inspect serving-audit completion')
        time.sleep(30)
    raise TimeoutError('serving audit did not finish within eight hours')


def junit(path):
    totals = dict(tests=0, failures=0, errors=0, skipped=0)
    for report in path.glob('TEST-*.xml'):
        suite = ET.parse(report).getroot()
        for key in totals:
            totals[key] += int(suite.get(key, '0'))
    if totals != dict(tests=7, failures=0, errors=0, skipped=0):
        raise ValueError('Kotlin consumer coverage differs: ' + str(totals))
    return totals


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
    wait_for_calibration()
    work = ROOT / 'work'
    shutil.copytree(ROOT / 'source', work)
    for name, command, expected in [('rust', ['rustc', '--version'], 'rustc 1.96.0'),
                                     ('java', [JAVA / 'bin/java', '--version'], '17.0.19'),
                                     ('swift', ['swift', '--version'], 'Swift version 6.4.2')]:
        run('version-' + name, command, work, timeout=60)
        if expected not in (ROOT / ('version-' + name + '.log')).read_text():
            raise ValueError('unexpected ' + name + ' toolchain')
    crate = work / 'bindings/uniffi/Cargo.toml'
    run('uniffi-tests', ['cargo', 'test', '--locked', '--manifest-path', crate, '--', '--test-threads=2'], work)
    rows = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', (ROOT / 'uniffi-tests.log').read_text())
    if sum(int(row[0]) for row in rows) != 18 or any(int(row[1]) or int(row[2]) for row in rows):
        raise ValueError('UniFFI test coverage differs from the declared 18 cases')
    kotlin = work / 'bindings/kotlin'
    run('gradle-wrapper', ['bash', kotlin / 'scripts/bootstrap-gradle-wrapper.sh'], kotlin)
    run('jvm-native', ['bash', kotlin / 'scripts/build-jvm-native.sh'], kotlin)
    library = kotlin / 'jvm/build/natives/darwin-aarch64/libwarm_mobile.dylib'
    interfaces = {}
    for variant in ('jvm', 'android'):
        output = ROOT / ('generated-' + variant)
        run('generate-' + variant, ['bash', work / 'bindings/uniffi/scripts/generate-bindings.sh',
            'kotlin', output, kotlin / ('uniffi-' + variant + '.toml')], work, {'LIBRARY': str(library)})
        generated = output / 'com/writerslogic/warm/warm_mobile.kt'
        committed = kotlin / variant / 'src/main/kotlin/com/writerslogic/warm/warm_mobile.kt'
        if normalized(generated) != normalized(committed):
            raise ValueError('generated Kotlin interface changed: ' + variant)
        interfaces[variant] = hashlib.sha256(normalized(generated).encode()).hexdigest()
    gradle = [kotlin / 'gradlew', '--no-daemon', '--max-workers=2']
    run('jvm-jar', gradle + [':jvm:jar'], kotlin)
    jars = list((kotlin / 'jvm/build/libs').glob('*.jar'))
    if len(jars) != 1:
        raise ValueError('unexpected JVM artifact set')
    with zipfile.ZipFile(jars[0]) as packed:
        if hashlib.sha256(packed.read('darwin-aarch64/libwarm_mobile.dylib')).hexdigest() != digest(library):
            raise ValueError('JVM archive omitted or changed its native library')
    consumer = ROOT / 'jvm-consumer'
    (consumer / 'libs').mkdir(parents=True)
    shutil.copyfile(jars[0], consumer / 'libs/warm-jvm.jar')
    shutil.copytree(kotlin / 'jvm/src/test', consumer / 'src/test')
    (consumer / 'settings.gradle.kts').write_text('rootProject.name = "warm-qualified-consumer"\n')
    (consumer / 'build.gradle.kts').write_text('''plugins { kotlin("jvm") version "2.2.21" }
repositories { mavenCentral() }
kotlin { jvmToolchain(17) }
dependencyLocking { lockAllConfigurations() }
dependencies {
    implementation(files("libs/warm-jvm.jar"))
    implementation("net.java.dev.jna:jna:5.19.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
    testImplementation(kotlin("test"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}
tasks.test { useJUnitPlatform() }
''')
    run('consumer-dependencies', gradle + ['-p', consumer, 'dependencies', '--write-locks'], consumer)
    run('consumer-tests', gradle + ['-p', consumer, 'test'], consumer)
    kotlin_tests = junit(consumer / 'build/test-results/test')
    run('swift-xcframework', ['bash', work / 'bindings/swift/scripts/build-xcframework.sh'], work, {'SLICES': 'macos'})
    swift = work / 'bindings/swift'
    if normalized(swift / 'Sources/WarmMemory/WarmMobile.swift') != normalized(ROOT / 'source/bindings/swift/Sources/WarmMemory/WarmMobile.swift'):
        raise ValueError('generated Swift interface changed')
    interfaces['swift'] = hashlib.sha256(normalized(swift / 'Sources/WarmMemory/WarmMobile.swift').encode()).hexdigest()
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
    swift_log = (ROOT / 'swift-consumer-tests.log').read_text()
    if not re.search(r'Executed 7 tests, with 0 failures', swift_log) or re.search(r'\b[1-9]\d* tests? skipped\b', swift_log):
        raise ValueError('Swift consumer did not pass every declared test')
    for name in ('bindings/uniffi/Cargo.lock', 'bindings/uniffi/bindgen/Cargo.lock'):
        if digest(work / name) != freeze['files'][name]:
            raise ValueError('build changed a frozen Cargo lock')
    verify()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(ROOT / 'freeze.json'), 'uniffiTests': 18,
            'kotlinTests': kotlin_tests, 'swiftTests': 7, 'interfaces': interfaces,
            'jvmArchiveSha256': digest(consumer / 'libs/warm-jvm.jar'),
            'swiftArchiveSha256': digest(package), 'nativeLibrarySha256': digest(library),
            'logs': {path.name: digest(path) for path in sorted(ROOT.glob('*.log'))},
            'scope': 'macOS arm64 host lifecycle; no mobile-device, migration or crash-recovery inference'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
