"""Package and verify the frozen wholly withdrawable witness result."""
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

REPO = Path(__file__).resolve().parents[3]
DOCS = Path(__file__).resolve().parent
CACHE = Path('/Volumes/A/.hms-bench-cache')
ORIGINAL = CACHE / 'warm-predictive-matrix-witness-20261103-v1'
ROOT = CACHE / 'warm-matrix-witness-reproduction-20261103-v1'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def main():
    freeze = json.loads((ORIGINAL / 'freeze.json').read_text())
    result = json.loads((ORIGINAL / 'finished.json').read_text())
    assert result['complete'] and result['gatePass']
    assert digest(ORIGINAL / 'freeze.json') == result['freezeSha256']
    for name, expected in freeze['files'].items():
        assert digest(ORIGINAL / 'source' / name) == expected, name
    for name, expected in result['outputs'].items():
        assert digest(ORIGINAL / name) == expected, name
    ROOT.mkdir(exist_ok=False)
    bundle = ROOT / 'bundle'
    source = bundle / 'benchmarks/research/source_prediction'
    source.mkdir(parents=True)
    for name in freeze['files']:
        shutil.copyfile(ORIGINAL / 'source' / name, source / name)
    for relative in ('benchmarks/research/source_prediction/reproduce_witness.py',
                     'benchmarks/research/temporal_trace/reproduce.py',
                     'docs/research/milestones-20261102/MATRIX-WITNESS.md',
                     'docs/research/milestones-20261102/WITNESS-TIGHTNESS.md',
                     'docs/research/milestones-20261102/SOURCE-PREDICTION.md'):
        destination = bundle / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(REPO / relative, destination)
    (bundle / 'original').mkdir()
    for name in ('freeze.json', 'finished.json', *result['outputs']):
        shutil.copyfile(ORIGINAL / name, bundle / 'original' / name)
    (bundle / 'README.md').write_text('''# Wholly withdrawable deletion-bound reproduction

Read docs/research/milestones-20261102/MATRIX-WITNESS.md for the proof and scope.
From this extracted directory run:

```sh
uv run --no-project --python 3.12.12 python benchmarks/research/source_prediction/reproduce_witness.py --bundle "$PWD" --output "$PWD/replication-001"
```

The output directory must not exist. No model, paid service or corpus is needed.
uv may obtain the specified free Python runtime. Expected: five families, 37,448
exact comparisons, 62 distinguishing masks, 17,841 jointly unknown masks, three
existing-checker cases and three focused tests. Seven deterministic evidence
files must match byte-for-byte. Timing and Python version may vary.

This proves the stated mathematical bound, not a system advantage or established
novelty. Local packaging verification is not independent evaluation. Preserve the
whole output directory, including failures, and report machine/runtime details
and the verified archive SHA256 when returning the independent results.
''')
    expected = {key: result[key] for key in ('complete', 'gatePass', 'cases',
                'readoutComparisons', 'jointUnknownMasks', 'distinguishingMasks',
                'existingCheckerCases', 'freezeSha256')}
    manifest = {'schema': 'warm.matrix-witness-reproduction/1',
                'claim': 'Sharp wholly withdrawable positive-evidence bound; no system advantage or established novelty',
                'files': {str(path.relative_to(bundle)): digest(path)
                          for path in sorted(bundle.rglob('*')) if path.is_file()},
                'expected': {'result': expected, 'outputs': {
                    name: checksum for name, checksum in result['outputs'].items() if name != 'tests.log'}}}
    write(bundle / 'manifest.json', manifest)
    archive = ROOT / 'matrix-witness-reproduction-v1.tar.gz'
    with tarfile.open(archive, 'w:gz') as stream:
        for path in sorted(bundle.rglob('*')):
            if path.is_file():
                stream.add(path, arcname=str(path.relative_to(bundle)), recursive=False)
    extracted = ROOT / 'packaging-check'
    extracted.mkdir()
    with tarfile.open(archive) as stream:
        stream.extractall(extracted, filter='data')
    command = [sys.executable, 'benchmarks/research/source_prediction/reproduce_witness.py',
               '--bundle', str(extracted), '--output', str(extracted / 'replication-001')]
    with (ROOT / 'packaging-check.log').open('xb') as log:
        subprocess.run(command, cwd=extracted, stdout=log, stderr=subprocess.STDOUT,
                       timeout=600, check=True)
    reproduction = json.loads((extracted / 'replication-001/reproduction.json').read_text())
    assert reproduction['complete'] and reproduction['matchedOutputs'] == 7
    tampered = ROOT / 'integrity-check'
    shutil.copytree(bundle, tampered)
    with (tampered / 'benchmarks/research/source_prediction/matrix_witness.py').open('ab') as stream:
        stream.write(b'\n')
    command = [sys.executable, 'benchmarks/research/source_prediction/reproduce_witness.py',
               '--bundle', str(tampered), '--output', str(tampered / 'replication-001')]
    with (ROOT / 'integrity-check.log').open('xb') as log:
        rejected = subprocess.run(command, cwd=tampered, stdout=log,
                                  stderr=subprocess.STDOUT, timeout=60)
    assert rejected.returncode != 0 and not (tampered / 'replication-001').exists()
    assert 'source bundle differs from its manifest' in (ROOT / 'integrity-check.log').read_text()
    receipt = {'complete': True, 'checkedAt': datetime.now(timezone.utc).isoformat(),
               'independenceVerified': False, 'archive': str(archive),
               'archiveBytes': archive.stat().st_size, 'archiveSha256': digest(archive),
               'files': len(manifest['files']), 'manifestSha256': digest(bundle / 'manifest.json'),
               'reproduction': reproduction, 'tamperedSourceRejectedBeforeRun': True}
    write(ROOT / 'finished.json', receipt)
    write(DOCS / 'matrix-witness-reproduction-check.json', receipt)
    shutil.copyfile(ROOT / 'packaging-check.log', DOCS / 'matrix-witness-reproduction-check.log')
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
