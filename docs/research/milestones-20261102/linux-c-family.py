"""Build packaged native bindings and check fresh Linux arm64 glibc consumers."""

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


ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103/c-family-v2')
JAVA = ROOT / 'tools/jdk-25.0.5+8'
GO = ROOT / 'tools/go/bin/go'
MVN = ROOT / 'tools/apache-maven-3.9.16/bin/mvn'
TARGET = ROOT / 'target'
ENV = {name: value for name, value in os.environ.items() if name in ('PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL')}
ENV.update({'LIMA_HOME': '/Volumes/A/.hms-lima', 'JAVA_HOME': str(JAVA),
            'PATH': str(ROOT / 'tools/dotnet') + ':' + str(GO.parent) + ':' + str(JAVA / 'bin') + ':' + str(Path.home() / '.cargo/bin') + ':' + ENV['PATH'], 'DOTNET_ROOT': str(ROOT / 'tools/dotnet'),
            'CARGO_TARGET_DIR': str(TARGET), 'CARGO_BUILD_JOBS': '2', 'RAYON_NUM_THREADS': '2',
            'GOTOOLCHAIN': 'local', 'GOCACHE': str(ROOT / 'go-cache'),
            'DOTNET_CLI_HOME': str(ROOT / 'dotnet-home'), 'NUGET_PACKAGES': str(ROOT / 'nuget-cache'),
            'DOTNET_CLI_TELEMETRY_OPTOUT': '1', 'DOTNET_SKIP_FIRST_TIME_EXPERIENCE': '1'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def run(name, command, cwd=ROOT, extra=None, timeout=7200):
    with (ROOT / (name + '.log')).open('xb') as stream:
        process = subprocess.Popen([str(value) for value in command], cwd=cwd, env={**ENV, **(extra or {})},
                                   stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
            if code:
                raise RuntimeError(f'{name} failed with exit {code}')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)


def wait_for_work():
    prior = json.loads((ROOT / 'prior-work-terminal.json').read_text())
    if prior['macFamily'].get('complete') is not True or prior['macRecovery'].get('complete') is not True:
        raise ValueError('macOS artifact and recovery checks must pass before this follow-up')


def verify_source(freeze):
    for name, expected in freeze['files'].items():
        if digest(ROOT / 'source' / name) != expected:
            raise ValueError('frozen source changed: ' + name)
    if digest(ROOT / 'run.py') != freeze['runnerSha256']:
        raise ValueError('runner changed after freeze')


def junit(directory):
    files = list(directory.glob('TEST-*.xml'))
    if not files:
        raise ValueError('missing JUnit report')
    totals = {'tests': 0, 'failures': 0, 'errors': 0, 'skipped': 0}
    for path in files:
        suite = ET.parse(path).getroot()
        for name in totals:
            totals[name] += int(suite.get(name, '0'))
    if not totals['tests'] or any(totals[name] for name in ('failures', 'errors', 'skipped')):
        raise ValueError('JUnit consumer did not pass without skips')
    return totals


def main():
    wait_for_work()
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    verify_source(freeze)
    work = ROOT / 'work'
    shutil.copytree(ROOT / 'source', work)
    (ROOT / 'global.json').write_text('{"sdk":{"version":"8.0.130","rollForward":"disable"}}\n')
    versions = {}
    for name, command, expected in [('rust', ['rustc', '--version'], 'rustc 1.96.0'),
                                     ('go', [GO, 'version'], 'go1.24.13'),
                                     ('java', [JAVA / 'bin/java', '--version'], '25.0.5'),
                                     ('maven', [MVN, '--version'], '3.9.16'),
                                     ('dotnet', ['dotnet', '--version'], '8.0.130')]:
        run('version-' + name, command, timeout=60)
        value = (ROOT / ('version-' + name + '.log')).read_text()
        if expected not in value:
            raise ValueError('toolchain version mismatch: ' + name)
        versions[name] = value
    write(ROOT / 'versions.json', versions)
    crate = work / 'bindings/c/Cargo.toml'
    run('c-rust-tests', ['cargo', 'test', '--locked', '--manifest-path', crate, '--', '--test-threads=2'])
    log = (ROOT / 'c-rust-tests.log').read_text()
    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', log)
    if not summaries or not sum(int(row[0]) for row in summaries) or any(int(row[1]) or int(row[2]) for row in summaries) or 'SKIPPED:' in log:
        raise ValueError('C Rust test coverage has a failure, skip or missing report')
    run('c-release', ['cargo', 'build', '--release', '--locked', '--manifest-path', crate])
    run('c-package', ['bash', work / 'bindings/c/scripts/package-native.sh', 'linux-arm64', '', ROOT / 'dist'])
    archives = list((ROOT / 'dist').glob('*.tar.gz'))
    if len(archives) != 1:
        raise ValueError('unexpected C archive count')
    package_sha = archives[0].with_name(archives[0].name + '.sha256').read_text().split()[0]
    if package_sha != digest(archives[0]):
        raise ValueError('C archive checksum mismatch')
    installed = ROOT / 'installed-c'
    installed.mkdir()
    with tarfile.open(archives[0]) as archive:
        archive.extractall(installed, filter='data')
    package = next(installed.iterdir())
    shared = package / 'lib/libwriterslogic_warm_c.so'
    static = package / 'lib/libwriterslogic_warm_c.a'
    for name, args in [('shared', ['-L' + str(package / 'lib'), '-lwriterslogic_warm_c', '-Wl,-rpath,' + str(package / 'lib')]),
                       ('static', [static, '-ldl'])]:
        executable = ROOT / ('c-consumer-' + name)
        run('c-compile-' + name, ['cc', '-std=c11', '-D_DEFAULT_SOURCE', '-Wall', '-Wextra', '-Werror',
                                '-I' + str(package / 'include'), work / 'bindings/c/tests/c/test_warm.c',
                                '-o', executable, *args, '-lpthread', '-lm'])
        run('c-consumer-' + name, [executable], timeout=300)
        if 'c abi test: all checks passed' not in (ROOT / ('c-consumer-' + name + '.log')).read_text():
            raise ValueError('missing C executable result')
    go = ROOT / 'consumer-go'
    shutil.copytree(work / 'bindings/go', go)
    (go / 'lib/linux_arm64').mkdir(parents=True)
    shutil.copyfile(static, go / 'lib/linux_arm64' / static.name)
    run('go-vet', [GO, 'vet', './...'], cwd=go)
    run('go-tests', [GO, 'test', '-race', '-count=1', '-json', './...'], cwd=go)
    events = [json.loads(line) for line in (ROOT / 'go-tests.log').read_text().splitlines() if line.startswith('{')]
    if any(event['Action'] in ('fail', 'skip') for event in events) or not any(event['Action'] == 'pass' and 'Test' in event for event in events):
        raise ValueError('Go test failure, skip or missing test coverage')
    java = work / 'bindings/java'
    bundle = java / 'natives/linux-arm64'
    bundle.mkdir(parents=True)
    shutil.copyfile(shared, bundle / shared.name)
    maven_flags = ['-B', '-Dmaven.repo.local=' + str(ROOT / 'maven-repository'), '-Dwarm.native.dir=']
    run('java-package', [MVN, *maven_flags, 'install'], cwd=java)
    java_source_tests = junit(java / 'target/surefire-reports')
    jar = java / 'target/writerslogic-warm-0.1.0.jar'
    with zipfile.ZipFile(jar) as archive:
        if hashlib.sha256(archive.read('META-INF/native/linux-arm64/' + shared.name)).hexdigest() != digest(shared):
            raise ValueError('Java bundled native library mismatch')
    consumer = ROOT / 'consumer-java'
    shutil.copytree(java / 'src/test', consumer / 'src/test')
    pom = (java / 'pom.xml').read_text().replace('<artifactId>writerslogic-warm</artifactId>', '<artifactId>warm-qualification-consumer</artifactId>', 1)
    pom = pom.replace('<dependencies>', '<dependencies><dependency><groupId>com.writerslogic</groupId><artifactId>writerslogic-warm</artifactId><version>0.1.0</version></dependency>', 1)
    pom = re.sub(r'<resources>.*?</resources>', '', pom, flags=re.S)
    (consumer / 'pom.xml').write_text(pom)
    run('java-consumer', [MVN, *maven_flags, 'test'], cwd=consumer)
    java_installed_tests = junit(consumer / 'target/surefire-reports')
    if java_source_tests != java_installed_tests:
        raise ValueError('Java installed test coverage differs')
    dotnet = work / 'bindings/dotnet/src/WarmMemory'
    native = dotnet / 'natives/linux-arm64/native'
    native.mkdir(parents=True)
    shutil.copyfile(shared, native / shared.name)
    run('dotnet-package', ['dotnet', 'pack', dotnet / 'WarmMemory.csproj', '-c', 'Release', '-o', ROOT / 'dist'])
    nupkg = ROOT / 'dist/WritersLogic.WarmMemory.0.1.0.nupkg'
    with zipfile.ZipFile(nupkg) as archive:
        if hashlib.sha256(archive.read('runtimes/linux-arm64/native/' + shared.name)).hexdigest() != digest(shared):
            raise ValueError('.NET bundled native library mismatch')
    consumer_dotnet = ROOT / 'consumer-dotnet'
    shutil.copytree(work / 'bindings/dotnet/tests/WarmMemory.Tests', consumer_dotnet)
    project = consumer_dotnet / 'WarmMemory.Tests.csproj'
    body = re.sub(r'<ProjectReference\s+Include="[^"]+"\s*/>', '<PackageReference Include="WritersLogic.WarmMemory" Version="0.1.0" />', project.read_text())
    if body == project.read_text():
        raise ValueError('could not replace .NET source reference with installed package')
    project.write_text(body)
    (consumer_dotnet / 'NuGet.Config').write_text('<configuration><packageSources><clear/><add key="local" value="' + str(ROOT / 'dist') + '"/><add key="nuget" value="https://api.nuget.org/v3/index.json"/></packageSources></configuration>\n')
    run('dotnet-consumer', ['dotnet', 'test', project, '-c', 'Release', '--logger', 'trx;LogFileName=qualification.trx', '--results-directory', consumer_dotnet / 'results'], cwd=consumer_dotnet)
    report = ET.parse(consumer_dotnet / 'results/qualification.trx').getroot()
    counters = report.find('.//{http://microsoft.com/schemas/VisualStudio/TeamTest/2010}Counters')
    if counters is None or int(counters.get('total', '0')) == 0 or counters.get('total') != counters.get('passed'):
        raise ValueError('.NET installed tests did not all pass')
    verify_source(freeze)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'scope': 'Linux arm64 glibc C packaged static/shared, Go source consumer with race checks, installed Java jar and .NET package tests; explicit crash/migration and other OS targets remain open',
            'freezeSha256': digest(ROOT / 'freeze.json'), 'javaTests': java_installed_tests,
            'dotnetTests': dict(counters.attrib), 'goTestPasses': sum(event['Action'] == 'pass' and 'Test' in event for event in events),
            'artifacts': {str(path.relative_to(ROOT)): digest(path) for path in [*archives, jar, nupkg, shared, static]}}


if __name__ == '__main__':
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(ROOT / 'finished.json', result)
