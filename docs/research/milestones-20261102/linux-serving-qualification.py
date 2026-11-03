"""Run the frozen 18-job order, retaining completed quality failures unchanged."""
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess

ROOT = Path('/home/davidcondrey.guest/warm-milestones-20261103/serving-qualification-v1')
spec = importlib.util.spec_from_file_location('serving_seal', ROOT / 'linux-serving-seal.py')
seal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(seal)
INPUTS = ROOT / 'inputs'
OUTPUT = ROOT / 'measurements'


def main():
    seal.verify()
    sealed = json.loads((ROOT / 'sealed.json').read_text())
    selected = ROOT / 'selection/configuration-seal.json'
    if not sealed.get('complete') or seal.digest(selected) != sealed['configurationSealSha256']:
        raise ValueError('configuration seal is incomplete or changed')
    admission = json.loads((ROOT / 'host-observation-started.json').read_text())
    if (admission.get('started') is not True or admission.get('firstSampleCommandFailures') != 0
            or admission['freezeSha256'] != seal.digest(ROOT / 'freeze.json')):
        raise ValueError('physical-host observation must start before qualification')
    jobs = []
    for ordinal in range(18):
        arguments = ['taskset', '-c', '0-3', *seal.PYTHON,
            'benchmarks/public/matched_server/qualify.py', '--seal', str(selected),
            '--inputs', str(INPUTS), '--output', str(OUTPUT), '--job', str(ordinal)]
        with (ROOT / f'job-{ordinal:02d}.log').open('xb') as log:
            process = subprocess.Popen(arguments, cwd=seal.SOURCE, env=seal.ENV,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=14400)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=30)
        result = json.loads((OUTPUT / f'job-{ordinal:02d}/finished.json').read_text())
        sources = json.loads((OUTPUT / f'job-{ordinal:02d}-source-verification.json').read_text())
        if not result.get('complete') or sources.get('pass') is not True:
            raise RuntimeError(f'job {ordinal} did not complete with unchanged source; evidence retained')
        expected_exit = 0 if result['qualityPassed'] else 1
        if code != expected_exit:
            raise RuntimeError(f'job {ordinal} returned an unexplained exit status')
        jobs.append({'job': ordinal, 'exitCode': code, 'qualityPassed': result['qualityPassed'],
                     'finishedSha256': seal.digest(OUTPUT / f'job-{ordinal:02d}/finished.json')})
        seal.verify()
        seal.write(ROOT / f'job-{ordinal:02d}-accepted.json', jobs[-1])
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(), 'jobs': jobs,
            'allQualityPassed': all(row['qualityPassed'] for row in jobs), 'claimQualified': False,
            'freezeSha256': seal.digest(ROOT / 'freeze.json'),
            'pending': ['Raw serving-job audit', 'Physical-host isolation review',
                        'Actual engine settings and resource review', 'Second physical host', 'Comparison gates']}


if __name__ == '__main__':
    result = {'complete': False, 'error': {'type': 'Interrupted', 'message': 'No terminal result'}}
    try:
        result = main()
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        seal.write(ROOT / 'finished.json', result)
