"""Compare the frozen identity with shared per-probability decision diagrams."""

import argparse
from datetime import datetime, timezone
import gc
import json
from pathlib import Path
import shutil
import statistics
import time

from probe import Profile, equivalent
from run import digest, write
from symbolic_control import BudgetExceeded, Diagrams


def audit(root, plan):
    started = time.monotonic()
    results = []
    for sources in plan['sourceCounts']:
        for weighting in plan['weights']:
            records = tuple((source % plan['outcomes'], 1 << source,
                             1 if weighting == 'uniform' else 1 << source) for source in range(sources))
            left = Profile(sources, plan['outcomes'], records)
            right = Profile(sources, plan['outcomes'],
                            tuple((outcome, mask | (1 << other), count)
                                  for outcome, mask, count in records for other in range(sources)))
            candidate, controls, capped = [], {'ascending': [], 'descending': []}, set()
            for repetition in range(plan['repetitions']):
                methods = ['polynomial', 'ascending', 'descending']
                methods = methods[repetition:] + methods[:repetition]
                for method in methods:
                    if method in capped:
                        continue
                    if time.monotonic() - started > plan['limits']['seconds']:
                        raise TimeoutError('frozen symbolic comparison time limit exceeded')
                    gc.collect()
                    if method == 'polynomial':
                        start = time.perf_counter()
                        result = equivalent(left, right)
                        elapsed = time.perf_counter() - start
                        if not result['equivalent']:
                            raise AssertionError('polynomial identity failed its constructed case')
                        candidate.append({'seconds': elapsed, 'result': result})
                    else:
                        diagrams = Diagrams(method == 'descending',
                                            plan['limits']['diagramNodesAndTerminals'],
                                            plan['limits']['diagramStates'])
                        start = time.perf_counter()
                        try:
                            first, second = diagrams.profile(left), diagrams.profile(right)
                            elapsed = time.perf_counter() - start
                            if first != second:
                                raise AssertionError('decision diagrams disagree with polynomial identity')
                            for mask in (0, 1, (1 << sources) - 1, (1 << sources) - 2):
                                if diagrams.read(first, mask) != left.read(mask) or diagrams.read(second, mask) != right.read(mask):
                                    raise AssertionError('decision diagram readout differs from observations')
                            controls[method].append({'complete': True, 'seconds': elapsed, **diagrams.stats()})
                        except BudgetExceeded as error:
                            controls[method].append({'complete': False, 'secondsUntilLimit': time.perf_counter() - start,
                                                     'reason': str(error), **diagrams.stats()})
                            capped.add(method)
                        finally:
                            del diagrams
            both_complete = all(len(rows) == plan['repetitions'] and all(row['complete'] for row in rows)
                                for rows in controls.values())
            ratio = None
            if both_complete:
                best = min(statistics.median(row['seconds'] for row in rows) for rows in controls.values())
                ratio = statistics.median(row['seconds'] for row in candidate) / best
            result = {'sources': sources, 'weights': weighting, 'candidate': candidate,
                      'controls': controls, 'bothOrdersComplete': both_complete,
                      'candidateOverFastestControl': ratio}
            write(root / f'case-{sources}-{weighting}.json', result)
            results.append(result)
            print(json.dumps({'sources': sources, 'weights': weighting, 'bothOrdersComplete': both_complete,
                              'candidateOverFastestControl': ratio}), flush=True)
    return {'complete': True, 'cases': results, 'seconds': time.monotonic() - started,
            'performanceQualified': False, 'noveltyEstablished': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parent
    frozen = args.output / 'source'
    frozen.mkdir()
    paths = [*sorted(source.glob('*.py')), source / 'PLAN.json', source / 'SYMBOLIC-PLAN.json']
    for path in paths:
        shutil.copy2(path, frozen / path.name)
    hashes = {path.name: digest(path) for path in paths}
    write(args.output / 'freeze.json', {'schema': 'warm.source-prediction-symbolic-freeze/1', 'files': hashes,
                                      'startedAt': datetime.now(timezone.utc).isoformat()})
    result = {'complete': False, 'error': 'Interrupted before completion'}
    try:
        result = audit(args.output, json.loads((frozen / 'SYMBOLIC-PLAN.json').read_text()))
        if any(digest(source / name) != value or digest(frozen / name) != value for name, value in hashes.items()):
            raise RuntimeError('research source changed during comparison')
        result['rawSha256'] = {path.name: digest(path) for path in sorted(args.output.glob('case-*.json'))}
        result['freezeSha256'] = digest(args.output / 'freeze.json')
        result['finishedAt'] = datetime.now(timezone.utc).isoformat()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(args.output / 'finished.json', result)
