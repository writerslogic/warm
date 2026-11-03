"""Start arm64 WARM recalibration after local package checks finish."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path('/Volumes/A/.hms-bench-cache/warm-warm-recalibration-20261103-v3')
WAIT = Path('/Volumes/A/.hms-bench-cache/warm-wasm-qualification-20261103-v4/finished.json')
GUEST = '/home/davidcondrey.guest/warm-milestones-20261103'
ENV = {name: value for name, value in os.environ.items() if name in ('HOME', 'PATH', 'LANG', 'LC_ALL', 'TMPDIR')}
ENV['LIMA_HOME'] = '/Volumes/A/.hms-lima'


def main():
    deadline = time.monotonic() + 28800
    while not WAIT.exists():
        if time.monotonic() > deadline:
            raise TimeoutError('local package checks did not finish within eight hours')
        time.sleep(30)
    result = json.loads(WAIT.read_text())
    if result.get('complete') is not True:
        raise ValueError('local package qualification failed; recalibration not started')
    script = ROOT / 'linux-run.py'
    expected = json.loads((ROOT / 'source.json').read_text())['scriptSha256']
    if hashlib.sha256(script.read_bytes()).hexdigest() != expected:
        raise ValueError('queued runner changed')
    subprocess.run(['limactl', 'copy', str(script), 'warm-arm64:' + GUEST + '/warm-recalibration-v3.py'], env=ENV, check=True)
    command = "from pathlib import Path; import hashlib,subprocess; root=Path(" + repr(GUEST) + "); script=root/'warm-recalibration-v3.py'; assert hashlib.sha256(script.read_bytes()).hexdigest()==" + repr(expected) + "; log=(root/'evidence/warm-recalibration-v3-launcher.log').open('xb'); process=subprocess.Popen([str(root/'tools/uv/bin/uv'),'run','--no-project','--python','3.12',str(script)],stdout=log,stderr=subprocess.STDOUT,start_new_session=True); (root/'evidence/warm-recalibration-v3-launcher.pid').write_text(str(process.pid)+'\\n'); print(process.pid)"
    output = subprocess.check_output(['limactl', 'shell', 'warm-arm64', 'python3', '-c', command], env=ENV, text=True)
    return {'complete': True, 'launchedAt': datetime.now(timezone.utc).isoformat(), 'guestPid': int(output.strip()),
            'guestOutput': GUEST + '/evidence/warm-recalibration-v3', 'scriptSha256': expected,
            'prerequisiteFreezeSha256': result['freezeSha256']}


if __name__ == '__main__':
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        with (ROOT / 'coordinator-finished.json').open('x') as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write('\n')
