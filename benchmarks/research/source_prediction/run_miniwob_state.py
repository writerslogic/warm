import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import json
from pathlib import Path

from sparse_state import canonical, minimize, quotient, withdraw


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    with path.open('xb') as stream:
        stream.write(encoded(value) + b'\n')


def readouts(states, records, partition):
    grouped = [defaultdict(Counter) for _ in range(states)]
    for state, action, target, source, count in records:
        grouped[state][action][partition[target]] += count
    return [{action: tuple((target, Fraction(count, sum(outcomes.values()))) for target, count in sorted(outcomes.items()))
             for action, outcomes in row.items()} for row in grouped]


def check(root):
    freeze = json.loads((root / 'freeze.json').read_text())
    for name, expected in freeze['source'].items():
        if digest(root / 'source' / name) != expected:
            raise ValueError('frozen source mismatch: ' + name)
    for name, expected in freeze['inputs'].items():
        if digest(root / 'inputs' / name) != expected:
            raise ValueError('frozen input mismatch: ' + name)
    return freeze


def main(root, output):
    freeze = check(root)
    results = []
    for name in sorted(freeze['inputs']):
        packet = json.loads((root / 'inputs' / name).read_text())
        states = packet['states']
        actions = sorted(packet['actions'])
        state_ids = {value: index for index, value in enumerate(states)}
        action_ids = {value: index for index, value in enumerate(actions)}
        records = [[state_ids[state], action_ids[action], state_ids[target], source, count]
                   for state, action, target, source, count in packet['records']]
        count, action_count = len(states), len(actions)
        safe = minimize(count, action_count, 128, records)
        current = minimize(count, action_count, 128, records, current=True)
        partition = safe['partition']
        compact = quotient(count, action_count, 128, records, partition)
        baseline_packet = {'states': states, 'actions': [packet['actions'][action] for action in actions], 'records': records}
        compact_packet = {**baseline_packet, 'records': compact, 'stateMap': partition}
        tag = name.removesuffix('-transitions.json')
        write(output / (tag + '-baseline.json'), baseline_packet)
        write(output / (tag + '-quotient.json'), compact_packet)
        active = {row[0] for row in records}
        blocks = max(partition) + 1
        baseline_reads = readouts(count, records, partition)
        compact_reads = readouts(blocks, compact, tuple(range(blocks)))
        if any(baseline_reads[state] != compact_reads[partition[state]] for state in range(count)):
            raise AssertionError('initial quotient readout mismatch')
        checks, repaired_packet, mapping = count, compact, partition
        sequence = []
        for removed_count in (1, 2, 4, 8, 16, 32, 64, 128):
            removed = set(range(removed_count))
            surviving = withdraw(records, removed)
            fresh = minimize(count, action_count, 128, surviving)['partition']
            retained = withdraw(repaired_packet, removed)
            old_blocks = max(mapping) + 1
            repair = minimize(old_blocks, action_count, 128, retained)['partition']
            updated = canonical(repair[block] for block in mapping)
            if fresh != updated:
                raise AssertionError('retained-model repair differs from full surviving evidence')
            repaired_packet = quotient(old_blocks, action_count, 128, retained, repair)
            mapping = updated
            actual = readouts(count, surviving, fresh)
            expected = readouts(max(mapping) + 1, repaired_packet, tuple(range(max(mapping) + 1)))
            if any(actual[state] != expected[mapping[state]] for state in range(count)):
                raise AssertionError('repaired quotient readout mismatch')
            checks += count
            sequence.append({'removedSources': removed_count, 'blocks': max(mapping) + 1,
                             'retainedRecords': len(repaired_packet), 'partitionSha256': hashlib.sha256(encoded(mapping)).hexdigest()})
        write(output / (tag + '-withdrawals.json'), sequence)
        results.append({'cell': tag, 'states': count, 'activeStates': len(active), 'safeBlocks': blocks,
                        'safeActiveBlocks': len({partition[state] for state in active}),
                        'currentBlocks': max(current['partition']) + 1,
                        'safeIterations': safe['iterations'], 'currentIterations': current['iterations'],
                        'baselineBytes': len(encoded(baseline_packet)), 'quotientBytes': len(encoded(compact_packet)),
                        'retainedRecords': len(compact), 'originalRecords': len(records),
                        'readoutChecks': checks, 'sequentialRetractions': len(sequence)})
    write(output / 'summary.json', results)
    check(root)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(root / 'freeze.json'), 'cells': len(results),
            'readoutChecks': sum(row['readoutChecks'] for row in results),
            'sequentialRetractions': sum(row['sequentialRetractions'] for row in results),
            'raw': {path.name: digest(path) for path in sorted(output.iterdir()) if path.is_file()}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, required=True)
    arguments = parser.parse_args()
    output = arguments.root / 'analysis-001'
    output.mkdir()
    try:
        result = main(arguments.root, output)
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(output / 'finished.json', result)
        print(json.dumps(result, sort_keys=True), flush=True)
