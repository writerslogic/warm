"""Qualify a frozen installed WebAssembly package across its local runtimes."""
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tarfile

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-wasm-qualification-20261103-v4')
WORK = ROOT / 'work'
PACKAGE = WORK / 'bindings/wasm/npm'
NODE = Path('/Users/davidcondrey/.cache/warm-qualification-toolchains/node-v22.23.4-darwin-arm64/bin/node')
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME')}
ENV['PATH'] = str(NODE.parent) + ':' + str(ROOT / 'tools/bin') + ':' + str(ROOT / 'tools/Firefox.app/Contents/MacOS') + ':' + ENV['PATH']
ENV.update({'WARM_WASM_TARGET_DIR': '/Users/davidcondrey/.cache/warm-wasm-qualification-target',
            'CARGO_BUILD_JOBS': '2', 'BINARYEN_CORES': '2',
            'WARM_CHROME': '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
            'WARM_FIREFOX': str(ROOT / 'tools/Firefox.app/Contents/MacOS/firefox'),
            'MOZ_HEADLESS': '1', 'WASM_BINDGEN_TEST_TIMEOUT': '180'})
RUNTIMES = {'node': [str(NODE)], 'deno': ['/opt/homebrew/bin/deno', 'run', '--allow-read', '--allow-write', '--allow-env'],
            'bun': [str(ROOT / 'tools/bin/bun')]}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, data):
    with path.open('x') as stream:
        json.dump(data, stream, indent=2, sort_keys=True)
        stream.write('\n')


def verify(freeze):
    for name, expected in freeze['files'].items():
        for base in (ROOT / 'source', WORK):
            if digest(base / name) != expected:
                raise ValueError('frozen file changed: ' + name)
    for name, expected in freeze['tools'].items():
        if digest(Path(name)) != expected:
            raise ValueError('tool changed: ' + name)
    if digest(ROOT / 'run.py') != freeze['runnerSha256']:
        raise ValueError('runner changed')


def run(name, args, cwd=PACKAGE, timeout=3600):
    with (ROOT / (name + '.log')).open('xb') as stream:
        process = subprocess.Popen([str(x) for x in args], cwd=cwd, env=ENV,
                                   stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
            if code:
                raise RuntimeError(name + ' failed with exit ' + str(code))
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    return (ROOT / (name + '.log')).read_text()


def recovery(consumer):
    spec = importlib.util.spec_from_file_location('native_recovery_protocol', WORK / 'benchmarks/public/native_recovery/run.py')
    protocol = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(protocol)
    program = consumer / 'recovery.mjs'
    shutil.copy2(WORK / 'benchmarks/public/wasm_qualification/recovery.mjs', program)
    deno_consumer = ROOT / 'runtime-consumers/deno'
    shutil.copy2(program, deno_consumer / 'recovery.mjs')
    output = ROOT / 'recovery'
    output.mkdir()
    cases = []
    for runtime, command in RUNTIMES.items():
        for encryption in ('plain', 'pbkdf2', 'argon2id'):
            for death in (signal.SIGKILL, signal.SIGTERM):
                directory = output / f'{runtime}-{encryption}-{death.name}'
                directory.mkdir()
                store = directory / 'writer-store'
                writer_consumer = deno_consumer if runtime == 'deno' else consumer
                writer_program = writer_consumer / 'recovery.mjs'
                with (directory / 'writer.stderr').open('xb') as errors, (directory / 'ledger.ndjson').open('x') as ledger:
                    process = subprocess.Popen(command + [str(writer_program), 'writer', str(store), encryption],
                                               cwd=writer_consumer, env=ENV, stdout=subprocess.PIPE, stderr=errors,
                                               start_new_session=True)
                    try:
                        protocol.ready(process, ledger)
                        if process.poll() is not None:
                            raise ValueError('writer closed before deliberate death')
                        os.killpg(process.pid, death)
                        code = process.wait(timeout=30)
                        if code != -death:
                            raise ValueError('unexpected writer exit status: ' + str(code))
                    finally:
                        if process.poll() is None:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait(timeout=30)
                        process.stdout.close()
                for reader, reader_command in RUNTIMES.items():
                    read_store = directory / (reader + '-reader-store')
                    shutil.copytree(store, read_store)
                    name = f'recovery-{runtime}-{encryption}-{death.name}-{reader}'
                    reader_consumer = deno_consumer if reader == 'deno' else consumer
                    log = run(name, reader_command + [str(reader_consumer / 'recovery.mjs'), 'reader', str(read_store), encryption], reader_consumer, 180)
                    if json.loads(log) != protocol.VERIFIED:
                        raise ValueError('reader receipt differs')
                cases.append({'writer': runtime, 'encryption': encryption, 'signal': death.name,
                              'readers': list(RUNTIMES), 'ledgerSha256': digest(directory / 'ledger.ndjson')})
    write(ROOT / 'recovery.json', cases)
    return cases


def main():
    freeze = json.loads((ROOT / 'freeze.json').read_text())
    verify(freeze)
    prior = ROOT.parent / 'warm-wasm-qualification-20261103-v1'
    if digest(prior / 'freeze.json') != freeze['reusedBuild']['sourceFreezeSha256']:
        raise ValueError('prior build source receipt changed')
    for name, expected in freeze['reusedBuild']['artifacts'].items():
        if digest(PACKAGE / name) != expected or digest(prior / 'work/bindings/wasm/npm' / name) != expected:
            raise ValueError('reused build artifact changed: ' + name)
    verify(freeze)
    pack = run('pack', [NODE, 'scripts/pack-check.mjs', ROOT / 'packed'])
    if 'skipped' in pack.lower() or any(name + ': ok ' not in pack for name in RUNTIMES) or 'types:' not in pack:
        raise ValueError('packed consumer skipped a required runtime or type check')
    consumer = ROOT / 'packed/consumer'
    installed = consumer / 'node_modules/@writerslogic/warm-wasm'
    archives = list((ROOT / 'packed').glob('*.tgz'))
    if len(archives) != 1:
        raise ValueError('expected exactly one packed archive')
    packed_hashes = {}
    with tarfile.open(archives[0]) as archive:
        members = [member for member in archive.getmembers() if member.isfile()]
        for member in members:
            path = Path(member.name)
            if not path.parts or path.parts[0] != 'package' or '..' in path.parts:
                raise ValueError('invalid packed member')
            relative = Path(*path.parts[1:])
            expected = hashlib.sha256(archive.extractfile(member).read()).hexdigest()
            packed_hashes[relative] = expected
            if digest(installed / relative) != expected:
                raise ValueError('installed artifact mismatch')
    shutil.copytree(PACKAGE / 'test', consumer / 'test')
    for runtime in RUNTIMES:
        path = consumer / f'test/{runtime}/{runtime}.test.mjs'
        source = path.read_text()
        entry = 'deno' if runtime == 'deno' else 'node'
        for old, new in [(f'../../dist/{entry}.js', f'@writerslogic/warm-wasm/{entry}'),
                         ('../../dist/file.js', '@writerslogic/warm-wasm/storage/file')]:
            if source.count(old) != 1:
                raise ValueError('unexpected test import')
            source = source.replace(old, new)
        path.write_text(source)
    deno_consumer = ROOT / 'runtime-consumers/deno'
    deno_consumer.mkdir(parents=True)
    write(deno_consumer / 'package.json', {'name': 'wasm-deno-consumer', 'private': True, 'type': 'module'})
    npm = NODE.parent.parent / 'lib/node_modules/npm/bin/npm-cli.js'
    run('deno-install', [NODE, npm, 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund',
                         archives[0], '@types/node@22.20.5'], deno_consumer)
    shutil.copytree(consumer / 'test', deno_consumer / 'test')
    for relative, expected in packed_hashes.items():
        if digest(deno_consumer / 'node_modules/@writerslogic/warm-wasm' / relative) != expected:
            raise ValueError('Deno installed artifact mismatch')
    node = run('installed-node-tests', [NODE, '--test', 'test/node/node.test.mjs'], consumer)
    deno = run('installed-deno-tests', ['/opt/homebrew/bin/deno', 'test', '--allow-read', '--allow-write', '--allow-env', 'test/deno/'], deno_consumer)
    bun = run('installed-bun-tests', [ROOT / 'tools/bin/bun', 'test', 'test/bun/'], consumer)
    if not all(re.search(pattern, node) for pattern in (r'# tests 18\b', r'# pass 18\b', r'# fail 0\b', r'# skipped 0\b')):
        raise ValueError('unexpected installed Node test totals')
    if not re.search(r'18 passed.*0 failed', deno) or 'ignored' in deno.lower():
        raise ValueError('unexpected installed Deno test totals')
    if not re.search(r'18 pass', bun) or not re.search(r'0 fail', bun):
        raise ValueError('unexpected installed Bun test totals')
    shutil.move(PACKAGE / 'dist', ROOT / 'built-dist')
    (PACKAGE / 'dist').symlink_to(installed / 'dist', target_is_directory=True)
    browser = run('browser', [NODE, 'test/browser/run.mjs'], timeout=1800)
    if browser.count('page tests: 7 passed') != 2 or browser.count('persistence + offline): passed') != 4 or 'SKIP' in browser:
        raise ValueError('incomplete browser lifecycle checks')
    bundler = run('bundler', [NODE, 'test/bundler/run.mjs'], timeout=1800)
    if bundler.count('entry in Chrome: ok') != 2:
        raise ValueError('incomplete bundler checks')
    rust = run('rust-browser-tests', [NODE, 'scripts/wasm-test.mjs'], timeout=7200)
    if len(re.findall(r'4 passed; 0 failed; 0 ignored', rust)) != 2:
        raise ValueError('incomplete Rust browser tests')
    cases = recovery(consumer)
    verify(freeze)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(ROOT / 'freeze.json'), 'archiveSha256': digest(archives[0]),
            'archive': str(archives[0].relative_to(ROOT)), 'installedFiles': len(members),
            'runtimeTests': {name: 18 for name in RUNTIMES}, 'browserTests': {'chrome': 7, 'firefox': 7},
            'offlinePersistenceDemos': 4, 'webpackEntries': 2, 'rustBrowserTests': 8,
            'recoveryWriters': len(cases), 'recoveryReaders': len(cases) * 3,
            'logs': {path.name: digest(path) for path in sorted(ROOT.glob('*.log'))}}


if __name__ == '__main__':
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(ROOT / 'finished.json', result)
