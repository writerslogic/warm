"""After the guest restarts, qualify DSPy and seal the serving configurations."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-serving-qualification-20261103-v1')
DSPY = Path('/Volumes/A/.hms-bench-cache/warm-dspy-qualification-20261103-v1')
BASE = '/home/davidcondrey.guest/warm-milestones-20261103'
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV['LIMA_HOME'] = '/Volumes/A/.hms-lima'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def run(name, arguments, timeout=7200, input_text=None):
    with (ROOT / (name + '.log')).open('x') as log:
        subprocess.run(arguments, env=ENV, input=input_text, text=True, stdout=log,
                       stderr=subprocess.STDOUT, timeout=timeout, check=True)


def main():
    deadline = time.monotonic() + 3600
    while True:
        state = json.loads(subprocess.check_output(['limactl', 'list', 'warm-arm64', '--format', 'json'],
                                                   env=ENV, text=True, timeout=30))
        if state['status'] == 'Running':
            ready = subprocess.run(['limactl', 'shell', 'warm-arm64', '--', 'true'], env=ENV,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
            if ready.returncode == 0:
                break
        if time.monotonic() > deadline:
            raise TimeoutError('guest did not restart within one hour')
        time.sleep(30)
    inspection = '''import json,os
from pathlib import Path
p=Path('/home/davidcondrey.guest/warm-milestones-20261103')
v=os.statvfs(p)
memory={line.split(':')[0]:int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith(('MemTotal:','SwapTotal:'))}
record={'freeBytes':v.f_bavail*v.f_frsize,'cpuCount':os.cpu_count(),'memory':memory}
print(json.dumps(record))
assert record['freeBytes']>=21*2**30,'guest filesystem still too small'
assert record['cpuCount']==4 and 7*2**30 < memory['MemTotal']<=8*2**30 and memory['SwapTotal']==0
'''
    run('guest-admission', ['limactl', 'shell', 'warm-arm64', '--', 'python3', '-'], timeout=60, input_text=inspection)
    bundles = [(ROOT / 'run-bundle.tar', 'serving-v1.tar', 'serving-qualification-v1',
                'edb4ce972277910a242ee0d3a5903c79b3e561567676ebbf3df7cf0dc3b8b867'),
               (DSPY / 'run-bundle.tar', 'dspy-v1.tar', 'dspy-qualification-v1',
                'd3d4f8737f55ae9719f449cb2f0fc6543832309ebc5ab870f39e247e312faa46')]
    for local, remote, directory, expected in bundles:
        if digest(local) != expected:
            raise ValueError('bundle changed: ' + local.name)
        run('transfer-' + directory, ['limactl', 'copy', str(local), 'warm-arm64:' + BASE + '/' + remote], timeout=120)
        extraction = '''from pathlib import Path
import hashlib,tarfile
base=Path(%r);archive=base/%r;destination=base/%r
with archive.open('rb') as f:assert hashlib.file_digest(f,'sha256').hexdigest()==%r
destination.mkdir()
with tarfile.open(archive) as packed:
    members=packed.getmembers()
    assert len(members)<=300
    for member in members:
        name=Path(member.name)
        assert member.isfile() and not name.is_absolute() and '..' not in name.parts and 0<=member.size<=4*2**20
    packed.extractall(destination,filter='data')
print('Verified and extracted',len(members),'files.')
''' % (BASE, remote, directory, expected)
        run('extract-' + directory, ['limactl', 'shell', 'warm-arm64', '--', 'python3', '-'], timeout=120, input_text=extraction)
    run('dspy-qualification', ['limactl', 'shell', 'warm-arm64', '--', 'python3',
                               BASE + '/dspy-qualification-v1/linux-dspy-qualification.py'])
    run('serving-seal', ['limactl', 'shell', 'warm-arm64', '--', 'python3',
                          BASE + '/serving-qualification-v1/linux-serving-seal.py'])
    for remote, local in [('dspy-qualification-v1/results/finished.json', 'dspy-finished.json'),
                          ('serving-qualification-v1/sealed.json', 'sealed.json'),
                          ('serving-qualification-v1/selection/configuration-seal.json', 'configuration-seal.json')]:
        run('fetch-' + local, ['limactl', 'copy', 'warm-arm64:' + BASE + '/' + remote, str(ROOT / local)], timeout=120)
    if not all(json.loads((ROOT / name).read_text()).get('complete') is True for name in ('dspy-finished.json', 'sealed.json')):
        raise ValueError('incomplete preflight receipt')
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'heldOutVectorsOpened': False, 'configurationSealSha256': digest(ROOT / 'configuration-seal.json'),
            'dspyFinishedSha256': digest(ROOT / 'dspy-finished.json'),
            'scope': 'Preparation complete; held-out measurements and host/guest observers have not started.'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        with (ROOT / 'preflight-finished.json').open('x') as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write('\n')
