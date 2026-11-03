"""Build and check the current local model features using existing pinned weights."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time

REPO = Path('/Volumes/A/holographic-memory')
CANONICAL = REPO / 'docs/research/milestones-20261102'
ROOT = Path('/Volumes/A/.hms-bench-cache/warm-local-models-qualification-20261103-v1')
BASIS = ROOT.with_name('warm-core-qualification-20261103-v2')
INTEGRATIONS = ROOT.with_name('warm-integrations-qualification-20261103-v1')
SOURCE = ROOT / 'source'
FEATURES = 'security,conversation-memory,local-models,server,cli,mimalloc'
ENV = {name: value for name, value in os.environ.items()
       if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV.update({'CARGO_HOME': '/Users/davidcondrey/.local/share/cargo',
            'RUSTUP_HOME': '/Users/davidcondrey/.local/share/rustup',
            'CARGO_TARGET_DIR': str(ROOT / 'target'), 'CARGO_BUILD_JOBS': '2',
            'RAYON_NUM_THREADS': '2', 'HF_HUB_OFFLINE': '1',
            'WARM_CONVERSATION_MODEL_DIR': '/Volumes/A/hms-models/Qwen3-Embedding-0.6B'})


def digest(path, algorithm='sha256'):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, algorithm).hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def run(name, args, timeout=7200):
    with (ROOT / (name + '.log')).open('xb') as log:
        child = subprocess.Popen(args, cwd=SOURCE, env=ENV, stdout=log,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=30)
    if code:
        raise RuntimeError(name + ' failed with exit ' + str(code))


def verify(freeze):
    for name, expected in freeze['files'].items():
        if digest(SOURCE / name) != expected:
            raise ValueError('local model source changed: ' + name)


def main():
    request = json.loads((ROOT / 'preparation-request.json').read_text())
    deadline = time.monotonic() + 172800
    while not (INTEGRATIONS / 'preparation-finished.json').exists():
        if time.monotonic() > deadline:
            raise TimeoutError('integration qualification did not terminate within 48 hours')
        time.sleep(30)
    terminal = json.loads((INTEGRATIONS / 'preparation-finished.json').read_text())
    if 'error' in terminal:
        raise ValueError('integration launcher failure requires inspection before model work')
    for path in (ROOT.with_name('warm-serving-qualification-20261103-v1') / 'execution-launcher-finished.json',
                 Path('/Users/davidcondrey/.cache/warm-serving-isolation-20261103-v1/finished.json')):
        if json.loads(path.read_text()).get('complete') is not True:
            raise ValueError('serving execution/observations not complete')
    if digest(BASIS / 'freeze.json') != request['basisFreezeSha256']:
        raise ValueError('core source basis changed')
    basis = json.loads((BASIS / 'freeze.json').read_text())
    SOURCE.mkdir()
    for name, expected in basis['files'].items():
        if digest(BASIS / 'source' / name) != expected:
            raise ValueError('core source file changed: ' + name)
        target = SOURCE / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(BASIS / 'source' / name, target)
    for name, expected in request['testFiles'].items():
        if digest(REPO / name) != expected:
            raise ValueError('queued model test changed: ' + name)
        target = SOURCE / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists() and digest(target) != expected:
            raise ValueError('model test would change source basis')
        shutil.copy2(REPO / name, target)
        basis['files'][name] = expected
    weights = {}
    for name, model in request['models'].items():
        directory = Path(model['directory'])
        if {path.name for path in directory.glob('*.safetensors')} != {filename for filename in model['files'] if filename.endswith('.safetensors')}:
            raise ValueError('model weight file inventory changed: ' + name)
        files = {}
        for filename, record in model['files'].items():
            path = directory / filename
            metadata = directory / '.cache/huggingface/download' / (filename + '.metadata')
            if path.stat().st_size != record['bytes'] or metadata.read_text().splitlines()[:2] != [model['revision'], record['etag']]:
                raise ValueError('model metadata or size changed: ' + name + '/' + filename)
            sha = digest(path)
            if len(record['etag']) == 64:
                if sha != record['etag']:
                    raise ValueError('model LFS digest differs: ' + name + '/' + filename)
            elif len(record['etag']) == 40:
                hashed = hashlib.sha1(b'blob ' + str(path.stat().st_size).encode() + b'\0')
                with path.open('rb') as stream:
                    for block in iter(lambda: stream.read(2**20), b''):
                        hashed.update(block)
                if hashed.hexdigest() != record['etag']:
                    raise ValueError('model Git blob digest differs: ' + name + '/' + filename)
            else:
                raise ValueError('unsupported model etag')
            files[filename] = {'bytes': path.stat().st_size, 'sha256': sha, 'metadataSha256': digest(metadata)}
        weights[name] = {'revision': model['revision'], 'directory': str(directory), 'files': files}
    freeze = {'frozenAt': datetime.now(timezone.utc).isoformat(), 'basisFreezeSha256': request['basisFreezeSha256'],
              'files': basis['files'], 'models': weights, 'features': FEATURES,
              'scope': 'Current native model-feature build and existing model/conversation tests; no QA advantage or server lifecycle claim.'}
    write(ROOT / 'freeze.json', freeze)
    shutil.copy2(ROOT / 'freeze.json', CANONICAL / 'local-models-v1-freeze.json')
    verify(freeze)
    cargo = ['cargo', '+1.96.0']
    run('build', [*cargo, 'build', '--release', '--locked', '--features', FEATURES,
                  '--bin', 'warm-server', '--bin', 'public-bench'])
    run('model-unit-tests', [*cargo, 'test', '--release', '--locked', '--features', FEATURES,
                            '--lib', 'core::models::', '--', '--test-threads=1'])
    run('conversation-model-tests', [*cargo, 'test', '--release', '--locked', '--features', FEATURES,
                                    '--test', 'conversation_models', '--', '--include-ignored', '--test-threads=1'])
    summaries = {}
    for name in ('model-unit-tests', 'conversation-model-tests'):
        log = (ROOT / (name + '.log')).read_text()
        matches = re.findall(r'^test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;', log, re.M)
        if not matches or int(matches[-1][0]) == 0 or int(matches[-1][1]) != 0:
            raise ValueError('model test suite empty or contains skips: ' + name)
        summaries[name] = {'passed': int(matches[-1][0]), 'logSha256': digest(ROOT / (name + '.log'))}
    if summaries['conversation-model-tests']['passed'] != 2:
        raise ValueError('both declared model conversation tests must execute')
    verify(freeze)
    for name, model in weights.items():
        directory = Path(model['directory'])
        if {path.name for path in directory.glob('*.safetensors')} != {filename for filename in model['files'] if filename.endswith('.safetensors')}:
            raise ValueError('model weight inventory changed during checks: ' + name)
        for filename, expected in model['files'].items():
            metadata = directory / '.cache/huggingface/download' / (filename + '.metadata')
            if digest(directory / filename) != expected['sha256'] or digest(metadata) != expected['metadataSha256']:
                raise ValueError('model file or metadata changed during checks: ' + name + '/' + filename)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(), 'tests': summaries,
            'freezeSha256': digest(ROOT / 'freeze.json'),
            'binaries': {name: digest(ROOT / 'target/release' / name) for name in ('warm-server', 'public-bench')},
            'paidServicesUsed': False,
            'pending': ['HTTP local-model lifecycle and recovery', 'Full generator/reranker workflows',
                        'Real memory utility', 'Other platforms and devices']}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(ROOT / 'finished.json', result)
        shutil.copy2(ROOT / 'finished.json', CANONICAL / 'local-models-v1-finished.json')
