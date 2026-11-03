"""Build and check an isolated native Python distribution after calibration stops."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time


ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103/python-native-v1')
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103/evidence'
ENV = {**os.environ, 'LIMA_HOME': '/Volumes/A/.hms-lima',
       'CARGO_TARGET_DIR': str(ROOT / 'target'),
       'CARGO_BUILD_JOBS': '2', 'CARGO_PROFILE_RELEASE_STRIP': 'none',
       'RAYON_NUM_THREADS': '2', 'OPENBLAS_NUM_THREADS': '1', 'OMP_NUM_THREADS': '1'}
ENV['PATH'] = str(ROOT.parent / 'tools/uv/bin') + ':' + str(Path.home() / '.cargo/bin') + ':' + ENV['PATH']
ENV.pop('VIRTUAL_ENV', None)
BUILD_PYTHON = subprocess.check_output([str(ROOT.parent / 'tools/uv/bin/uv'), 'python', 'find', '3.12.12'], env=ENV, text=True).strip()


def run(name, args, cwd=ROOT, timeout=7200):
    with (ROOT / (name + '.log')).open('x') as log:
        subprocess.run(args, cwd=cwd, env=ENV, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=timeout)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    prior = json.loads((ROOT.parent / 'native-recovery-v1/finished.json').read_text())
    if prior.get('complete') is not True:
        raise RuntimeError('prior Linux recovery has not passed')
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    source = ROOT / 'source'
    if {name: digest(source / name) for name in freeze['files']} != freeze['files']:
        raise RuntimeError('native source changed before build')
    run('toolchains', ['rustc', '--version'])
    if 'rustc 1.96.0 ' not in (ROOT / 'toolchains.log').read_text():
        raise RuntimeError('unexpected Rust toolchain')
    dist = ROOT / 'dist'
    run('sdist', ['uvx', '--from', 'maturin==1.16.0', 'maturin', 'sdist', '--out', str(dist)], cwd=source)
    archives = list(dist.glob('*.tar.gz'))
    if len(archives) != 1 or archives[0].stat().st_size >= 100000000:
        raise RuntimeError('source distribution missing or over declared size limit')
    unpacked = ROOT / 'sdist-source'
    unpacked.mkdir()
    with tarfile.open(archives[0]) as archive:
        archive.extractall(unpacked, filter='data')
    roots = list(unpacked.iterdir())
    if len(roots) != 1:
        raise RuntimeError('unexpected source distribution layout')
    for name in freeze['files']:
        if name.startswith('src/') and digest(roots[0] / name) != freeze['files'][name]:
            raise RuntimeError('packaged Rust source mismatch: ' + name)
    run('wheel', ['uvx', '--from', 'maturin==1.16.0', 'maturin', 'build', '--release', '--locked',
                  '--interpreter', BUILD_PYTHON, '--out', str(dist)], cwd=roots[0])
    wheels = list(dist.glob('*.whl'))
    if len(wheels) != 1 or 'abi3' not in wheels[0].name or 'aarch64' not in wheels[0].name:
        raise RuntimeError('unexpected wheel ABI/platform')
    consumers = []
    for tag, python, numpy, pytest in [('39', '3.9.25', '2.0.2', '8.3.5'),
                                      ('312', '3.12.12', '2.0.2', '8.3.5'),
                                      ('314', '3.14.7', '2.5.4', '9.2.0')]:
        consumer = ROOT / ('consumer-' + tag)
        consumer.mkdir()
        run('python-install-' + tag, ['uv', 'python', 'install', python])
        python = subprocess.check_output(['uv', 'python', 'find', python], env=ENV, text=True).strip()
        version = subprocess.check_output([python, '-c', 'import platform;print(platform.python_version())'], env=ENV, text=True).strip()
        (consumer / '.python-version').write_text(python + '\n')
        (consumer / 'pyproject.toml').write_text('[project]\nname="warm-native-consumer"\nversion="0.0.0"\nrequires-python="==' + '.'.join(version.split('.')[:2]) + '.*"\ndependencies=["warm-python @ ' + wheels[0].as_uri() + '","numpy==' + numpy + '","pytest==' + pytest + '"]\n')
        run('install-' + tag, ['uv', 'sync', '--project', str(consumer)])
        run('smoke-' + tag, ['uv', 'run', '--project', str(consumer), '--frozen', 'python', '-I',
                            str(source / 'scripts/ci/smoke_python.py'), '--expect-version', '0.6.5'])
        run('tests-' + tag, ['uv', 'run', '--project', str(consumer), '--frozen', 'python', '-I', '-m', 'pytest',
                            '-q', '-rs', '-p', 'no:cacheprovider', str(source / 'tests/python')])
        consumers.append({'python': version, 'numpy': numpy, 'pytest': pytest,
                          'lockSha256': digest(consumer / 'uv.lock')})
    if {name: digest(source / name) for name in freeze['files']} != freeze['files']:
        raise RuntimeError('frozen native source changed')
    return {'complete': True, 'consumers': consumers,
            'artifacts': {path.name: digest(path) for path in archives + wheels},
            'freezeSha256': digest(ROOT / 'freeze.json')}


if __name__ == '__main__':
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        with (ROOT / 'finished.json').open('x') as stream:
            json.dump(result, stream, indent=2)
            stream.write('\n')
