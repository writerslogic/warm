"""Transfer and launch the frozen WARM recalibration runner."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-warm-recalibration-20261103-v5')
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103'
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV['LIMA_HOME'] = '/Volumes/A/.hms-lima'


def main():
    prerequisite = json.loads(Path('/Volumes/A/.hms-bench-cache/warm-wasm-qualification-20261103-v4/finished.json').read_text())
    if prerequisite.get('complete') is not True:
        raise ValueError('local package qualification has not passed')
    script = ROOT / 'linux-run.py'
    expected = json.loads((ROOT / 'source.json').read_text())['scriptSha256']
    if hashlib.sha256(script.read_bytes()).hexdigest() != expected:
        raise ValueError('frozen runner changed')
    subprocess.run(['limactl', 'copy', str(script), 'warm-arm64:' + GUEST + '/warm-recalibration-v5.py'],
                   env=ENV, check=True, timeout=60)
    command = '''from pathlib import Path
import hashlib, subprocess
root = Path('/home/davidcondrey.guest/warm-milestones-20261103')
script = root / 'warm-recalibration-v5.py'
if hashlib.sha256(script.read_bytes()).hexdigest() != EXPECTED:
    raise ValueError('transferred runner changed')
if (root / 'evidence/warm-recalibration-v5').exists():
    raise FileExistsError('recalibration output already exists')
with (root / 'evidence/warm-recalibration-v5-launcher.log').open('xb') as log:
    process = subprocess.Popen([str(root / 'tools/uv/bin/uv'), 'run', '--no-project',
        '--python', '3.12', str(script)], stdout=log, stderr=subprocess.STDOUT,
        start_new_session=True)
(root / 'evidence/warm-recalibration-v5-launcher.pid').write_text(str(process.pid) + chr(10))
print(process.pid)
'''.replace('EXPECTED', repr(expected))
    output = subprocess.check_output(['limactl', 'shell', 'warm-arm64', 'python3', '-c', command],
                                     env=ENV, text=True, timeout=60)
    return {'complete': True, 'launchedAt': datetime.now(timezone.utc).isoformat(),
            'guestPid': int(output.strip()), 'scriptSha256': expected,
            'guestOutput': GUEST + '/evidence/warm-recalibration-v5'}


if __name__ == '__main__':
    result = {'complete': False}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        with (ROOT / 'coordinator-finished.json').open('x') as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write('\n')
