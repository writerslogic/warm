import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import time


FAMILY = Path('/home/davidcondrey.guest/warm-milestones-20261103/c-family-v2')
DIAGNOSTIC = Path('/home/davidcondrey.guest/warm-milestones-20261103/evidence/python-sdk-live-v2/finished.json')
ACKS = [{'phase': 'inserted', 'documents': 32}, {'phase': 'replaced', 'documents': 16},
        {'phase': 'ready', 'deleted': 8}]
VERIFIED = {'phase': 'verified', 'records': 24, 'deleted': 8, 'resumedWrite': True}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write('\n')


def accept_ack(line, stage):
    if not 0 <= stage < len(ACKS) or len(line) > 4096:
        raise ValueError('unexpected acknowledgement stage or size')
    value = json.loads(line)
    if value != ACKS[stage]:
        raise ValueError('writer acknowledgement differs from frozen protocol')
    return value


def ready(process, ledger):
    deadline, buffer, stage, received = time.monotonic() + 120, b'', 0, 0
    while stage < len(ACKS):
        if time.monotonic() >= deadline:
            raise TimeoutError('writer did not reach flushed boundary')
        readable, _, _ = select.select([process.stdout], [], [], min(1, max(0, deadline - time.monotonic())))
        if not readable:
            continue
        chunk = os.read(process.stdout.fileno(), 4096)
        if not chunk:
            raise ValueError('writer exited before durable boundary')
        received += len(chunk)
        if received > 16384:
            raise ValueError('writer protocol exceeded byte limit')
        buffer += chunk
        while b'\n' in buffer:
            line, buffer = buffer.split(b'\n', 1)
            value = accept_ack(line, stage)
            ledger.write(json.dumps(value, sort_keys=True) + '\n')
            ledger.flush()
            os.fsync(ledger.fileno())
            stage += 1
        if stage == len(ACKS) and buffer.strip():
            raise ValueError('unexpected trailing writer output')


def run(command, log, environment, cwd, timeout=900):
    with log.open('xb') as stream:
        child = subprocess.Popen([str(item) for item in command], cwd=cwd, env=environment,
                                 stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
            if code:
                raise RuntimeError(log.name + ' failed with status ' + str(code))
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=30)


def verify_artifacts(receipt):
    if receipt.get('complete') is not True or receipt['freezeSha256'] != digest(FAMILY / 'freeze.json'):
        raise ValueError('C-family qualification has not passed with its frozen source')
    for name, expected in receipt['artifacts'].items():
        relative = Path(name)
        if relative.is_absolute() or '..' in relative.parts or digest(FAMILY / relative) != expected:
            raise ValueError('qualified artifact mismatch')


def main(root):
    deadline = time.monotonic() + 172800
    while not (FAMILY / 'finished.json').exists() or not DIAGNOSTIC.exists():
        if time.monotonic() > deadline:
            raise TimeoutError('prior local checks still pending after 48 hours')
        time.sleep(60)
    family = json.loads((FAMILY / 'finished.json').read_text())
    verify_artifacts(family)
    write(root / 'artifact-receipt.json', family)
    freeze = json.loads((root / 'freeze.json').read_text())
    if freeze['familyFreezeSha256'] != digest(FAMILY / 'freeze.json'):
        raise ValueError('C-family source differs from the recovery preselection')
    def verify_source():
        for name, expected in freeze['files'].items():
            if digest(root / 'source' / name) != expected:
                raise ValueError('frozen recovery source changed')
    verify_source()
    environment = {key: value for key, value in os.environ.items() if key in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'TMPDIR')}
    environment.update({'PATH': str(FAMILY / 'tools/dotnet') + ':' + environment['PATH'], 'DOTNET_ROOT': str(FAMILY / 'tools/dotnet'), 'DOTNET_CLI_HOME': str(root / 'dotnet-home'), 'NUGET_PACKAGES': str(root / 'nuget-cache'),
                        'DOTNET_CLI_TELEMETRY_OPTOUT': '1', 'DOTNET_SKIP_FIRST_TIME_EXPERIENCE': '1',
                        'GOTOOLCHAIN': 'local', 'GOCACHE': str(root / 'go-cache'), 'GOMAXPROCS': '2'})
    (root / 'global.json').write_text('{"sdk":{"version":"8.0.130","rollForward":"disable"}}\n')
    package = FAMILY / 'installed-c/writerslogic-warm-c-0.1.0-linux-arm64'
    run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-I' + str(package / 'include'),
         root / 'source/recovery.c', '-o', root / 'recovery-c', '-L' + str(package / 'lib'),
         '-lwriterslogic_warm_c', '-Wl,-rpath,' + str(package / 'lib')], root / 'compile-c.log', environment, root)
    go = root / 'go'
    go.mkdir()
    shutil.copyfile(root / 'source/recovery.go', go / 'main.go')
    (go / 'go.mod').write_text('module warm-recovery-consumer\n\ngo 1.24\n\nrequire github.com/writerslogic/warm/bindings/go v0.0.0\nreplace github.com/writerslogic/warm/bindings/go => ' + json.dumps(str(FAMILY / 'consumer-go')) + '\n')
    go_tool = FAMILY / 'tools/go/bin/go'
    run([go_tool, 'mod', 'tidy'], root / 'go-tidy.log', environment, go)
    run([go_tool, 'build', '-o', root / 'recovery-go', '.'], root / 'compile-go.log', environment, go)
    java = FAMILY / 'tools/jdk-25.0.5+8/bin'
    jar = FAMILY / 'work/bindings/java/target/writerslogic-warm-0.1.0.jar'
    (root / 'java').mkdir()
    run([java / 'javac', '--release', '22', '-cp', jar, '-d', root / 'java', root / 'source/RecoveryConsumer.java'],
        root / 'compile-java.log', environment, root)
    dotnet = root / 'dotnet'
    dotnet.mkdir()
    shutil.copyfile(root / 'source/Program.cs', dotnet / 'Program.cs')
    (dotnet / 'Recovery.csproj').write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings><Nullable>enable</Nullable></PropertyGroup><ItemGroup><PackageReference Include="WritersLogic.WarmMemory" Version="0.1.0"/></ItemGroup></Project>\n')
    (dotnet / 'NuGet.Config').write_text('<configuration><packageSources><clear/><add key="local" value="' + str(FAMILY / 'dist') + '"/><add key="nuget" value="https://api.nuget.org/v3/index.json"/></packageSources></configuration>\n')
    run(['dotnet', 'build', '-c', 'Release', '-o', dotnet / 'bin'], root / 'compile-dotnet.log', environment, dotnet)
    commands = {'c': [str(root / 'recovery-c')], 'go': [str(root / 'recovery-go')],
                'java': [str(java / 'java'), '--enable-native-access=ALL-UNNAMED', '-cp', str(root / 'java') + os.pathsep + str(jar), 'RecoveryConsumer'],
                'dotnet': ['dotnet', str(dotnet / 'bin/Recovery.dll')]}
    cases = []
    for language, command in commands.items():
        for label, termination in (('kill', signal.SIGKILL), ('terminate', signal.SIGTERM)):
            folder = root / (language + '-' + label)
            folder.mkdir()
            store = folder / 'store'
            with (folder / 'writer.log').open('xb') as error, (folder / 'acks.ndjson').open('x') as ledger:
                child = subprocess.Popen(command + ['write', str(store)], stdout=subprocess.PIPE, stderr=error,
                                         env=environment, cwd=root, start_new_session=True)
                try:
                    ready(child, ledger)
                    if child.poll() is not None:
                        raise ValueError('writer exited before forced termination')
                    os.killpg(child.pid, termination)
                    code = child.wait(timeout=30)
                    if code == 0:
                        raise ValueError('writer unexpectedly exited normally')
                finally:
                    if child.poll() is None:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait(timeout=30)
                    child.stdout.close()
            readers = []
            for reader, verifier in commands.items():
                log = folder / ('verify-' + reader + '.log')
                run(verifier + ['verify', str(store)], log, environment, root, timeout=120)
                lines = [line for line in log.read_text().splitlines() if line.startswith('{')]
                if len(lines) != 1 or json.loads(lines[0]) != VERIFIED:
                    raise ValueError('missing or invalid verifier receipt')
                readers.append(reader)
            result = {'writer': language, 'termination': label, 'exitCode': code, 'verifiedReaders': readers,
                      'ledgerSha256': digest(folder / 'acks.ndjson')}
            write(folder / 'finished.json', result)
            cases.append(result)
    verify_artifacts(family)
    verify_source()
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(root / 'freeze.json'), 'writerCases': len(cases),
            'readerChecks': sum(len(case['verifiedReaders']) for case in cases), 'cases': cases}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, required=True)
    arguments = parser.parse_args()
    try:
        result = main(arguments.root)
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(arguments.root / 'finished.json', result)
