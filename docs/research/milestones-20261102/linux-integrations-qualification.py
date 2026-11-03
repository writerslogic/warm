"""Qualify fresh framework wheels after the serving measurements terminate."""
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
ROOT = BASE / 'integrations-qualification-v1'
SERVER = BASE / 'dense-product-v2/warm-server'
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
UV = str(BASE / 'tools/uv/bin/uv')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def run(folder, name, args, timeout=1800, require_success=True):
    with (folder / (name + '.log')).open('xb') as log:
        child = subprocess.Popen(args, cwd=ROOT, env=ENV, stdout=log,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=30)
    if require_success and code:
        raise RuntimeError(name + ' failed with exit ' + str(code))
    return code


def verify(freeze):
    for name, expected in freeze['files'].items():
        if digest(ROOT / name) != expected:
            raise ValueError('frozen integration input changed: ' + name)
    if digest(SERVER) != freeze['serverSha256']:
        raise ValueError('frozen integration server changed')


def profile(name, spec, freeze):
    folder = ROOT / 'results' / name
    folder.mkdir()
    result = {'complete': False, 'profile': name}
    try:
        verify(freeze)
        project = folder / 'dependencies'
        project.mkdir()
        dependencies = ['pytest==9.2.0', 'pytest-asyncio==1.4.1', 'numpy', 'httpx>=0.27', 'sniffio', *spec['dependencies']]
        (project / 'pyproject.toml').write_text(
            '[project]\nname = "warm-integration-consumer-' + name + '"\nversion = "0.0.0"\n'
            'requires-python = ">=3.12,<3.13"\ndependencies = ' + json.dumps(dependencies) + '\n')
        run(folder, 'lock', [UV, 'lock', '--project', str(project), '--python', '3.12.12'])
        requirements = project / 'requirements.txt'
        run(folder, 'export', [UV, 'export', '--project', str(project), '--frozen',
                              '--no-emit-project', '--format', 'requirements-txt', '--output-file', str(requirements)])
        hashes = {path.name: digest(path) for path in (project / 'pyproject.toml', project / 'uv.lock', requirements)}
        write(folder / 'dependency-freeze.json', {
            'frozenAt': datetime.now(timezone.utc).isoformat(), 'files': hashes,
            'sourceFreezeSha256': digest(ROOT / 'freeze.json')})
        consumer = folder / 'consumer'
        python = str(consumer / 'bin/python')
        run(folder, 'environment', [UV, 'venv', '--python', '3.12.12', str(consumer)])
        run(folder, 'dependencies', [UV, 'pip', 'sync', '--require-hashes', '--python', python, str(requirements)])
        wheels = {'sdk': ROOT / 'dist/warm_sdk-0.1.3-py3-none-any.whl',
                  'integrations': ROOT / 'dist/writerslogic_warm_integrations-0.1.0-py3-none-any.whl'}
        run(folder, 'install', [UV, 'pip', 'install', '--no-deps', '--python', python,
                                *(str(path) for path in wheels.values())])
        run(folder, 'dependency-check', [UV, 'pip', 'check', '--python', python])
        for package, wheel in wheels.items():
            run(folder, 'installed-' + package, [python, '-I', str(ROOT / ('check-installed-' + package + '.py')),
                                                str(wheel), str(folder / ('installed-' + package + '.json')), '3.12.12'])
        account = pwd.getpwuid(os.getuid())
        if account.pw_uid == 0:
            raise ValueError('framework consumers must run as an ordinary user')
        shell = '''set -eu
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
  LITELLM_LOCAL_MODEL_COST_MAP=True HF_HUB_OFFLINE=1 OTEL_SDK_DISABLED=true \\
  CREWAI_TELEMETRY_DISABLED=true HAYSTACK_TELEMETRY_ENABLED=false MEM0_TELEMETRY=False "$@"
'''
        modules = [ROOT / 'source/integrations/python/tests' / path for path in spec['modules']]
        command = ['sudo', '-n', 'unshare', '--net', '--', '/bin/sh', '-c', shell,
                   'warm-integrations-qualification', account.pw_name, account.pw_dir, str(SERVER),
                   python, '-I', str(ROOT / 'run-integration-tests.py'), str(folder / 'collection.json'),
                   '-q', '-rs', '-p', 'no:cacheprovider', '--asyncio-mode=auto',
                   '--junitxml=' + str(folder / 'tests.xml'), *(str(path) for path in modules)]
        code = run(folder, 'tests', command, timeout=3600, require_success=False)
        report = ET.parse(folder / 'tests.xml').getroot()
        counts = {field: sum(int(suite.attrib.get(field, 0)) for suite in report.iter('testsuite'))
                  for field in ('tests', 'failures', 'errors', 'skipped')}
        collection = json.loads((folder / 'collection.json').read_text())
        collected_modules = {Path(item['path']).resolve() for item in collection}
        if collected_modules != set(modules) or counts['tests'] != len(collection):
            raise ValueError('test collection does not cover all frozen modules')
        expected_functions = {(str(path), node.name) for path in modules
                              for node in ast.parse(path.read_text()).body
                              if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith('test_')}
        actual_functions = {(str(Path(item['path']).resolve()), item['originalName']) for item in collection}
        if not expected_functions <= actual_functions:
            raise ValueError('frozen top-level test function was not collected')
        skipped = [{'class': case.attrib.get('classname'), 'name': case.attrib.get('name'),
                    'reason': case.find('skipped').attrib.get('message', '')}
                   for case in report.iter('testcase') if case.find('skipped') is not None]
        for path, expected in hashes.items():
            if digest(project / path) != expected:
                raise ValueError('frozen dependency profile changed')
        verify(freeze)
        result = {'complete': True, 'profile': name, 'exitCode': code, 'counts': counts,
                  'gatePass': code == 0 and not any(counts[field] for field in ('failures', 'errors', 'skipped')),
                  'skipped': skipped, 'skipReviewRequired': bool(skipped),
                  'dependencyFreezeSha256': digest(folder / 'dependency-freeze.json'),
                  'reportSha256': digest(folder / 'tests.xml'), 'collectionSha256': digest(folder / 'collection.json')}
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
    finally:
        write(folder / 'finished.json', result)
    return result


def main():
    if not (BASE / 'serving-qualification-v1/stop-guest-observation').exists():
        raise ValueError('serving observations must finish before integration qualification')
    terminal = json.loads((BASE / 'serving-qualification-v1/finished.json').read_text())
    if terminal.get('complete') is not True:
        raise ValueError('serving execution requires review before unrelated work')
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    verify(freeze)
    (ROOT / 'results').mkdir()
    plan = json.loads((ROOT / 'plan.json').read_text())
    results = [profile(name, spec, freeze) for name, spec in plan['profiles'].items()]
    verify(freeze)
    return {'complete': all(row['complete'] for row in results),
            'gatePass': all(row.get('gatePass') is True for row in results),
            'completedAt': datetime.now(timezone.utc).isoformat(), 'profiles': results,
            'freezeSha256': digest(ROOT / 'freeze.json'), 'paidServicesUsed': False,
            'scope': 'Fresh Linux arm64/Python 3.12.12 wheels, real WARM server, local scripted models in loopback-only namespaces. Wire-only tests retain that limit; no real Letta server or local-model text-query claim.'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
