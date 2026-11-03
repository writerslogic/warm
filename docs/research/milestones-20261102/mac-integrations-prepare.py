"""Queue the remaining framework consumers after all local timing ends."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time

REPO = Path('/Volumes/A/holographic-memory')
CANONICAL = REPO / 'docs/research/milestones-20261102'
ROOT = Path('/Volumes/A/.hms-bench-cache/warm-integrations-qualification-20261103-v1')
BASIS = ROOT.with_name('warm-dspy-qualification-20261103-v2')
SERVING = ROOT.with_name('warm-serving-qualification-20261103-v1')
GUEST_BASE = '/home/davidcondrey.guest/warm-milestones-20261103'
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV['LIMA_HOME'] = '/Volumes/A/.hms-lima'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def run(name, arguments, timeout=120, input_text=None):
    with (ROOT / (name + '.log')).open('x') as log:
        subprocess.run(arguments, env=ENV, input=input_text, text=True, stdout=log,
                       stderr=subprocess.STDOUT, check=True, timeout=timeout)


def main():
    request = json.loads((ROOT / 'preparation-request.json').read_text())
    deadline = time.monotonic() + 86400
    terminal = SERVING / 'execution-launcher-finished.json'
    host_terminal = Path('/Users/davidcondrey/.cache/warm-serving-isolation-20261103-v1/finished.json')
    guest_samples = Path('/Users/davidcondrey/.cache/warm-serving-guest-isolation-20261103-v1/samples.ndjson')
    while not (terminal.exists() and host_terminal.exists()):
        if time.monotonic() > deadline:
            raise TimeoutError('serving/observer prerequisites did not finish in 24 hours')
        time.sleep(30)
    if not all(json.loads(path.read_text()).get('complete') is True for path in (terminal, host_terminal)):
        raise ValueError('serving/observer failure requires inspection before new work')
    while True:
        with guest_samples.open('rb') as stream:
            stream.seek(max(0, guest_samples.stat().st_size - 65536))
            rows = stream.read().splitlines()
        try:
            last = json.loads(rows[-1])
        except json.JSONDecodeError:
            last = {}
        if last.get('type') == 'terminal':
            if last.get('complete') is not True:
                raise ValueError('guest observer did not finish successfully')
            break
        if time.monotonic() > deadline:
            raise TimeoutError('guest observer did not stop')
        time.sleep(5)
    if digest(BASIS / 'freeze.json') != request['basisFreezeSha256']:
        raise ValueError('basis source freeze changed')
    basis = json.loads((BASIS / 'freeze.json').read_text())
    names = [name for name in basis['files'] if name.startswith(('source/', 'dist/'))
             or name in ('check-installed-sdk.py', 'check-installed-integrations.py')]
    for name in names:
        if digest(BASIS / name) != basis['files'][name]:
            raise ValueError('basis source/wheel changed: ' + name)
        target = ROOT / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(BASIS / name, target)
    for source, entry in request['files'].items():
        path = REPO / source
        if digest(path) != entry['sha256']:
            raise ValueError('queued input changed: ' + source)
        target = ROOT / entry['destination']
        if target.exists() and digest(target) != entry['sha256']:
            raise ValueError('overlay would change frozen basis: ' + source)
        target.parent.mkdir(parents=True, exist_ok=True)
        if not target.exists():
            shutil.copy2(path, target)
        if entry['destination'] not in names:
            names.append(entry['destination'])
    freeze = {'schema': 'warm.integrations-qualification-source/1',
              'frozenAt': datetime.now(timezone.utc).isoformat(),
              'basisFreezeSha256': request['basisFreezeSha256'], 'serverSha256': basis['serverSha256'],
              'files': {name: digest(ROOT / name) for name in sorted(names)},
              'dependencyPolicy': 'Each independent profile records a uv lock and hash-bearing requirements freeze before installing and executing its tests.'}
    write(ROOT / 'freeze.json', freeze)
    archive = ROOT / 'run-bundle.tar'
    with tarfile.open(archive, 'x') as packed:
        for name in sorted([*names, 'freeze.json']):
            packed.add(ROOT / name, arcname=name, recursive=False)
    archive_hash = digest(archive)
    write(ROOT / 'transfer.json', {'archiveSha256': archive_hash, 'freezeSha256': digest(ROOT / 'freeze.json'),
                                   'files': len(names), 'createdAt': datetime.now(timezone.utc).isoformat()})
    for name in ('freeze.json', 'transfer.json'):
        shutil.copy2(ROOT / name, CANONICAL / ('integrations-v1-' + name))
    run('transfer', ['limactl', 'copy', str(archive), 'warm-arm64:' + GUEST_BASE + '/integrations-v1.tar'])
    extraction = '''from pathlib import Path
import hashlib,tarfile
base=Path(%r);archive=base/'integrations-v1.tar';destination=base/'integrations-qualification-v1'
with archive.open('rb') as f:assert hashlib.file_digest(f,'sha256').hexdigest()==%r
destination.mkdir()
with tarfile.open(archive) as packed:
 members=packed.getmembers();assert len(members)==%r
 for m in members:
  p=Path(m.name)
  assert m.isfile() and not p.is_absolute() and '..' not in p.parts and 0<=m.size<=4*2**20
 packed.extractall(destination,filter='data')
print('Verified and extracted',len(members),'files')
''' % (GUEST_BASE, archive_hash, len(names) + 1)
    run('extraction', ['limactl', 'shell', 'warm-arm64', '--', 'python3', '-'], input_text=extraction)
    run('qualification', ['limactl', 'shell', 'warm-arm64', '--', 'python3',
                          GUEST_BASE + '/integrations-qualification-v1/linux-integrations-qualification.py'], timeout=86400)
    run('fetch-finished', ['limactl', 'copy', 'warm-arm64:' + GUEST_BASE + '/integrations-qualification-v1/finished.json',
                          str(ROOT / 'finished.json')])
    shutil.copy2(ROOT / 'finished.json', CANONICAL / 'integrations-v1-finished.json')
    result = json.loads((ROOT / 'finished.json').read_text())
    return {'complete': result['complete'], 'gatePass': result.get('gatePass', False),
            'completedAt': datetime.now(timezone.utc).isoformat(), 'finishedSha256': digest(ROOT / 'finished.json')}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'preparation-finished.json', result)
