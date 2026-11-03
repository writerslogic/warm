"""Stage a frozen Linux C-family check after the macOS artifact/recovery checks."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


ROOT = Path('/Volumes/A/.hms-bench-cache/warm-linux-c-family-qualification-20261103-v2')
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103/c-family-v2'
FAMILY = Path('/Volumes/A/.hms-bench-cache/warm-c-family-qualification-20261103-v3/finished.json')
RECOVERY = Path('/Volumes/A/.hms-bench-cache/warm-native-binding-recovery-20261103-v3/finished.json')
ENV = {name: value for name, value in os.environ.items() if name in ('PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL')}
ENV['LIMA_HOME'] = '/Volumes/A/.hms-lima'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    deadline = time.monotonic() + 172800
    while not FAMILY.exists() or not RECOVERY.exists():
        if time.monotonic() >= deadline:
            raise TimeoutError('macOS checks still pending after 48 hours')
        time.sleep(60)
    prior = {'macFamily': json.loads(FAMILY.read_text()), 'macRecovery': json.loads(RECOVERY.read_text())}
    if any(value.get('complete') is not True for value in prior.values()):
        raise ValueError('macOS C-family/recovery failed; inspect before staging the Linux follow-up')
    transfer = json.loads((ROOT / 'transfer.json').read_text())
    if digest(ROOT / 'transfer.tar') != transfer['sha256']:
        raise ValueError('frozen transfer changed')
    if digest(ROOT / 'launch.py') != transfer['coordinatorSha256']:
        raise ValueError('coordinator changed')
    with (ROOT / 'stage.log').open('xb') as log:
        subprocess.run(['limactl', 'shell', 'warm-arm64', '--', 'test', '-f',
                        '/home/davidcondrey.guest/warm-milestones-20261103/evidence/sdk-frameworks-v1/finished.json'],
                       env=ENV, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=30)
        remote_archive = '/home/davidcondrey.guest/warm-milestones-20261103/c-family-v2-transfer.tar'
        subprocess.run(['limactl', 'copy', str(ROOT / 'transfer.tar'), 'warm-arm64:' + remote_archive],
                       env=ENV, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=900)
        code = '''
from pathlib import Path
import hashlib,json,subprocess,tarfile
root=Path(GUEST)
archive=Path(ARCHIVE)
with archive.open('rb') as stream:
    assert hashlib.file_digest(stream,'sha256').hexdigest()==EXPECTED
root.mkdir()
with tarfile.open(archive) as stream:stream.extractall(root,filter='data')
freeze=json.loads((root/'freeze.json').read_text())
for tool in freeze['tools']:
    path=root/'downloads'/tool['name']
    with path.open('rb') as stream:
        assert hashlib.file_digest(stream,tool['algorithm']).hexdigest()==tool['digest']
    destination=root/'tools'
    if tool['name'].startswith('dotnet-sdk-'):destination=destination/'dotnet'
    destination.mkdir(parents=True,exist_ok=True)
    with tarfile.open(path) as stream:stream.extractall(destination,filter='data')
(root/'prior-work-terminal.json').write_text(json.dumps(PRIOR,indent=2)+'\\n')
with (root/'launcher.log').open('xb') as log:
    process=subprocess.Popen(['/usr/bin/python3','-u',str(root/'run.py')],cwd=root,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
(root/'launcher.pid').write_text(str(process.pid)+'\\n')
print(json.dumps({'pid':process.pid,'root':str(root)}))
'''
        bindings = 'GUEST=' + repr(GUEST) + '\nARCHIVE=' + repr(remote_archive) + '\nEXPECTED=' + repr(transfer['sha256']) + '\nPRIOR=' + repr(prior) + '\n'
        result = subprocess.check_output(['limactl', 'shell', 'warm-arm64', '--', 'python3', '-c', bindings + code], env=ENV, timeout=300, text=True)
    return {'launched': True, 'guest': json.loads(result), 'transferSha256': transfer['sha256']}


if __name__ == '__main__':
    try:
        result = main()
    except Exception as error:
        result = {'launched': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        with (ROOT / 'launch-finished.json').open('x') as stream:
            json.dump(result, stream, indent=2)
            stream.write('\n')
