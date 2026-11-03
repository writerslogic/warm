"""Qualify installed SDK adapters with pinned dependencies and loopback-only networking."""

import argparse
import ast
import hashlib
import json
import os
from pathlib import Path
import pwd
import signal
import subprocess
import time
import xml.etree.ElementTree as ET


MODULES = (
    'test_langchain_live.py', 'test_llamaindex_live.py', 'test_haystack_live.py',
    'test_framework_tools_live.py', 'test_pydantic_ai_live.py',
)


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def run(command, log, cwd, timeout=900):
    environment = {key: value for key, value in os.environ.items()
                   if key in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'TMPDIR')}
    with log.open('xb') as stream:
        child = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT,
                                 cwd=cwd, env=environment, start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
            if code:
                raise RuntimeError(f'{log.name} exited with status {code}')
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=30)


def verify(root, source, server):
    for name, expected in source['files'].items():
        if digest(root / name) != expected:
            raise RuntimeError('frozen framework source changed: ' + name)
    if digest(server) != source['serverSha256']:
        raise RuntimeError('server differs from the frozen source')


def main(args):
    root = Path(__file__).resolve().parent
    source = json.loads((root / 'source.json').read_text())
    verify(root, source, args.server)
    deadline = time.monotonic() + 86400
    while not all(path.is_file() for path in args.wait_for):
        if time.monotonic() > deadline:
            raise TimeoutError('queued prerequisite checks did not finish within twenty-four hours')
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
    consumer = root / 'consumer'
    run([str(args.uv), 'venv', '--python', '3.12.12', str(consumer)],
        args.output / 'venv.log', root)
    executable = consumer / 'bin/python'
    run([str(args.uv), 'pip', 'sync', '--require-hashes', '--python', str(executable),
         str(root / 'requirements.txt')], args.output / 'dependencies.log', root, timeout=1800)
    wheel = root / 'dist/warm_sdk-0.1.3-py3-none-any.whl'
    run([str(args.uv), 'pip', 'install', '--no-deps', '--python', str(executable), str(wheel)],
        args.output / 'install.log', root)
    run([str(args.uv), 'pip', 'check', '--python', str(executable)], args.output / 'dependency-check.log', root)
    run([str(args.uv), 'pip', 'freeze', '--python', str(executable)], args.output / 'installed-requirements.txt', root)
    run([str(executable), '-I', str(root / 'check_installed.py'), str(wheel),
         str(args.output / 'installed.json'), '3.12.12'], args.output / 'installed.log', root)
    account = pwd.getpwuid(os.getuid())
    if account.pw_uid == 0:
        raise RuntimeError('framework tests must run as an ordinary user')
    network_shell = '''set -eu
ip link set lo up
test "$(ip -o link show | wc -l)" -eq 1
ip link show lo >/dev/null
test -z "$(ip route show)"
test -z "$(ip -6 route show default)"
test_user=$1
test_home=$2
test_server=$3
shift 3
exec runuser -u "$test_user" -- env -i PATH=/usr/bin:/bin HOME="$test_home" LANG=C.UTF-8 \
  WARM_SERVER_BIN="$test_server" RAYON_NUM_THREADS=2 NO_PROXY=127.0.0.1,localhost \
  OTEL_SDK_DISABLED=true CREWAI_TELEMETRY_DISABLED=true HAYSTACK_TELEMETRY_ENABLED=false \
  HF_HUB_OFFLINE=1 "$@"
'''
    command = ['sudo', '-n', 'unshare', '--net', '--', '/bin/sh', '-c', network_shell,
               'warm-sdk-frameworks', account.pw_name, account.pw_dir, str(args.server),
               str(executable), '-I', '-m', 'pytest', '-q', '-rs', '-p', 'no:cacheprovider',
               '--junitxml=' + str(args.output / 'frameworks.xml')]
    command.extend(str(root / 'tests' / name) for name in MODULES)
    run(command, args.output / 'frameworks.log', root)
    report = ET.parse(args.output / 'frameworks.xml').getroot()
    counts = {name: sum(int(suite.attrib.get(name, 0)) for suite in report.iter('testsuite'))
              for name in ('tests', 'failures', 'errors', 'skipped')}
    if any(counts[name] for name in ('failures', 'errors', 'skipped')):
        raise RuntimeError('framework qualification requires zero failures, errors and skips')
    expected = {(Path(name).stem, node.name) for name in MODULES
                for node in ast.parse((root / 'tests' / name).read_text()).body
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith('test_')}
    observed = {(case.attrib['classname'].split('.')[-1], case.attrib['name'].split('[')[0])
                for case in report.iter('testcase')}
    if observed != expected or counts['tests'] < len(expected):
        raise RuntimeError('collected framework tests differ from the frozen modules')
    verify(root, source, args.server)
    return {'complete': True, 'scope': 'Six installed HTTP SDK framework adapters, Python 3.12.12, Linux arm64',
            'counts': counts, 'testFunctions': len(expected), 'network': 'private network namespace with loopback only',
            'installed': json.loads((args.output / 'installed.json').read_text()),
            'sourceSha256': digest(root / 'source.json'), 'benchmarkClaim': False,
            'exclusions': ['DSPy', 'separate integrations package', 'cloud importer services',
                           'model-generated answer quality', 'other Python and platform combinations']}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--uv', required=True, type=Path)
    parser.add_argument('--server', required=True, type=Path)
    parser.add_argument('--wait-for', required=True, type=Path, nargs='+')
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
