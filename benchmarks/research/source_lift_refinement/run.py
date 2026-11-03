# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Frozen development comparison of exact source-certificate compilers."""

import argparse
from contextlib import contextmanager
import hashlib
from itertools import product
import json
from pathlib import Path
import random
import shutil
import signal
from statistics import median
import time
import tracemalloc

from refine import compile_method, verify_variants
from source_lift.probe import Record, packed


METHODS = ('lift-refine', 'provenance', 'union', 'union-refine', 'direct-refine')


@contextmanager
def deadline(seconds):
    def expired(signum, frame):
        raise TimeoutError('compiler exceeded the frozen thirty-second deadline')

    if signal.getitimer(signal.ITIMER_REAL)[0] != 0:
        raise RuntimeError('cannot replace an existing timer')
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def generate(family, count, seed, spec):
    rng = random.Random(seed)
    prime, width = spec['field'], spec['width']
    hidden = tuple(rng.randrange(prime) for _ in range(width - 1))
    rows = []
    for source in range(1, count + 1):
        for _ in range(spec['rowsPerSource']):
            sources = (source,)
            if family == 'source-pairs':
                other = rng.randrange(1, count)
                other += int(other >= source)
                sources = tuple(sorted((source, other)))
            coefficients = tuple(rng.randrange(prime) for _ in range(width - 1))
            value = sum(a * b for a, b in zip(coefficients, hidden)) % prime
            rows.append(Record(len(rows), sources, (*coefficients, value)))
    return rows


def trial(method, rows, width, prime, budget, fanin, trace=False):
    if trace:
        tracemalloc.start()
    started, cpu = time.perf_counter(), time.process_time()
    try:
        with deadline(30):
            selected, details = compile_method(method, rows, width, prime, budget, fanin)
        receipt = {'complete': True, 'seconds': time.perf_counter() - started,
                   'cpuSeconds': time.process_time() - cpu, 'tracing': trace,
                   'selected': [row.ordinal for row in selected], 'details': details}
        if trace:
            receipt['peakTracedBytes'] = tracemalloc.get_traced_memory()[1]
        return (selected, details), receipt
    except Exception as error:
        return None, {'complete': False, 'seconds': time.perf_counter() - started,
                      'cpuSeconds': time.process_time() - cpu, 'tracing': trace,
                      'error': {'type': type(error).__name__, 'message': str(error)}}
    finally:
        if trace:
            tracemalloc.stop()


def run_case(output, family, count, seed, spec):
    output.mkdir()
    rows = generate(family, count, seed, spec)
    width, prime, budget = spec['width'], spec['field'], spec['withdrawalBudget']
    fanin = 2 if family == 'source-pairs' else 1
    write(output / 'input.json', [{'ordinal': row.ordinal, 'sources': row.sources, 'values': row.values} for row in rows])
    variants, timings, failed, packets = {}, {name: [] for name in METHODS}, set(), {}
    for repetition in range(spec['repetitions']):
        order = METHODS[repetition:] + METHODS[:repetition]
        for method in order:
            if method in failed:
                continue
            result, receipt = trial(method, rows, width, prime, budget, fanin)
            write(output / f'{method}-{repetition}.json', receipt)
            if result is None:
                failed.add(method)
                continue
            packet = packed(result[0], width, prime, budget, fanin)
            if method in packets and packet != packets[method]:
                raise AssertionError('compiler changed retained evidence across repetitions')
            variants[method], packets[method] = result, packet
            timings[method].append(receipt['seconds'])
    for method in METHODS:
        if method in failed:
            continue
        result, receipt = trial(method, rows, width, prime, budget, fanin, trace=True)
        write(output / f'{method}-allocation.json', receipt)
        if result is None:
            failed.add(method)
        elif packed(result[0], width, prime, budget, fanin) != packets[method]:
            raise AssertionError('allocation instrumentation changed retained evidence')
    states, fallbacks = 0, 0
    with (output / 'verification.ndjson').open('x') as stream:
        for state in verify_variants(rows, variants, width, prime, budget):
            stream.write(json.dumps(state) + '\n')
            states += 1
            fallbacks += state['fullReferenceRequired']
    for method, packet in packets.items():
        (output / f'{method}.bin').write_bytes(packet)
    sizes = {method: len(packet) for method, packet in packets.items()}
    medians = {method: median(values) for method, values in timings.items() if len(values) == spec['repetitions']}
    eligible, fastest, ratio, passed = [], None, None, False
    if not failed and set(sizes) == set(METHODS):
        smallest_control = min(size for name, size in sizes.items() if name != 'lift-refine')
        eligible = [name for name in METHODS if name != 'lift-refine' and sizes[name] <= 1.1 * smallest_control]
        fastest = min(eligible, key=lambda name: medians[name])
        ratio = medians[fastest] / medians['lift-refine']
        passed = (sizes['lift-refine'] <= .5 * sizes['provenance']
                  and sizes['lift-refine'] <= 1.1 * smallest_control and ratio >= 2.)
    result = {'family': family, 'sources': count, 'seed': seed, 'inputRows': len(rows),
              'correctnessPassedForCompletedCompilers': True, 'verifiedStates': states,
              'fullReferenceFallbacks': fallbacks, 'failedCompilers': sorted(failed),
              'packedBytes': sizes, 'constructionMedianSeconds': medians,
              'eligibleControls': eligible, 'fastestEligibleControl': fastest,
              'constructionRatio': ratio, 'screenPassed': passed, 'benchmarkClaim': False}
    write(output / 'finished.json', result)
    print(json.dumps(result), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.output.is_absolute():
        parser.error('output must be absolute')
    args.output.mkdir()
    here, parent = Path(__file__).parent, Path(__file__).parent.parent
    plan = json.loads((here / 'PLAN.json').read_text())
    files = [*here.glob('*.py'), here / 'PLAN.json', parent / 'source_lift/probe.py', parent / 'source_lift/PLAN.json']
    sources = {str(path.relative_to(parent)): digest(path) for path in sorted(files)}
    write(args.output / 'freeze.json', {'plan': plan, 'files': sources})
    for relative in sources:
        destination = args.output / 'source' / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(parent / relative, destination)
    results, started = [], time.perf_counter()
    try:
        spec = plan['cohort']
        for family, count, seed in product(spec['families'], spec['sourceCounts'], spec['seeds']):
            results.append(run_case(args.output / f'{family}-{count}-{seed}', family, count, seed, spec))
        if any(digest(parent / relative) != expected for relative, expected in sources.items()):
            raise RuntimeError('frozen source changed during the experiment')
        hashes = {str(path.relative_to(args.output)): digest(path) for path in sorted(args.output.rglob('*')) if path.is_file()}
        write(args.output / 'finished.json', {'complete': True, 'screenPassed': all(row['screenPassed'] for row in results),
              'cases': results, 'files': hashes, 'elapsedSeconds': time.perf_counter() - started, 'benchmarkClaim': False})
    except Exception as error:
        write(args.output / 'failed.json', {'complete': False, 'completedCases': len(results),
              'error': {'type': type(error).__name__, 'message': str(error)}})
        raise


if __name__ == '__main__':
    main()
