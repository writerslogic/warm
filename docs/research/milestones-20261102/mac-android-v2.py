"""Qualify extracted Kotlin artifacts and an owned Android emulator consumer."""
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import time
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-android-qualification-20261103-v2')
PRIOR = Path('/Volumes/A/.hms-bench-cache/warm-android-qualification-20261103-v1')
SDK = Path('/Volumes/A/android-sdk')
JAVA = Path('/Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home')
APP = 'com.writerslogic.warm.qualification'
SERIAL = 'emulator-5576'
AVD = 'warm-qualified-20261103-v2'
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV.update({'JAVA_HOME': str(JAVA), 'PATH': str(JAVA / 'bin') + ':' + ENV['PATH'],
            'ANDROID_HOME': str(SDK), 'ANDROID_NDK_HOME': str(SDK / 'ndk/27.3.13750724'),
            'ANDROID_AVD_HOME': str(ROOT / 'avd'), 'ANDROID_SERIAL': SERIAL,
            'CARGO_TARGET_DIR': str(ROOT / 'target'), 'CARGO_BUILD_JOBS': '2', 'RAYON_NUM_THREADS': '2'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def run(name, command, cwd=ROOT, timeout=7200, input_text=None, extra=None):
    with (ROOT / (name + '.log')).open('xb') as log:
        process = subprocess.Popen([str(item) for item in command], cwd=cwd, env={**ENV, **(extra or {})},
                                   stdin=subprocess.PIPE if input_text is not None else subprocess.DEVNULL,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            process.communicate(input=None if input_text is None else input_text.encode(), timeout=timeout)
            if process.returncode:
                raise RuntimeError(name + ' failed; retained log: ' + name + '.log')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)


def adb(*arguments, check=True, timeout=30):
    return subprocess.run([str(SDK / 'platform-tools/adb'), '-s', SERIAL, *arguments], env=ENV,
                          capture_output=True, check=check, timeout=timeout)


def instrument(name, classes, mode=None):
    args = ['shell', 'am', 'instrument', '-w', '-r', '-e', 'class', classes]
    if mode is not None:
        args += ['-e', 'case', mode]
    command = [SDK / 'platform-tools/adb', '-s', SERIAL, *args,
               APP + '.test/androidx.test.runner.AndroidJUnitRunner']
    run(name, command, timeout=900)
    return (ROOT / (name + '.log')).read_text()


def passed(log, count):
    if not re.search(r'OK \(' + str(count) + r' tests?\)', log) or any(
            marker in log for marker in ('FAILURES!!!', 'INSTRUMENTATION_FAILED', 'INSTRUMENTATION_ABORTED',
                                         'INSTRUMENTATION_STATUS_CODE: -3', 'INSTRUMENTATION_STATUS_CODE: -4')):
        raise ValueError('instrumentation did not pass the exact declared cohort')


def android_consumer(kotlin):
    gradle = [kotlin / 'gradlew', '--no-daemon', '--max-workers=2']
    built_kotlin = PRIOR / 'work/bindings/kotlin'
    archives = list((built_kotlin / 'android/build/outputs/aar').glob('*-release.aar'))
    (ROOT / 'independent-strip').mkdir()
    if len(archives) != 1:
        raise ValueError('unexpected Android archive set')
    native = {}
    with zipfile.ZipFile(archives[0]) as packed:
        expected = {'jni/' + abi + '/libwarm_mobile.so' for abi in ('arm64-v8a', 'armeabi-v7a', 'x86_64', 'x86')}
        if {name for name in packed.namelist() if name.endswith('/libwarm_mobile.so')} != expected:
            raise ValueError('Android ABI members differ')
        for name in sorted(expected):
            relative = name.removeprefix('jni/')
            built = built_kotlin / 'android/src/main/jniLibs' / relative
            processed = built_kotlin / 'android/build/intermediates/stripped_native_libs/release/stripReleaseDebugSymbols/out/lib' / relative
            independent = ROOT / 'independent-strip' / (Path(relative).parts[0] + '.so')
            strip = SDK / 'ndk/27.3.13750724/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-strip'
            run('strip-' + Path(relative).parts[0], [strip, '--strip-unneeded', '-o', independent, built], timeout=60)
            packaged = hashlib.sha256(packed.read(name)).hexdigest()
            if packaged != digest(independent) or packaged != digest(processed):
                raise ValueError('AAR member differs from the verified stripping transformation: ' + name)
            native[name] = {'compiledSha256': digest(built), 'packagedSha256': packaged,
                            'independentStripSha256': digest(independent)}
    consumer = ROOT / 'android-consumer'
    (consumer / 'libs').mkdir(parents=True)
    shutil.copyfile(archives[0], consumer / 'libs/warm-android.aar')
    shutil.copyfile(kotlin / 'gradle.properties', consumer / 'gradle.properties')
    (consumer / 'settings.gradle.kts').write_text('''pluginManagement { repositories { google(); mavenCentral(); gradlePluginPortal() } }
dependencyResolutionManagement { repositories { google(); mavenCentral() } }
rootProject.name = "warm-qualified-android"
''')
    (consumer / 'build.gradle.kts').write_text('''plugins {
    id("com.android.application") version "8.13.2"
    id("org.jetbrains.kotlin.android") version "2.2.21"
}
android {
    namespace = "com.writerslogic.warm.qualification"
    compileSdk = 35
    defaultConfig {
        applicationId = "com.writerslogic.warm.qualification"
        minSdk = 24
        targetSdk = 35
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}
kotlin { jvmToolchain(17) }
dependencyLocking { lockAllConfigurations() }
dependencies {
    implementation(files("libs/warm-android.aar"))
    implementation("net.java.dev.jna:jna:5.19.1@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.annotation:annotation:1.9.1")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("org.jetbrains.kotlin:kotlin-test:2.2.21")
    androidTestImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}
''')
    (consumer / 'src/main').mkdir(parents=True)
    (consumer / 'src/main/AndroidManifest.xml').write_text('<manifest xmlns:android="http://schemas.android.com/apk/res/android"><application android:label="WARM Qualification"/></manifest>\n')
    tests = consumer / 'src/androidTest/kotlin'
    tests.mkdir(parents=True)
    jvm_test = (kotlin / 'jvm/src/test/kotlin/WarmStoreTest.kt').read_text()
    jvm_test = jvm_test.replace('import kotlin.test.AfterTest', 'import org.junit.After as AfterTest')
    jvm_test = jvm_test.replace('import kotlin.test.Test', 'import org.junit.Test')
    jvm_test = jvm_test.replace('Files.createTempDirectory("warm-kt")',
        'Files.createTempDirectory(androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.toPath(), "warm-kt")')
    (tests / 'WarmStoreTest.kt').write_text(jvm_test)
    shutil.copyfile(kotlin / 'android/src/androidTest/kotlin/WarmStoreInstrumentedTest.kt', tests / 'WarmStoreInstrumentedTest.kt')
    shutil.copyfile(ROOT / 'source/benchmarks/public/mobile_qualification/RecoveryConsumerTest.kt', tests / 'RecoveryConsumerTest.kt')
    run('android-consumer-build', gradle + ['-p', consumer, 'assembleDebug', 'assembleDebugAndroidTest', '--write-locks'], consumer)
    return consumer, native


def recover(mode, protocol):
    label = 'recovery-' + mode
    remote = 'files/' + label + '/'
    command = [str(SDK / 'platform-tools/adb'), '-s', SERIAL, 'shell', 'am', 'instrument', '-w', '-r',
               '-e', 'class', 'RecoveryConsumerTest#writer', '-e', 'case', mode,
               APP + '.test/androidx.test.runner.AndroidJUnitRunner']
    with (ROOT / (label + '-writer.log')).open('xb') as log:
        process = subprocess.Popen(command, env=ENV, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            deadline = time.monotonic() + 180
            while True:
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError('writer did not reach completed-snapshot boundary: ' + mode)
                ready = adb('exec-out', 'run-as', APP, 'cat', remote + 'ready', check=False)
                if ready.returncode == 0 and ready.stdout == b'ready\n':
                    break
                time.sleep(1)
            raw = adb('exec-out', 'run-as', APP, 'cat', remote + 'acks.ndjson').stdout
            if len(raw) > 16384 or len(raw.splitlines()) != 3:
                raise ValueError('unexpected recovery acknowledgement size/count')
            with (ROOT / (label + '-acks.ndjson')).open('x') as ledger:
                for stage, line in enumerate(raw.splitlines()):
                    value = protocol.accept_ack(line, stage)
                    ledger.write(json.dumps(value, sort_keys=True) + '\n')
                    ledger.flush()
                    os.fsync(ledger.fileno())
            before = adb('shell', 'pidof', APP).stdout.decode().strip()
            if not before or any(not item.isdigit() for item in before.split()):
                raise ValueError('consumer writer process missing')
            adb('shell', 'am', 'force-stop', APP)
            process.wait(timeout=30)
            if adb('shell', 'pidof', APP, check=False).stdout.strip():
                raise ValueError('force-stop left the consumer running')
        finally:
            if process.poll() is None:
                adb('shell', 'am', 'force-stop', APP, check=False)
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    passed(instrument(label + '-reader', 'RecoveryConsumerTest#reader', mode), 1)
    verified = json.loads(adb('exec-out', 'run-as', APP, 'cat', remote + 'verified.json').stdout)
    if verified != protocol.VERIFIED:
        raise ValueError('recovered-state verification differs')
    result = {'case': mode, 'writerPid': before, 'termination': 'am force-stop',
              'writerCommandExitCode': process.returncode, 'verified': verified,
              'acknowledgementsSha256': digest(ROOT / (label + '-acks.ndjson'))}
    write(ROOT / (label + '.json'), result)
    return result


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    def verify():
        for name, expected in freeze['files'].items():
            path = ROOT / 'source' / name
            if digest(path) != expected or path.stat().st_mode & 0o777 != freeze['modes'][name]:
                raise ValueError('frozen source content/mode changed: ' + name)
        for name, expected in freeze['assets'].items():
            if digest(Path(name)) != expected:
                raise ValueError('toolchain asset changed: ' + name)
        for name, expected in freeze['priorArtifacts'].items():
            if digest(PRIOR / name) != expected:
                raise ValueError('prior artifact changed: ' + name)
        if digest(ROOT / 'run.py') != freeze['runnerSha256']:
            raise ValueError('runner changed')
    verify()
    prior = Path('/Volumes/A/.hms-bench-cache/warm-owner-membership-20261103-v1/finished.json')
    if not prior.is_file() or json.loads(prior.read_text()).get('complete') is not True:
        raise ValueError('local timing job has not finished')
    write(ROOT / 'prerequisite.json', {'path': str(prior), 'sha256': digest(prior), 'purpose': 'no simultaneous timing load'})
    work = ROOT / 'work'
    shutil.copytree(ROOT / 'source', work)
    for name, command, expected in [('rust', ['rustc', '--version'], 'rustc 1.96.0'),
        ('java', [JAVA / 'bin/java', '--version'], '17.0.19'),
        ('emulator', [SDK / 'emulator/emulator', '-version'], '37.2.12')]:
        run('version-' + name, command, work, timeout=60)
        if expected not in (ROOT / ('version-' + name + '.log')).read_text():
            raise ValueError('unexpected toolchain: ' + name)
    kotlin = work / 'bindings/kotlin'
    shutil.copy2(PRIOR / 'work/bindings/kotlin/gradle/wrapper/gradle-wrapper.jar', kotlin / 'gradle/wrapper/gradle-wrapper.jar')
    jvm = freeze['priorJvmTests']
    (ROOT / 'jvm-consumer/libs').mkdir(parents=True)
    shutil.copyfile(PRIOR / 'jvm-consumer/libs/warm-jvm.jar', ROOT / 'jvm-consumer/libs/warm-jvm.jar')
    write(ROOT / 'reused-builds.json', {'sourceFreezeSha256': digest(PRIOR / 'freeze.json'),
        'jvmTests': jvm, 'artifacts': freeze['priorArtifacts'],
        'reason': 'Prior builds and seven JVM tests passed; archive verification needed to model the deterministic Gradle strip step.'})
    consumer, native = android_consumer(kotlin)
    for name in ('bindings/uniffi/Cargo.lock', 'bindings/uniffi/bindgen/Cargo.lock'):
        if digest(work / name) != freeze['files'][name]:
            raise ValueError('build changed frozen Cargo lock: ' + name)
    (ROOT / 'avd').mkdir()
    for port in (5576, 5577):
        with socket.socket() as connection:
            connection.bind(('127.0.0.1', port))
    run('create-avd', [SDK / 'cmdline-tools/latest/bin/avdmanager', 'create', 'avd', '-n', AVD,
                     '-k', 'system-images;android-35;google_apis;arm64-v8a', '-p', ROOT / 'device.avd'], input_text='no\n')
    emulator = None
    with (ROOT / 'emulator.log').open('xb') as log:
        try:
            emulator = subprocess.Popen([str(SDK / 'emulator/emulator'), '-avd', AVD, '-port', '5576',
                '-no-window', '-no-audio', '-no-boot-anim', '-no-snapshot', '-gpu', 'swiftshader',
                '-memory', '2048', '-cores', '2'], env=ENV, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True)
            write(ROOT / 'emulator-launched.json', {'pid': emulator.pid, 'serial': SERIAL, 'avd': AVD})
            adb('wait-for-device', timeout=180)
            deadline = time.monotonic() + 240
            while adb('shell', 'getprop', 'sys.boot_completed').stdout.strip() != b'1':
                if emulator.poll() is not None or time.monotonic() >= deadline:
                    raise TimeoutError('owned emulator did not boot')
                time.sleep(3)
            device = {name: adb('shell', 'getprop', prop).stdout.decode().strip() for name, prop in
                [('api', 'ro.build.version.sdk'), ('abi', 'ro.product.cpu.abi'), ('fingerprint', 'ro.build.fingerprint')]}
            if device['api'] != '35' or device['abi'] != 'arm64-v8a':
                raise ValueError('wrong emulator configuration')
            write(ROOT / 'device.json', device)
            apk = list((consumer / 'build/outputs/apk/debug').glob('*.apk'))
            tests = list((consumer / 'build/outputs/apk/androidTest/debug').glob('*.apk'))
            if len(apk) != 1 or len(tests) != 1:
                raise ValueError('unexpected consumer APK set')
            for name, path in [('consumer', apk[0]), ('tests', tests[0])]:
                run('install-' + name, [SDK / 'platform-tools/adb', '-s', SERIAL, 'install', path], timeout=180)
            passed(instrument('android-lifecycle', 'WarmStoreTest,WarmStoreInstrumentedTest'), 9)
            spec = importlib.util.spec_from_file_location('recovery_protocol', ROOT / 'source/benchmarks/public/native_recovery/run.py')
            protocol = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(protocol)
            recovery = [recover(mode, protocol) for mode in ('plain', 'encrypted')]
            sample = list((PRIOR / 'work/bindings/kotlin/sample-compose/build/outputs/apk/debug').glob('*.apk'))
            if len(sample) != 1:
                raise ValueError('unexpected Compose sample APK set')
            run('install-sample', [SDK / 'platform-tools/adb', '-s', SERIAL, 'install', sample[0]], timeout=180)
            run('launch-sample', [SDK / 'platform-tools/adb', '-s', SERIAL, 'shell', 'am', 'start', '-W',
                                '-n', 'com.writerslogic.warm.sample/.MainActivity'], timeout=60)
            deadline = time.monotonic() + 30
            while adb('shell', 'run-as', 'com.writerslogic.warm.sample', 'test', '-d', 'files/warm-notes', check=False).returncode:
                if time.monotonic() >= deadline:
                    raise ValueError('sample did not create its store')
                time.sleep(1)
            if not adb('shell', 'pidof', 'com.writerslogic.warm.sample').stdout.strip():
                raise ValueError('sample exited after launch')
        finally:
            if emulator is not None:
                if emulator.poll() is None:
                    adb('emu', 'kill', check=False)
                    try:
                        emulator.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        os.killpg(emulator.pid, signal.SIGKILL)
                        emulator.wait(timeout=30)
                write(ROOT / 'emulator-stopped.json', {'pid': emulator.pid, 'exitCode': emulator.returncode})
    verify()
    artifacts = {str(path): digest(path) for path in
                 [ROOT / 'jvm-consumer/libs/warm-jvm.jar', consumer / 'libs/warm-android.aar', apk[0], tests[0], sample[0]]}
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(ROOT / 'freeze.json'), 'jvmTests': jvm, 'androidLifecycleTests': 9,
            'nativeMembers': native, 'recovery': recovery, 'sampleStartup': True, 'device': device,
            'artifacts': artifacts, 'logs': {path.name: digest(path) for path in sorted(ROOT.glob('*.log'))},
            'scope': 'macOS arm64 JVM; API 35 arm64 emulator; completed-snapshot recovery; other ABIs build-only'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
