"""Exact state-abstraction and repeated-retraction screen with raw model receipts."""

import argparse
from dataclasses import asdict
from datetime import datetime, timezone
import json
from pathlib import Path
import random
import shutil
import time

from run import digest, write
from state import Model, canonical, minimize, quotient
from symbolic_control import BudgetExceeded
from test_state import brute_valid, partitions, raw_distribution, trace_distribution


def packed(model, mapping):
    return json.dumps({'model': asdict(model), 'stateMap': mapping}, sort_keys=True, separators=(',', ':')).encode()


def build_correlated(sources, weighting):
    records = []
    for state in range(4):
        for action in range(2):
            for source in range(sources):
                count = 1 if weighting == 'uniform' else 1 << source
                target = 2 * ((source + action) % 2) + state % 2
                if state % 2:
                    records.extend((state, action, target, (1 << source) | (1 << other), count)
                                   for other in range(sources))
                else:
                    records.append((state, action, target, 1 << source, count))
    return Model(sources, 2, (0, 0, 1, 1), tuple(records))


def audit(root, plan):
    started = time.monotonic()
    rng = random.Random(plan['random']['seed'])
    spec = plan['random']
    models = [(f'random-{index}', Model(spec['sources'], spec['actions'],
               tuple(rng.randrange(spec['labels']) for _ in range(spec['states'])),
               tuple((rng.randrange(spec['states']), rng.randrange(spec['actions']), rng.randrange(spec['states']),
                      rng.randrange(1 << spec['sources']), rng.randint(1, spec['maximumCount']))
                     for _ in range(spec['records'])))) for index in range(spec['models'])]
    models += [(f'correlated-{sources}-{weighting}', build_correlated(sources, weighting))
               for sources in plan['correlated']['sourceCounts'] for weighting in plan['correlated']['weights']]
    independent = Model(8, 2, (0, 0, 1, 1), tuple((state, action, 2 * action, 1 << (2 * state + action), 1)
                                               for state in range(4) for action in range(2)))
    models.append(('independent-histories', independent))
    models.append(('withdrawal-enables-new-merge', Model(2, 1, (0, 0, 1),
                  ((0, 0, 2, 1, 1), (0, 0, 0, 2, 1), (1, 0, 1, 1, 1), (1, 0, 1, 2, 1)))))
    verifier_rng = random.Random(plan['verification']['seed'])
    results = []
    total_checks = sequence_steps = current_wrong = 0
    for name, model in models:
        if time.monotonic() - started > plan['limits']['seconds']:
            raise TimeoutError('frozen state-abstraction time limit exceeded')
        candidate = minimize(model)
        partition = candidate['partition']
        reduced = quotient(model, partition)
        controls = {}
        if name.startswith('random-'):
            valid = [p for p in partitions(len(model.labels)) if brute_valid(model, p)]
            minimum = min(len(set(p)) for p in valid)
            optimal = [p for p in valid if len(set(p)) == minimum]
            if optimal != [partition]:
                raise AssertionError('independent partition enumeration disagrees: ' + name)
            controls['partitionEnumeration'] = {'partition': optimal[0], 'validPartitions': len(valid)}
        for descending in (False, True):
            order = 'diagram-descending' if descending else 'diagram-ascending'
            try:
                result = minimize(model, 'diagram', descending)
                if result['partition'] != partition:
                    raise AssertionError('diagram quotient differs: ' + name)
                controls[order] = {'complete': True, **result}
            except BudgetExceeded as error:
                controls[order] = {'complete': False, 'reason': str(error)}
        direct = minimize(model, 'source-normalized')['partition']
        current = minimize(model, 'current')['partition']
        controls['sourceNormalized'] = direct
        controls['currentOnly'] = current
        current_wrong += int(current != partition)
        if model.sources <= plan['verification']['exhaustiveWithdrawalMaximumSources']:
            masks = list(range(1 << model.sources))
        else:
            chosen = {0, (1 << model.sources) - 1}
            while len(chosen) < plan['verification']['sampledWithdrawalsOtherwise']:
                chosen.add(verifier_rng.randrange(1 << model.sources))
            masks = sorted(chosen)
        checks = 0
        for removed in masks:
            for state in range(len(model.labels)):
                for action in range(model.actions):
                    original = raw_distribution(model, state, action, removed)
                    compressed = raw_distribution(reduced, partition[state], action, removed)
                    expected = None if original is None else tuple(sum(value for index, value in enumerate(original)
                                                                     if partition[index] == block)
                                                                  for block in range(len(reduced.labels)))
                    if expected != compressed:
                        raise AssertionError('quotient changes a predictive transition: ' + name)
                    checks += 1
        sequences = []
        for _ in range(plan['verification']['sequentialOrdersPerModel']):
            order = list(range(model.sources))
            verifier_rng.shuffle(order)
            original, compacted, mapping = model, reduced, partition
            states_after = []
            for source in order:
                original, compacted = original.withdraw(1 << source), compacted.withdraw(1 << source)
                next_partition = minimize(compacted)['partition']
                mapping = canonical(tuple(next_partition[block] for block in mapping))
                compacted = quotient(compacted, next_partition)
                if mapping != minimize(original)['partition']:
                    raise AssertionError('recompaction differs from a fresh surviving-evidence model: ' + name)
                for state in range(len(model.labels)):
                    actions = tuple(index % model.actions for index in range(plan['verification']['traceHorizon']))
                    if trace_distribution(original, state, actions, 0) != trace_distribution(compacted, mapping[state], actions, 0):
                        raise AssertionError('recompaction changed a multi-step outcome distribution: ' + name)
                states_after.append(len(compacted.labels))
                sequence_steps += 1
            sequences.append({'sourceOrder': order, 'statesAfterEachWithdrawal': states_after})
        full_bytes = len(packed(model, tuple(range(len(model.labels)))))
        reduced_bytes = len(packed(reduced, partition))
        if name == 'independent-histories' and (partition != (0, 1, 2, 3) or reduced_bytes != full_bytes):
            raise AssertionError('independent source-support stress case unexpectedly compacted')
        result = {'case': name, 'model': asdict(model), 'candidate': candidate, 'controls': controls,
                  'quotient': asdict(reduced), 'withdrawalsChecked': masks, 'transitionChecks': checks,
                  'sequentialWithdrawals': sequences, 'fullPacketBytes': full_bytes, 'quotientPacketBytes': reduced_bytes}
        write(root / (name + '.json'), result)
        results.append({key: result[key] for key in ('case', 'transitionChecks', 'fullPacketBytes', 'quotientPacketBytes')})
        total_checks += checks
        if not name.startswith('random-'):
            print(json.dumps(results[-1]), flush=True)
    return {'complete': True, 'models': len(models), 'transitionChecks': total_checks,
            'sequentialWithdrawalSteps': sequence_steps, 'currentOnlyDifferentPartitions': current_wrong,
            'cases': results, 'seconds': time.monotonic() - started,
            'performanceQualified': False, 'noveltyEstablished': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parent
    frozen = args.output / 'source'
    frozen.mkdir()
    paths = [*sorted(source.glob('*.py')), *sorted(source.glob('*PLAN.json'))]
    for path in paths:
        shutil.copy2(path, frozen / path.name)
    hashes = {path.name: digest(path) for path in paths}
    write(args.output / 'freeze.json', {'schema': 'warm.source-prediction-state-freeze/1', 'files': hashes,
                                      'startedAt': datetime.now(timezone.utc).isoformat()})
    result = {'complete': False, 'error': 'Interrupted before completion'}
    try:
        result = audit(args.output, json.loads((frozen / 'STATE-PLAN.json').read_text()))
        if any(digest(source / name) != value or digest(frozen / name) != value for name, value in hashes.items()):
            raise RuntimeError('research source changed during state screen')
        result['rawSha256'] = {path.name: digest(path) for path in sorted(args.output.glob('*.json'))
                               if path.name != 'freeze.json'}
        result['freezeSha256'] = digest(args.output / 'freeze.json')
        result['finishedAt'] = datetime.now(timezone.utc).isoformat()
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(args.output / 'finished.json', result)
