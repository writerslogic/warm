# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Audit the observed development speedup against a stronger exact basis oracle."""

import argparse
import json
from pathlib import Path
import shutil
from statistics import median
import time

from basis_control import proof_union
from refine import compile_method
from run import deadline, digest, write
from source_lift.probe import Record, packed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--original', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.original.is_absolute() or not args.output.is_absolute():
        parser.error('absolute artifact paths required')
    args.output.mkdir()
    here, parent = Path(__file__).parent, Path(__file__).parent.parent
    sources = [*here.glob('*.py'), here / 'PLAN.json', parent / 'source_lift/probe.py']
    hashes = {str(path.relative_to(parent)): digest(path) for path in sorted(sources)}
    protocol = {'original': str(args.original), 'files': hashes,
                'scope': 'Stronger-control audit on already exposed development inputs; no new held-out or novelty claim.',
                'methods': ['lift-refine', 'proof-union', 'proof-union-refine'], 'repetitions': 3,
                'acceptance': 'New controls must produce byte-identical packets to the already exhaustively verified union and union-refine controls. Repeat candidate and new controls in rotated order. A faster matched-size new control falsifies the original 2x candidate advantage; this audit cannot by itself certify superiority over all algorithms.',
                'deadlineSecondsPerCompiler': 30, 'benchmarkClaim': False}
    write(args.output / 'freeze.json', protocol)
    for name in hashes:
        destination = args.output / 'source' / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(parent / name, destination)
    try:
        wait_until = time.monotonic() + 3600
        while not (args.original / 'finished.json').exists():
            if (args.original / 'failed.json').exists():
                raise RuntimeError('original experiment failed before completing its cohort')
            if time.monotonic() > wait_until:
                raise TimeoutError('original experiment did not finish within one hour')
            time.sleep(30)
        original = json.loads((args.original / 'finished.json').read_text())
        if original.get('complete') is not True:
            raise ValueError('original experiment is incomplete')
        spec = json.loads((args.original / 'freeze.json').read_text())['plan']['cohort']
        results = []
        for case in original['cases']:
            name = f"{case['family']}-{case['sources']}-{case['seed']}"
            folder = args.original / name
            for filename in ('input.json', 'union.bin', 'union-refine.bin', 'lift-refine.bin'):
                if digest(folder / filename) != original['files'][name + '/' + filename]:
                    raise ValueError('original evidence digest mismatch')
            rows = [Record(row['ordinal'], tuple(row['sources']), tuple(row['values']))
                    for row in json.loads((folder / 'input.json').read_text())]
            prime, width, budget = spec['field'], spec['width'], spec['withdrawalBudget']
            fanin = 2 if case['family'] == 'source-pairs' else 1
            expected = {method: (folder / (mapped + '.bin')).read_bytes()
                        for method, mapped in [('lift-refine', 'lift-refine'), ('proof-union', 'union'),
                                               ('proof-union-refine', 'union-refine')]}
            measurements = {method: [] for method in protocol['methods']}
            trials = []
            for repetition in range(protocol['repetitions']):
                methods = protocol['methods'][repetition:] + protocol['methods'][:repetition]
                for method in methods:
                    start = time.perf_counter()
                    with deadline(30):
                        if method == 'lift-refine':
                            selected, details = compile_method(method, rows, width, prime, budget, fanin)
                        else:
                            selected, details = proof_union(rows, width, prime, budget, fanin,
                                                            minimize=method == 'proof-union-refine')
                    elapsed = time.perf_counter() - start
                    packet = packed(selected, width, prime, budget, fanin)
                    if packet != expected[method]:
                        raise AssertionError('stronger control or repeated candidate changed its certified packet')
                    measurements[method].append(elapsed)
                    trials.append({'method': method, 'repetition': repetition, 'seconds': elapsed,
                                   'packetBytes': len(packet), 'identicalCertifiedPacket': True, 'details': details})
            medians = {method: median(values) for method, values in measurements.items()}
            target_size = len(expected['lift-refine'])
            matched = [method for method in protocol['methods'][1:] if len(expected[method]) <= 1.1 * target_size]
            fastest = min(matched, key=lambda method: medians[method]) if matched else None
            ratio = medians[fastest] / medians['lift-refine'] if fastest else None
            result = {'case': name, 'medians': medians, 'matchedNewControls': matched,
                      'fastestNewControl': fastest, 'candidateConstructionRatio': ratio,
                      'originalAdvantageSurvivesNewControls': ratio is not None and ratio >= 2,
                      'trials': trials, 'verifiedStatesInheritedFromByteIdenticalPackets': case['verifiedStates']}
            write(args.output / (name + '.json'), result)
            results.append({key: value for key, value in result.items() if key != 'trials'})
            print(json.dumps(results[-1]), flush=True)
        if any(digest(parent / name) != expected for name, expected in hashes.items()):
            raise RuntimeError('control audit source changed during the run')
        write(args.output / 'finished.json', {'complete': True, 'originalReceiptSha256': digest(args.original / 'finished.json'),
              'cases': results, 'originalAdvantageSurvivesNewControls': all(row['originalAdvantageSurvivesNewControls'] for row in results),
              'files': {str(path.relative_to(args.output)): digest(path) for path in sorted(args.output.rglob('*')) if path.is_file()},
              'benchmarkClaim': False})
    except Exception as error:
        write(args.output / 'failed.json', {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}})
        raise


if __name__ == '__main__':
    main()
