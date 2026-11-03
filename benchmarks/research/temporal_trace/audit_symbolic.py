# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Compare the temporal certificate with ordinary symbolic query compilation."""

import argparse
import json
from pathlib import Path
import shutil
import signal
from statistics import median
import time

from probe import Record, compile_certificate, packet
from run import digest, write
from symbolic_control import symbolic_certificate


def expression(value):
    return tuple(expression(item) if isinstance(item, list) else item for item in value)


def main(original, output):
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).parent
    files = sorted([*source.glob('*.py'), source / 'PLAN.json'])
    hashes = {path.name: digest(path) for path in files}
    prior = json.loads((original / 'finished.json').read_text())
    if prior.get('correctnessPassed') is not True:
        raise ValueError('original correctness cohort did not pass')
    original_source = json.loads((original / 'freeze.json').read_text())['sourceSha256']
    if any(hashes[name] != expected for name, expected in original_source.items()):
        raise ValueError('the original experiment implementation changed')
    freeze = {'schema': 'warm.temporal-symbolic-control/1', 'files': hashes,
              'originalReceiptSha256': digest(original / 'finished.json'), 'original': str(original),
              'methods': ['trace', 'symbolic'], 'repetitions': 3, 'deadlineSeconds': 30,
              'symbolicBounds': {'nodes': 250000, 'operationCache': 1000000, 'expressionStates': 1000000},
              'variableOrder': 'ascending source ID; no reordering or tuning',
              'acceptance': 'Both compilers must produce byte-identical packets to the already exhaustively verified trace union in every exposed cohort case. Existing byte savings against per-slot controls cannot establish superiority if symbolic compilation produces the same packets. Timings are shared-host development measurements only.',
              'benchmarkClaim': False}
    write(output / 'freeze.json', freeze)
    (output / 'source').mkdir()
    for path in files:
        shutil.copyfile(path, output / 'source' / path.name)

    def expired(signum, frame):
        raise TimeoutError('compiler exceeded thirty seconds')

    if signal.getitimer(signal.ITIMER_REAL)[0]:
        raise RuntimeError('cannot replace an active deadline')
    previous = signal.signal(signal.SIGALRM, expired)
    cases = []
    try:
        for result_path in sorted(original.glob('*/result.json')):
            folder, name = result_path.parent, result_path.parent.name
            for filename in ('input.json', 'trace.json', 'result.json'):
                if digest(folder / filename) != prior['filesSha256'][name + '/' + filename]:
                    raise ValueError('original evidence changed: ' + name + '/' + filename)
            data = json.loads((folder / 'input.json').read_text())
            rows = [Record(**{**item, 'sources': tuple(item['sources'])}) for item in data['records']]
            programs = list(map(expression, data['programs']))
            snapshots = list(map(tuple, data['snapshots']))
            expected = (folder / 'trace.json').read_bytes()
            trials = []
            for repetition in range(3):
                for method in ('trace', 'symbolic') if repetition % 2 == 0 else ('symbolic', 'trace'):
                    started = time.perf_counter()
                    signal.setitimer(signal.ITIMER_REAL, 30)
                    try:
                        compiler = compile_certificate if method == 'trace' else symbolic_certificate
                        selected, details = compiler(rows, programs, snapshots, data['budget'])
                    finally:
                        signal.setitimer(signal.ITIMER_REAL, 0)
                    elapsed = time.perf_counter() - started
                    serialized = packet(selected, programs, snapshots, data['budget'])
                    if serialized != expected:
                        raise AssertionError('compiler changed the exhaustively verified packet')
                    trials.append({'method': method, 'repetition': repetition, 'seconds': elapsed,
                                   'details': details, 'packetBytes': len(serialized), 'byteIdenticalToOriginal': True})
            medians = {method: median(item['seconds'] for item in trials if item['method'] == method)
                       for method in ('trace', 'symbolic')}
            result = {'case': name, 'medians': medians, 'symbolicToTraceConstructionRatio': medians['symbolic'] / medians['trace'],
                      'packetBytes': len(expected), 'bothPacketsIdentical': True, 'trials': trials,
                      'inheritedSnapshotWithdrawalChecks': json.loads(result_path.read_text())['states']}
            write(output / (name + '.json'), result)
            cases.append({key: value for key, value in result.items() if key != 'trials'})
            print(json.dumps(cases[-1]), flush=True)
        if len(cases) != 12:
            raise AssertionError('original cohort does not contain twelve cases')
        if hashes != {path.name: digest(path) for path in files}:
            raise RuntimeError('audit source changed during execution')
        write(output / 'finished.json', {'complete': True, 'cases': cases, 'sizeSuperiorityOverSymbolicControl': False,
                                        'benchmarkClaim': False, 'freezeSha256': digest(output / 'freeze.json'),
                                        'filesSha256': {str(path.relative_to(output)): digest(path)
                                                        for path in sorted(output.rglob('*')) if path.is_file()}})
    except Exception as error:
        write(output / 'failed.json', {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}})
        raise
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--original', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    main(args.original, args.output)
