"""Stream guest load and process observations to the host's internal disk."""
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import time

ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103/serving-qualification-v1')
FILES = ('stat', 'meminfo', 'vmstat', 'loadavg', 'pressure/cpu', 'pressure/io', 'pressure/memory')


def main():
    started = time.monotonic()
    count = 0
    while True:
        before = time.monotonic()
        processes = subprocess.check_output(['ps', '-eo', 'pid=,ppid=,time=,rss=,comm='], text=True, timeout=10)
        sample = {'type': 'sample', 'observedAt': datetime.now(timezone.utc).isoformat(),
                  'monotonic': before, 'elapsed': before - started, 'processes': processes,
                  'proc': {name: (Path('/proc') / name).read_text() for name in FILES}}
        if len(processes) > 2**21 or any(len(value) > 2**21 for value in sample['proc'].values()):
            raise ValueError('guest observation exceeded its bound')
        sample['seconds'] = time.monotonic() - before
        print(json.dumps(sample, sort_keys=True, allow_nan=False), flush=True)
        count += 1
        if (ROOT / 'stop-guest-observation').exists():
            return {'type': 'terminal', 'complete': True, 'samples': count,
                    'elapsed': time.monotonic() - started, 'isolationPassed': False}
        if time.monotonic() - started >= 86400:
            raise TimeoutError('guest observation exceeded 24 hours')
        time.sleep(max(0.0, 5.0 - (time.monotonic() - before)))


if __name__ == '__main__':
    result = {'type': 'terminal', 'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        print(json.dumps(result, sort_keys=True, allow_nan=False), flush=True)
