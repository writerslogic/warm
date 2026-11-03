"""Queue evidence audits after timing and retrieve a hash-checked export."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-serving-qualification-20261103-v1')
OUTPUT = ROOT / 'audit-export-v1'
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103/serving-qualification-v1'
CANONICAL = Path('/Volumes/A/holographic-memory/docs/research/milestones-20261102')
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


def run(name, args, timeout=120):
    with (OUTPUT / (name + '.log')).open('xb') as log:
        return subprocess.run(args, env=ENV, stdin=subprocess.DEVNULL,
                              stdout=log, stderr=subprocess.STDOUT, timeout=timeout)


def main():
    request = json.loads((OUTPUT / 'request.json').read_text())
    deadline = time.monotonic() + 86400
    prerequisites = [ROOT / 'execution-launcher-finished.json',
                     Path('/Users/davidcondrey/.cache/warm-serving-isolation-20261103-v1/finished.json')]
    while not all(path.exists() for path in prerequisites):
        if time.monotonic() > deadline:
            raise TimeoutError('timing prerequisites did not finish')
        time.sleep(30)
    if not all(json.loads(path.read_text()).get('complete') is True for path in prerequisites):
        raise ValueError('serving or host observer execution failed')
    source = OUTPUT / 'linux-serving-audit-export.py'
    if digest(source) != request['guestRunnerSha256']:
        raise ValueError('queued audit runner changed')
    run('transfer-runner', ['limactl', 'copy', str(source), 'warm-arm64:' + GUEST + '/' + source.name]).check_returncode()
    completed = run('guest-audit', ['limactl', 'shell', 'warm-arm64', '--', 'python3',
                                  GUEST + '/' + source.name], timeout=72000)
    run('fetch-finished', ['limactl', 'copy', 'warm-arm64:' + GUEST + '/audit-export-v1/finished.json',
                           str(OUTPUT / 'guest-finished.json')]).check_returncode()
    terminal = json.loads((OUTPUT / 'guest-finished.json').read_text())
    if completed.returncode or terminal.get('complete') is not True:
        raise RuntimeError('guest audit/export did not complete; its logs and failures are retained')
    archive = OUTPUT / 'serving-evidence-v1.tar.gz'
    run('fetch-archive', ['limactl', 'copy', 'warm-arm64:' + GUEST + '/audit-export-v1/' + archive.name,
                         str(archive)], timeout=1800).check_returncode()
    if archive.stat().st_size != terminal['archiveBytes'] or digest(archive) != terminal['archiveSha256']:
        raise ValueError('export archive digest or size mismatch')
    extracted = OUTPUT / 'evidence'
    extracted.mkdir()
    with tarfile.open(archive) as packed:
        members = packed.getmembers()
        if len(members) > 20000 or sum(member.size for member in members) > 4 * 2**30:
            raise ValueError('export allocation bound exceeded')
        names = set()
        for member in members:
            path = Path(member.name)
            if (not member.isfile() or path.is_absolute() or '..' in path.parts
                    or not 0 <= member.size <= 64 * 2**20 or member.name in names):
                raise ValueError('invalid archive member')
            names.add(member.name)
        packed.extractall(extracted, filter='data')
    manifest_path = extracted / 'audit-export-v1/manifest.json'
    if digest(manifest_path) != terminal['manifestSha256']:
        raise ValueError('manifest digest mismatch')
    manifest = json.loads(manifest_path.read_text())
    if names != {*manifest['files'], 'audit-export-v1/manifest.json'}:
        raise ValueError('export inventory mismatch')
    for name, expected in manifest['files'].items():
        path = extracted / name
        if path.stat().st_size != expected['bytes'] or digest(path) != expected['sha256']:
            raise ValueError('exported evidence changed: ' + name)
    shutil.copy2(OUTPUT / 'guest-finished.json', CANONICAL / 'serving-v1-raw-audits-finished.json')
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'archiveSha256': terminal['archiveSha256'], 'filesVerified': len(manifest['files']),
            'allRawAuditsPassed': terminal['allRawAuditsPassed'],
            'allQualityPassed': terminal['allQualityPassed'], 'claimQualified': False}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(OUTPUT / 'finished.json', result)
        shutil.copy2(OUTPUT / 'finished.json', CANONICAL / 'serving-v1-audit-export-finished.json')
