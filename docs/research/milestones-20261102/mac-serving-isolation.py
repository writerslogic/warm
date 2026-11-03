"""Retain physical-host observations; no automatic isolation-pass assertion."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-serving-qualification-20261103-v1')
OUTPUT = Path('/Users/davidcondrey/.cache/warm-serving-isolation-20261103-v1')
COMMANDS = {
    'processes': ['ps', '-axo', 'pid=,ppid=,time=,rss=,comm='],
    'virtualMemory': ['vm_stat'],
    'memoryPressure': ['memory_pressure', '-Q'],
    'swapUsage': ['sysctl', 'vm.swapusage'],
    'thermal': ['pmset', '-g', 'therm'],
    'power': ['pmset', '-g', 'batt'],
}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def main():
    OUTPUT.mkdir()
    expected = json.loads((ROOT / 'freeze.json').read_text())['artifacts']['mac-serving-isolation.py']
    if digest(Path(__file__)) != expected:
        raise ValueError('host observer changed after freeze')
    start = time.monotonic()
    count = 0
    failed_commands = 0
    inhibit = subprocess.Popen(['caffeinate', '-is', '-w', str(os.getpid())],
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        with (OUTPUT / 'samples.ndjson').open('x') as stream:
            while True:
                sample_start = time.monotonic()
                sample = {'observedAt': datetime.now(timezone.utc).isoformat(),
                          'elapsed': sample_start - start, 'monotonic': sample_start, 'commands': {}}
                for name, arguments in COMMANDS.items():
                    completed = subprocess.run(arguments, capture_output=True, text=True, timeout=10)
                    if len(completed.stdout) + len(completed.stderr) > 2**21:
                        raise ValueError('host observation exceeded its output bound')
                    failed_commands += int(completed.returncode != 0)
                    sample['commands'][name] = {'exitCode': completed.returncode,
                                               'stdout': completed.stdout, 'stderr': completed.stderr}
                sample['seconds'] = time.monotonic() - sample_start
                stream.write(json.dumps(sample, sort_keys=True, allow_nan=False) + '\n')
                stream.flush()
                if count % 12 == 0:
                    os.fsync(stream.fileno())
                count += 1
                if count == 1:
                    write(ROOT / 'host-observation-started.json', {
                        'started': True, 'pid': os.getpid(), 'startedAt': sample['observedAt'],
                        'freezeSha256': digest(ROOT / 'freeze.json'), 'rawDirectory': str(OUTPUT),
                        'firstSampleCommandFailures': failed_commands, 'isolationPassed': False})
                if (ROOT / 'stop-host-observation').exists():
                    break
                if time.monotonic() - start >= 86400:
                    raise TimeoutError('host observation exceeded 24 hours')
                time.sleep(max(0.0, 5.0 - (time.monotonic() - sample_start)))
            os.fsync(stream.fileno())
    finally:
        if inhibit.poll() is None:
            inhibit.terminate()
        inhibit.wait(timeout=30)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'samples': count, 'commandFailures': failed_commands,
            'elapsed': time.monotonic() - start, 'samplesSha256': digest(OUTPUT / 'samples.ndjson'),
            'isolationPassed': False, 'scope': 'Raw observations only; attribution and contamination review required.'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        if OUTPUT.is_dir():
            write(OUTPUT / 'finished.json', result)
