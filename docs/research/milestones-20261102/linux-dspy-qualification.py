"""Qualify both installed DSPy adapters without an external network route."""
from datetime import datetime, timezone
import ast
import hashlib
import json
import os
from pathlib import Path
import pwd
import signal
import subprocess
import xml.etree.ElementTree as ET

BASE = Path('/home/davidcondrey.guest/warm-milestones-20261103')
ROOT = BASE / 'dspy-qualification-v1'
SERVER = BASE / 'dense-product-v2/warm-server'
OUTPUT = ROOT / 'results'
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
CASES = {
    'sdk': (['bindings/python/warm-sdk/tests/test_dspy_live.py'], 9),
    'integrations': (['integrations/python/tests/dspy/test_dspy_retriever.py',
                      'integrations/python/tests/dspy/test_cache_isolation.py'], 11),
}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def verify(freeze):
    for name, expected in freeze['files'].items():
        if digest(ROOT / name) != expected:
            raise ValueError('frozen DSPy source/artifact changed: ' + name)
    if digest(SERVER) != freeze['serverSha256']:
        raise ValueError('frozen DSPy server changed')


def run(name, args, timeout=1800):
    with (OUTPUT / (name + '.log')).open('xb') as log:
        process = subprocess.Popen(args, cwd=ROOT, env=ENV, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    if code:
        raise RuntimeError(name + ' failed with exit ' + str(code))


def main():
    OUTPUT.mkdir()
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    verify(freeze)
    uv = str(BASE / 'tools/uv/bin/uv')
    consumer = ROOT / 'consumer'
    python = str(consumer / 'bin/python')
    run('environment', [uv, 'venv', '--python', '3.12.12', str(consumer)])
    run('dependencies', [uv, 'pip', 'sync', '--require-hashes', '--python', python,
                         str(ROOT / 'requirements.txt')])
    wheels = {'sdk': ROOT / 'dist/warm_sdk-0.1.3-py3-none-any.whl',
              'integrations': ROOT / 'dist/writerslogic_warm_integrations-0.1.0-py3-none-any.whl'}
    run('install', [uv, 'pip', 'install', '--no-deps', '--python', python, *(str(p) for p in wheels.values())])
    run('dependency-check', [uv, 'pip', 'check', '--python', python])
    run('installed-requirements', [uv, 'pip', 'freeze', '--python', python])
    for name, wheel in wheels.items():
        run('installed-' + name, [python, '-I', str(ROOT / ('check-installed-' + name + '.py')),
            str(wheel), str(OUTPUT / ('installed-' + name + '.json')), '3.12.12'])
    account = pwd.getpwuid(os.getuid())
    if account.pw_uid == 0:
        raise ValueError('consumer tests require an ordinary user')
    network_shell = '''set -eu
ip link set lo up
test "$(ip -o link show | wc -l)" -eq 1
test -z "$(ip route show)"
test -z "$(ip -6 route show default)"
test_user=$1
test_home=$2
test_server=$3
shift 3
exec runuser -u "$test_user" -- env -i PATH=/usr/bin:/bin HOME="$test_home" LANG=C.UTF-8 \\
  WARM_SERVER_BIN="$test_server" RAYON_NUM_THREADS=2 NO_PROXY=127.0.0.1,localhost \\
  LITELLM_LOCAL_MODEL_COST_MAP=True HF_HUB_OFFLINE=1 OTEL_SDK_DISABLED=true "$@"
'''
    results = {}
    for name, (modules, expected_count) in CASES.items():
        command = ['sudo', '-n', 'unshare', '--net', '--', '/bin/sh', '-c', network_shell,
                   'warm-dspy-qualification', account.pw_name, account.pw_dir, str(SERVER),
                   python, '-I', str(ROOT / 'run-dspy-tests.py'), '-q', '-rs', '-p', 'no:cacheprovider',
                   '--asyncio-mode=auto', '--junitxml=' + str(OUTPUT / (name + '.xml'))]
        command.extend(str(ROOT / 'source' / module) for module in modules)
        run(name, command, timeout=900)
        report = ET.parse(OUTPUT / (name + '.xml')).getroot()
        counts = {field: sum(int(suite.attrib.get(field, 0)) for suite in report.iter('testsuite'))
                  for field in ('tests', 'failures', 'errors', 'skipped')}
        expected = {(Path(module).stem, node.name) for module in modules
                    for node in ast.parse((ROOT / 'source' / module).read_text()).body
                    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith('test_')}
        actual = {(case.attrib['classname'].split('.')[-1], case.attrib['name'].split('[')[0])
                  for case in report.iter('testcase')}
        if actual != expected or counts['tests'] != expected_count or any(counts[k] for k in ('failures', 'errors', 'skipped')):
            raise ValueError(name + ' did not execute every declared case without skips')
        results[name] = {'counts': counts, 'functions': len(expected),
                         'reportSha256': digest(OUTPUT / (name + '.xml'))}
    verify(freeze)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'results': results, 'freezeSha256': digest(ROOT / 'freeze.json'),
            'network': 'Private loopback-only namespace, ordinary user, no inherited service credentials',
            'model': 'DSPy DummyLM and local hashing/deterministic embeddings; no answer-quality claim',
            'scope': 'Fresh wheel consumers on Linux arm64 / Python 3.12.12, pinned documented DSPy profile',
            'pending': ['Other integration packages and hosts', 'Migration and remaining recovery surfaces']}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        OUTPUT.mkdir(exist_ok=True)
        write(OUTPUT / 'finished.json', result)
