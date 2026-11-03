"""Validate and build the promoted bounded dense-search source in the arm64 guest."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess

ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103/dense-product-v2')
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV['PATH'] = '/home/davidcondrey.guest/.cargo/bin:' + ENV['PATH']
ENV.update({'CARGO_TARGET_DIR': '/home/davidcondrey.guest/warm-milestones-20261103/target', 'CARGO_BUILD_JOBS': '2'})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def verify(freeze):
    for name, expected in freeze['files'].items():
        if digest(ROOT / 'source' / name) != expected:
            raise ValueError('source changed: ' + name)


def run(name, args, timeout=7200):
    with (ROOT / (name + '.log')).open('xb') as stream:
        process = subprocess.Popen(args, cwd=ROOT / 'source', env=ENV, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
            if code:
                raise RuntimeError(name + ' failed with exit ' + str(code))
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    return (ROOT / (name + '.log')).read_text()


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    verify(freeze)
    version = subprocess.check_output(['rustc', '--version'], env=ENV, text=True).strip()
    if not version.startswith('rustc 1.96.0 '):
        raise ValueError('unexpected Rust version')
    if digest(ROOT / 'document-tests.log') != freeze['linuxDocumentTestsSha256']:
        raise ValueError('inherited document evidence changed')
    documents = (ROOT / 'document-tests.log').read_text()
    if 'test result: ok. 154 passed; 0 failed; 1 ignored;' not in documents:
        raise ValueError('unexpected document test count')
    server = run('server-tests', ['cargo', 'test', '--locked', '--bin', 'warm-server', '--features',
                                 'server,security', '--', '--test-threads=1'])
    totals = re.findall(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;', server)
    if not totals or int(totals[-1][0]) < 47 or int(totals[-1][1]) != 0:
        raise ValueError('incomplete server tests or unexplained ignores')
    run('server-build', ['cargo', 'build', '--locked', '--release', '--bin', 'warm-server', '--features', 'server,security'])
    source = Path(ENV['CARGO_TARGET_DIR']) / 'release/warm-server'
    binary = ROOT / 'warm-server'
    with source.open('rb') as src, binary.open('xb') as dest:
        import shutil
        shutil.copyfileobj(src, dest)
    binary.chmod(0o755)
    verify(freeze)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(), 'rust': version,
            'documentTests': 154, 'documentIgnored': ['lexical_latency_microbench'],
            'serverTests': int(totals[-1][0]), 'serverIgnored': 0,
            'binarySha256': digest(binary), 'freezeSha256': digest(ROOT / 'freeze.json'),
            'runnerSha256': digest(ROOT / 'run.py')}


if __name__ == '__main__':
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        with (ROOT / 'finished.json').open('x') as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write('\n')
