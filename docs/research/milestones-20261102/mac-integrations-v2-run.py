"""Transfer and run the repaired framework qualification bundle."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-integrations-qualification-20261103-v2')
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103'
CANONICAL = Path('/Volumes/A/holographic-memory/docs/research/milestones-20261102')
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV['LIMA_HOME'] = '/Volumes/A/.hms-lima'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def run(name, args, timeout=120, input_text=None):
    with (ROOT / (name + '.log')).open('x') as log:
        subprocess.run(args, env=ENV, input=input_text, text=True, stdout=log,
                       stderr=subprocess.STDOUT, check=True, timeout=timeout)


def main():
    transfer = json.loads((ROOT / 'transfer.json').read_text())
    archive = ROOT / 'run-bundle.tar'
    if digest(archive) != transfer['archiveSha256']:
        raise ValueError('bundle changed after freeze')
    run('transfer', ['limactl', 'copy', str(archive), 'warm-arm64:' + GUEST + '/integrations-v2.tar'])
    extraction = '''from pathlib import Path
import hashlib,tarfile
base=Path(%r);archive=base/'integrations-v2.tar';destination=base/'integrations-qualification-v2'
with archive.open('rb') as f:assert hashlib.file_digest(f,'sha256').hexdigest()==%r
destination.mkdir()
with tarfile.open(archive) as packed:
 members=packed.getmembers();assert len(members)==%r
 for m in members:
  p=Path(m.name)
  assert m.isfile() and not p.is_absolute() and '..' not in p.parts and 0<=m.size<=4*2**20
 packed.extractall(destination,filter='data')
print('Verified and extracted',len(members),'files')
''' % (GUEST, transfer['archiveSha256'], transfer['files'] + 1)
    run('extraction', ['limactl', 'shell', 'warm-arm64', '--', 'python3', '-'], input_text=extraction)
    run('qualification', ['limactl', 'shell', 'warm-arm64', '--', 'python3',
                         GUEST + '/integrations-qualification-v2/linux-integrations-qualification.py'], timeout=86400)
    run('fetch-finished', ['limactl', 'copy', 'warm-arm64:' + GUEST + '/integrations-qualification-v2/finished.json',
                           str(ROOT / 'finished.json')])
    shutil.copy2(ROOT / 'finished.json', CANONICAL / 'integrations-v2-finished.json')
    result = json.loads((ROOT / 'finished.json').read_text())
    return {'complete': result['complete'], 'gatePass': result['gatePass'],
            'completedAt': datetime.now(timezone.utc).isoformat(), 'finishedSha256': digest(ROOT / 'finished.json')}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        with (ROOT / 'launcher-finished.json').open('x') as stream:
            json.dump(result, stream, indent=2, sort_keys=True, allow_nan=False)
            stream.write('\n')
