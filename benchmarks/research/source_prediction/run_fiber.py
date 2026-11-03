import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
from functools import reduce
import hashlib
import json
import lzma
from math import gcd
from pathlib import Path
import zlib

from fiber import compile_model, expand
from fiber_codec import decode
from run_miniwob_state import encoded, readouts, write
from sparse_state import minimize, quotient, withdraw


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def varint(value):
    if type(value) is not int or not 0 <= value < 1 << 64:
        raise ValueError('unsigned encoding overflow')
    result = bytearray()
    while value >= 128:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def vector(values):
    return varint(len(values)) + b''.join(varint(value) for value in values)


def rows(values):
    return varint(len(values)) + b''.join(varint(value) for row in values for value in row)


def normalize(records):
    grouped = defaultdict(Counter)
    for state, action, target, source, count in records:
        grouped[(state, action)][(target, source)] += count
    result = []
    for (state, action), counts in sorted(grouped.items()):
        scale = reduce(gcd, counts.values())
        result.extend([state, action, target, source, count // scale]
                      for (target, source), count in sorted(counts.items()))
    return result


def adjacency(records, columns=(0, 1, 3, 2)):
    if not columns:
        if len(records) != 1:
            raise ValueError('duplicate packed adjacency leaf')
        return varint(records[0][4])
    groups = defaultdict(list)
    for row in records:
        groups[row[columns[0]]].append(row)
    result, previous = bytearray(varint(len(groups))), 0
    for key, group in sorted(groups.items()):
        result.extend(varint(key - previous))
        result.extend(adjacency(group, columns[1:]))
        previous = key
    return bytes(result)


def common(states, actions):
    result = bytearray(b'WPF1' + varint(len(states)))
    for state in states:
        if not isinstance(state, str) or len(state) != 64 or bytes.fromhex(state).hex() != state:
            raise ValueError('unexpected original-state hash')
        result.extend(bytes.fromhex(state))
    result.extend(varint(len(actions)))
    for action in actions:
        value = encoded(action)
        result.extend(varint(len(value)))
        result.extend(value)
    return bytes(result)


def compressions(formats, output, stem):
    result = {}
    for name, content in formats.items():
        for codec, value in [('raw', content), ('zlib9', zlib.compress(content, 9)),
                             ('xz6', lzma.compress(content, preset=6))]:
            path = output / (stem + '-' + name + '-' + codec + '.bin')
            with path.open('xb') as stream:
                stream.write(value)
            if codec == 'zlib9' and zlib.decompress(value) != content:
                raise AssertionError('zlib roundtrip mismatch')
            if codec == 'xz6' and lzma.decompress(value) != content:
                raise AssertionError('XZ roundtrip mismatch')
            result[name + '/' + codec] = len(value)
    return result


def verify(root, freeze):
    for folder in ('source', 'inputs'):
        for name, expected in freeze[folder].items():
            if digest(root / folder / name) != expected:
                raise ValueError('frozen file changed: ' + name)


def main(root, output):
    freeze = json.loads((root / 'freeze.json').read_text())
    verify(root, freeze)
    results = []
    for name in sorted(freeze['inputs']):
        source = json.loads((root / 'inputs' / name).read_text())
        states, action_keys = source['states'], sorted(source['actions'])
        state_ids = {state: index for index, state in enumerate(states)}
        action_ids = {action: index for index, action in enumerate(action_keys)}
        actions = [source['actions'][key] for key in action_keys]
        records = [[state_ids[state], action_ids[action], state_ids[target], source_id, count]
                   for state, action, target, source_id, count in source['records']]
        n, a = len(states), len(actions)
        packet = compile_model(n, a, 128, records)
        expanded = expand(packet, a, 128)
        mapping, instances = packet['stateMap'], len(packet['instances'])
        combined = records + [[state + n, action, target + n, source_id, count]
                              for state, action, target, source_id, count in expanded]
        checks, sequence = 0, []
        for removed_count in (0, 1, 2, 4, 8, 16, 32, 64, 128):
            surviving = withdraw(combined, set(range(removed_count)))
            partition = minimize(n + instances, a, 128, surviving)['partition']
            if any(partition[state] != partition[n + target] for state, target in enumerate(mapping)):
                raise AssertionError('template differs under a possible withdrawal')
            values = readouts(n + instances, surviving, partition)
            if any(values[state] != values[n + target] for state, target in enumerate(mapping)):
                raise AssertionError('template exact rational readout differs')
            checks += n
            sequence.append({'removedSources': removed_count, 'partitionSha256': hashlib.sha256(encoded(partition)).hexdigest()})
        safe = minimize(n, a, 128, records)['partition']
        compact = quotient(n, a, 128, records, safe)
        prefix = common(states, actions)
        control_formats = {}
        for label, body, state_map in [('original', records, None), ('quotient', compact, safe)]:
            base = {'states': states, 'actions': actions, 'records': body}
            if state_map is not None:
                base['stateMap'] = state_map
            control_formats[label + '-json'] = encoded(base)
            header = prefix + (b'F' if state_map is None else b'Q' + vector(state_map))
            normalized = normalize(body)
            control_formats[label + '-rows'] = header + rows(normalized)
            control_formats[label + '-adjacency'] = header + adjacency(normalized)
            wanted = {**base, 'records': normalized}
            if state_map is not None:
                wanted['stateMap'] = list(state_map)
            for mode in ('rows', 'adjacency'):
                if decode(control_formats[label + '-' + mode], mode == 'adjacency') != wanted:
                    raise AssertionError('control packed roundtrip mismatch')
        candidate = {'states': states, 'actions': actions, **packet}
        candidate_formats = {'json': encoded(candidate),
                             'packed': prefix + b'T' + varint(packet['controlCount']) + vector(mapping)
                             + rows([[control + 1, source_id + 1] for control, source_id in packet['instances']])
                             + rows(packet['rules']) + adjacency(normalize(packet['fallback']))}
        if decode(candidate_formats['packed']) != {**candidate, 'fallback': normalize(packet['fallback'])}:
            raise AssertionError('candidate packed roundtrip mismatch')
        stem = name.removesuffix('-transitions.json')
        controls = compressions(control_formats, output, stem + '-control')
        candidates = compressions(candidate_formats, output, stem + '-candidate')
        write(output / (stem + '-packet.json'), candidate)
        write(output / (stem + '-checks.json'), sequence)
        control_best, candidate_best = min(controls.values()), min(candidates.values())
        task = stem.removesuffix('-target-path').removesuffix('-raw')
        results.append({'cell': stem, 'task': task, 'states': n, 'activeStates': len({row[0] for row in records}),
                        'factoredActiveStates': sum(packet['instances'][mapping[s]][0] >= 0 for s in {row[0] for row in records}),
                        'controls': packet['controlCount'], 'instances': instances,
                        'rules': len(packet['rules']), 'fallbackRecords': len(packet['fallback']),
                        'originalRecords': len(records), 'expandedRecords': len(expanded),
                        'controlBytes': controls, 'candidateBytes': candidates,
                        'bestControlBytes': control_best, 'bestCandidateBytes': candidate_best,
                        'candidateRatio': candidate_best / control_best, 'readoutChecks': checks})
    write(output / 'summary.json', results)
    tasks = {row['task'] for row in results if row['candidateRatio'] <= .8}
    verify(root, freeze)
    return {'complete': True, 'completedAt': datetime.now(timezone.utc).isoformat(),
            'freezeSha256': digest(root / 'freeze.json'), 'cells': len(results),
            'readoutChecks': sum(row['readoutChecks'] for row in results),
            'promotionPassed': len(tasks) >= 2 and all(row['candidateRatio'] <= 1.05 for row in results),
            'raw': {path.name: digest(path) for path in sorted(output.iterdir()) if path.is_file()}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, required=True)
    args = parser.parse_args()
    output = args.root / 'analysis-001'
    output.mkdir()
    try:
        result = main(args.root, output)
    except Exception as error:
        result = {'complete': False, 'error': {'type': type(error).__name__, 'message': str(error)}}
        raise
    finally:
        write(output / 'finished.json', result)
        print(json.dumps({key: value for key, value in result.items() if key != 'raw'}), flush=True)
