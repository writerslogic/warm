"""Freeze and test the predictive-distribution identity on local exact data."""

import argparse
from dataclasses import asdict
from datetime import datetime, timezone
import hashlib
from itertools import product
import json
from pathlib import Path
import random
import shutil
import time

from probe import Profile, equivalent, multiplied_profile


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write('\n')


def line(stream, value):
    stream.write(json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n')


def encode(value):
    return None if value is None else [[item.numerator, item.denominator] for item in value]


def run(root, plan):
    start = time.monotonic()
    summary = {'pairs': 0, 'withdrawalChecks': 0, 'equivalentPairs': 0,
               'currentOnlyWrongPairs': 0, 'noSupportWrongPairs': 0, 'structuralMissedPairs': 0}
    cohorts = []
    with (root / 'profiles.ndjson').open('x') as profiles_raw, (root / 'pairs.ndjson').open('x') as pairs_raw:
        def check(label, left, right, masks, left_reads=None, right_reads=None):
            if time.monotonic() - start > plan['limits']['seconds']:
                raise TimeoutError('frozen source-prediction time limit exceeded')
            left_reads = [left.read(mask) for mask in masks] if left_reads is None else left_reads
            right_reads = [right.read(mask) for mask in masks] if right_reads is None else right_reads
            oracle = all(a == b for a, b in zip(left_reads, right_reads, strict=True))
            result = equivalent(left, right)
            if result['equivalent'] != oracle:
                raise AssertionError('candidate disagrees with exact readouts: ' + label)
            if not result['equivalent']:
                mask = result['withdrawn']
                if left.read(mask) == right.read(mask):
                    raise AssertionError('invalid concrete withdrawal witness: ' + label)
                if result['reason'] == 'distribution':
                    width = max((mask.bit_count() for profile in (left, right)
                                 for _, mask, _ in profile.records), default=0)
                    if result['withdrawn'].bit_count() > 2 * width:
                        raise AssertionError('witness exceeds twice the provenance width')
            current = left.read(0) == right.read(0)
            no_support = equivalent(left, right, check_support=False)['equivalent']
            structural = left.structural_key() == right.structural_key()
            if structural and not oracle:
                raise AssertionError('structural control falsely merged profiles')
            summary['pairs'] += 1
            summary['withdrawalChecks'] += len(masks)
            summary['equivalentPairs'] += int(oracle)
            summary['currentOnlyWrongPairs'] += int(current != oracle)
            summary['noSupportWrongPairs'] += int(no_support != oracle)
            summary['structuralMissedPairs'] += int(oracle and not structural)
            line(pairs_raw, {'case': label, 'candidate': result, 'oracleEquivalent': oracle,
                             'withdrawalChecks': len(masks), 'currentOnly': current,
                             'withoutSupport': no_support, 'structural': structural})
            return result

        def record(label, profile, masks, readouts):
            line(profiles_raw, {'id': label, 'profile': asdict(profile), 'withdrawals': masks,
                                'readouts': [encode(value) for value in readouts]})

        for index, spec in enumerate(plan['exhaustive']):
            slots = [(outcome, mask) for outcome in range(spec['outcomes']) for mask in spec['dependencyMasks']]
            profiles = [Profile(spec['sources'], spec['outcomes'],
                                tuple((outcome, mask, count) for (outcome, mask), count in zip(slots, values, strict=True) if count))
                        for values in product(spec['counts'], repeat=len(slots))]
            if len(profiles) != spec['profiles']:
                raise AssertionError('exhaustive profile count changed')
            masks = list(range(1 << spec['sources']))
            readouts = [[profile.read(mask) for mask in masks] for profile in profiles]
            for row, (profile, values) in enumerate(zip(profiles, readouts, strict=True)):
                record(f'exhaustive-{index}-{row}', profile, masks, values)
            before = summary['pairs']
            for first, left in enumerate(profiles):
                for second, right in enumerate(profiles):
                    check(f'exhaustive-{index}:{first}:{second}', left, right, masks, readouts[first], readouts[second])
            if summary['pairs'] - before != spec['orderedPairs']:
                raise AssertionError('exhaustive pair count changed')
            print(json.dumps({'exhaustive': index, 'pairs': spec['orderedPairs']}), flush=True)

        spec = plan['random']
        rng = random.Random(spec['seed'])
        allowed = [mask for mask in range(1 << spec['sources']) if mask.bit_count() <= spec['maximumWidth']]
        masks = list(range(1 << spec['sources']))
        for index in range(spec['pairs']):
            pair = [Profile(spec['sources'], spec['outcomes'],
                            tuple((rng.randrange(spec['outcomes']), rng.choice(allowed), rng.randint(1, spec['maximumCount']))
                                  for _ in range(spec['recordsPerProfile']))) for _ in range(2)]
            values = [[profile.read(mask) for mask in masks] for profile in pair]
            for side, (profile, observations) in enumerate(zip(pair, values, strict=True)):
                record(f'random-{index}-{side}', profile, masks, observations)
            check(f'random-{index}', *pair, masks, *values)
        print(json.dumps({'randomPairs': spec['pairs']}), flush=True)

        spec = plan['factorCases']
        rng = random.Random(spec['seed'])
        for sources in spec['sourceCounts']:
            left, right = multiplied_profile(sources, spec['outcomes'])
            exhaustive = sources <= spec['fullEnumerationMaximumSources']
            if exhaustive:
                masks = list(range(1 << sources))
            else:
                chosen = {0, (1 << sources) - 1}
                while len(chosen) < spec['sampledWithdrawalsOtherwise']:
                    chosen.add(rng.randrange(1 << sources))
                masks = sorted(chosen)
            values = [[profile.read(mask) for mask in masks] for profile in (left, right)]
            for side, (profile, observations) in enumerate(zip((left, right), values, strict=True)):
                record(f'factor-{sources}-{side}', profile, masks, observations)
            result = check(f'factor-{sources}', left, right, masks, *values)
            if not result['equivalent'] or left.structural_key() == right.structural_key():
                raise AssertionError('declared Boolean-factor distinction did not hold')
            cohorts.append({'sources': sources, 'originalRecords': len(left.records),
                            'multipliedRecords': len(right.records), 'oracleChecks': len(masks),
                            'exhaustiveOracle': exhaustive, 'candidate': result})
        return {'complete': True, **summary, 'factorCases': cohorts, 'seconds': time.monotonic() - start,
                'scope': 'Exact empirical predictive-profile equivalence; not a state minimizer or performance claim',
                'performanceQualified': False, 'noveltyEstablished': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parent
    frozen = args.output / 'source'
    frozen.mkdir()
    paths = [*sorted(source.glob('*.py')), source / 'PLAN.json']
    for path in paths:
        shutil.copy2(path, frozen / path.name)
    hashes = {path.name: digest(path) for path in paths}
    write(args.output / 'freeze.json', {'schema': 'warm.source-prediction-freeze/1', 'files': hashes,
                                      'startedAt': datetime.now(timezone.utc).isoformat()})
    result = {'complete': False, 'error': 'Interrupted before completion'}
    try:
        result = run(args.output, json.loads((frozen / 'PLAN.json').read_text()))
        if any(digest(source / name) != value or digest(frozen / name) != value for name, value in hashes.items()):
            raise RuntimeError('research source changed during the run')
        result['rawSha256'] = {name: digest(args.output / name) for name in ('profiles.ndjson', 'pairs.ndjson')}
        result['freezeSha256'] = digest(args.output / 'freeze.json')
        result['finishedAt'] = datetime.now(timezone.utc).isoformat()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(args.output / 'finished.json', result)
