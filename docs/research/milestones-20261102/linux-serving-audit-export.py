"""Audit the frozen serving jobs and export their evidence without server stores."""
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import tarfile

ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103/serving-qualification-v1')
OUTPUT = ROOT / 'audit-export-v1'
spec = importlib.util.spec_from_file_location('serving_seal', ROOT / 'linux-serving-seal.py')
seal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(seal)


def main():
    terminal = json.loads((ROOT / 'finished.json').read_text())
    if terminal.get('complete') is not True or len(terminal['jobs']) != 18:
        raise ValueError('all eighteen frozen jobs must finish before auditing')
    seal.verify()
    OUTPUT.mkdir()
    seal.write(OUTPUT / 'started.json', {
        'startedAt': datetime.now(timezone.utc).isoformat(),
        'runnerSha256': seal.digest(Path(__file__)),
        'executionFinishedSha256': seal.digest(ROOT / 'finished.json'),
        'sourceFreezeSha256': seal.digest(ROOT / 'freeze.json')})
    results = []
    for ordinal in range(18):
        name = f'job-{ordinal:02d}'
        arguments = [*seal.PYTHON, 'benchmarks/public/audit_serving_job.py',
                     '--root', str(ROOT / 'measurements'),
                     '--output', str(OUTPUT / name),
                     '--seal', str(ROOT / 'selection/configuration-seal.json'),
                     '--inputs', str(ROOT / 'inputs'), '--job', str(ordinal)]
        with (OUTPUT / (name + '.log')).open('xb') as log:
            child = subprocess.Popen(arguments, cwd=seal.SOURCE, env=seal.ENV,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = child.wait(timeout=3600)
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=30)
        result = {'job': ordinal, 'exitCode': code, 'logSha256': seal.digest(OUTPUT / (name + '.log'))}
        receipt = OUTPUT / name / 'finished.json'
        if receipt.exists():
            data = json.loads(receipt.read_text())
            result.update({key: data[key] for key in ('complete', 'rawJobEvidenceVerified', 'qualityPassed', 'resourcesPassed')})
            result['finishedSha256'] = seal.digest(receipt)
        results.append(result)
        seal.write(OUTPUT / (name + '-receipt.json'), result)
        print(json.dumps(result), flush=True)
    seal.verify()
    candidates = [ROOT / name for name in ('finished.json', 'freeze.json', 'sealed.json',
                  'host-observation-started.json', 'selection/configuration-seal.json')]
    candidates.extend(sorted(ROOT.glob('job-*.log')))
    candidates.extend(sorted(ROOT.glob('job-*-accepted.json')))
    for directory in (ROOT / 'measurements', OUTPUT):
        for folder, dirs, files in os.walk(directory, followlinks=False):
            dirs[:] = sorted(name for name in dirs if name != 'server')
            if any((Path(folder) / name).is_symlink() for name in dirs):
                raise ValueError('linked evidence directory')
            candidates.extend(Path(folder) / name for name in sorted(files))
    files = {}
    for path in sorted(candidates):
        name = path.relative_to(ROOT).as_posix()
        if path.is_symlink() or not path.is_file() or path.suffix not in ('.json', '.ndjson', '.log'):
            raise ValueError('unexpected evidence file: ' + name)
        size = path.stat().st_size
        if size > 64 * 2**20 or name in files:
            raise ValueError('oversized or duplicate evidence file: ' + name)
        files[name] = {'bytes': size, 'sha256': seal.digest(path)}
    if sum(item['bytes'] for item in files.values()) > 4 * 2**30:
        raise ValueError('evidence export exceeds four GiB')
    manifest = {'createdAt': datetime.now(timezone.utc).isoformat(), 'files': files,
                'omitted': 'Server stores/configuration/log directories remain on the guest; recorded engine settings are included in job receipts.',
                'sourceFreezeSha256': seal.digest(ROOT / 'freeze.json'), 'jobs': results,
                'claimQualified': False, 'independenceVerified': False}
    seal.write(OUTPUT / 'manifest.json', manifest)
    archive = OUTPUT / 'serving-evidence-v1.tar.gz'
    with tarfile.open(archive, 'x:gz', compresslevel=1) as packed:
        for name in sorted(files):
            packed.add(ROOT / name, arcname=name, recursive=False)
        packed.add(OUTPUT / 'manifest.json', arcname='audit-export-v1/manifest.json', recursive=False)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'allRawAuditsPassed': all(item.get('rawJobEvidenceVerified') is True and item['exitCode'] == 0 for item in results),
            'allQualityPassed': all(item.get('qualityPassed') is True for item in results),
            'jobs': results, 'manifestSha256': seal.digest(OUTPUT / 'manifest.json'),
            'archiveSha256': seal.digest(archive), 'archiveBytes': archive.stat().st_size,
            'claimQualified': False, 'independenceVerified': False}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        if OUTPUT.is_dir():
            seal.write(OUTPUT / 'finished.json', result)
