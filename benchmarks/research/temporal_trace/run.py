# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Freeze and exhaustively audit temporal trace certificates on local inputs."""

import argparse
from dataclasses import asdict
from datetime import datetime, timezone
import hashlib
from itertools import product
import json
from pathlib import Path
import platform
import random
import shutil
import time

from probe import Certificate, Index, Record, compile_certificate, execute, expression_slots, packet, reference, withdrawals


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write('\n')


def emit(stream, value):
    stream.write(json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False) + '\n')


def row(ordinal, slot, value, start, known, sources, end=None):
    text = f'Observation {ordinal}: channel {slot} has value {value} from time {start}; recorded at {known}.'
    return Record(ordinal, slot, value, start, end, known, tuple(sorted(sources)), text)


def audit(records, programs, snapshots, budget, state_stream=None, case=None):
    selected, details = compile_certificate(records, programs, snapshots, budget)
    heads, head_details = compile_certificate(records, programs, snapshots, budget, 'heads')
    negative, negative_details = compile_certificate(records, programs, snapshots, budget, 'positive-only')
    slots = set().union(*(expression_slots(program) for program in programs))
    indexed = [item for item in records if item.slot in slots and any(
        item.valid_from <= valid and (item.valid_to is None or valid < item.valid_to) and item.recorded_at <= known
        for valid, known in snapshots)]
    variants = {'trace': selected, 'heads': heads, 'indexed': indexed}
    readers = {name: Certificate(items, programs, snapshots, budget) for name, items in variants.items()}
    for items in variants.values():
        original = {item.ordinal: item for item in records}
        if any(original.get(item.ordinal) != item for item in items):
            raise AssertionError('compiler changed an original record')
    negative_reader = Certificate(negative, programs, snapshots, budget)
    union, states, negative_failures, state_hash = set(), 0, 0, hashlib.sha256()
    negative_answer_failures = 0
    for deleted in withdrawals(records, budget):
        for snapshot in snapshots:
            expected = reference(records, programs, snapshot, deleted)
            for name, reader in readers.items():
                if reader.evaluate(snapshot, deleted) != expected:
                    raise AssertionError(f'{case}: {name} changed output/trace at {snapshot}, {sorted(deleted)}')
            negative_answer = negative_reader.evaluate(snapshot, deleted)
            negative_failures += negative_answer != expected
            negative_answer_failures += negative_answer[0] != expected[0]
            union.update(ordinal for _, ordinal in expected[1] if ordinal is not None)
            state = {'case': case, 'deleted': sorted(deleted), 'snapshot': snapshot,
                     'outputs': expected[0], 'trace': expected[1],
                     'positiveOnlyMatches': negative_answer == expected,
                     'positiveOnlyAnswersMatch': negative_answer[0] == expected[0]}
            encoded = json.dumps(state, sort_keys=True, separators=(',', ':')).encode()
            state_hash.update(encoded + b'\n')
            if state_stream is not None:
                state_stream.write(encoded.decode() + '\n')
            states += 1
    exhaustive = [item for item in sorted(records, key=lambda item: item.ordinal) if item.ordinal in union]
    if packet(exhaustive, programs, snapshots, budget) != packet(selected, programs, snapshots, budget):
        raise AssertionError('recursive trace union differs from exhaustive union')
    packets = {name: packet(items, programs, snapshots, budget) for name, items in variants.items()}
    return {'case': case, 'states': states, 'stateSha256': state_hash.hexdigest(),
            'selected': {name: [item.ordinal for item in items] for name, items in variants.items()},
            'packetBytes': {name: len(data) for name, data in packets.items()},
            'packetSha256': {name: hashlib.sha256(data).hexdigest() for name, data in packets.items()},
            'traceCompiler': details, 'headCompiler': head_details, 'positiveOnlyCompiler': negative_details,
            'negativeControlMismatches': negative_failures, 'negativeControlAnswerMismatches': negative_answer_failures,
            'sizeScreen': len(packets['trace']) <= .7 * min(len(packets['heads']), len(packets['indexed']))}, packets


def random_case(seed, spec):
    rng, records = random.Random(seed), []
    for ordinal in range(spec['records']):
        start = rng.randrange(7)
        end = start + rng.randint(1, 4) if rng.randrange(2) else None
        records.append(row(ordinal, rng.randrange(spec['slots']), rng.randrange(3), start, rng.randrange(9),
                           rng.sample(range(1, spec['sources'] + 1), rng.randint(1, 2)), end))
    programs = [('if', ('eq', 0, 1), ('or', ('eq', 1, 0), ('eq', 2, 2)),
                 ('and', ('not', ('eq', 3, 1)), ('eq', 4, 0))),
                ('eq', 5, 1), ('eq', 6, 1), ('not', ('eq', 7, 2))]
    return records, programs


def cohort_case(family, count, seed, spec):
    rng, records = random.Random(seed), []
    for slot in range(3):
        support = (slot + 1, count) if family == 'joint-guards' else (slot + 1,)
        records.append(row(len(records), slot, 1, 0, 0, support))
    for slot in range(3, 5 + spec['coldSlots']):
        for revision in range(spec['revisions']):
            support = rng.sample(range(4, count + 1), rng.randint(1, 2))
            records.append(row(len(records), slot, rng.randrange(3), revision, revision + rng.randrange(2), support))
    guard = ('or', ('eq', 0, 1), ('or', ('eq', 1, 1), ('eq', 2, 1)))
    programs = [('if', guard, ('eq', 3, 1), ('eq', slot, 1)) for slot in range(5, 5 + spec['coldSlots'])]
    programs.append(('eq', 4, 1))
    if family == 'unconditional':
        programs = [('eq', slot, 1) for slot in range(5 + spec['coldSlots'])]
    return records, programs


def run(output):
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).parent
    files = sorted([*source.glob('*.py'), source / 'PLAN.json'])
    (output / 'source').mkdir()
    hashes = {path.name: digest(path) for path in files}
    for path in files:
        shutil.copyfile(path, output / 'source' / path.name)
    freeze = {'schema': 'warm.temporal-trace.freeze/1', 'sourceSha256': hashes,
              'startedAt': datetime.now(timezone.utc).isoformat(), 'python': platform.python_version(),
              'machine': platform.machine(), 'mode': 'local-development', 'qualifiedClaim': False}
    write(output / 'freeze.json', freeze)
    plan = json.loads((source / 'PLAN.json').read_text())
    witness = [row(0, 0, 1, 0, 0, (1,)), row(1, 0, 0, 1, 1, (2,)), row(2, 1, 1, 0, 0, (3,))]
    witness_programs, witness_snapshots = [('eq', 0, 1), ('eq', 1, 1)], [(2, 2)]
    bad, _ = compile_certificate(witness, witness_programs, witness_snapshots, 1, 'positive-only')
    witness_expected = reference(witness, witness_programs, (2, 2), frozenset({2}))
    witness_actual = execute(Index(bad), witness_programs, (2, 2), frozenset({2}))[0]
    if witness_expected[0] == witness_actual[0]:
        raise AssertionError('false-guard resurrection negative control unexpectedly passed')
    write(output / 'counterexample.json', {'records': [asdict(item) for item in witness],
                                          'programs': witness_programs, 'snapshot': (2, 2), 'deleted': [2],
                                          'expected': witness_expected, 'positiveOnly': witness_actual})
    total_cases, total_states, negative_failures = 0, 0, 0
    spec = plan['exhaustive']
    programs = [('eq', 0, 1), ('not', ('eq', 1, 0)),
                ('and', ('eq', 0, 2), ('eq', 1, 1)),
                ('if', ('eq', 1, 1), ('eq', 0, 0), ('not', ('eq', 0, 2)))]
    snapshots = list(map(tuple, spec['snapshots']))
    with (output / 'exhaustive.ndjson').open('x') as summaries, (output / 'exhaustive-states.ndjson').open('x') as states:
        for values, sources in product(product(spec['values'], repeat=4), product(spec['sources'], repeat=4)):
            records = [row(i, i // 2, values[i], i % 2, i % 2, (sources[i],)) for i in range(4)]
            for budget in spec['budgets']:
                result, _ = audit(records, programs, snapshots, budget, states, f'exhaustive-{total_cases}')
                result.update({'values': values, 'sources': sources, 'budget': budget})
                emit(summaries, result)
                total_cases += 1
                total_states += result['states']
                negative_failures += result['negativeControlAnswerMismatches']
    print(json.dumps({'exhaustiveCases': total_cases, 'snapshotWithdrawalChecks': total_states}), flush=True)
    spec = plan['random']
    with (output / 'random.ndjson').open('x') as summaries, (output / 'random-states.ndjson').open('x') as states:
        for seed in spec['seeds']:
            records, programs = random_case(seed, spec)
            result, _ = audit(records, programs, list(map(tuple, spec['snapshots'])), spec['budget'], states, f'random-{seed}')
            result.update({'seed': seed, 'records': [asdict(item) for item in records], 'programs': programs})
            emit(summaries, result)
            total_cases += 1
            total_states += result['states']
            negative_failures += result['negativeControlAnswerMismatches']
    spec, cohort = plan['cohort'], []
    for family, count, seed in product(spec['families'], spec['sourceCounts'], spec['seeds']):
        name = f'{family}-{count}-{seed}'
        directory = output / name
        directory.mkdir()
        records, programs = cohort_case(family, count, seed, spec)
        snapshots = list(map(tuple, spec['snapshots']))
        write(directory / 'input.json', {'records': [asdict(item) for item in records], 'programs': programs,
                                         'snapshots': snapshots, 'budget': spec['budget']})
        with (directory / 'states.ndjson').open('x') as states:
            started = time.perf_counter()
            result, packets = audit(records, programs, snapshots, spec['budget'], states, name)
            result.update({'family': family, 'sources': count, 'seed': seed,
                           'developmentAuditSeconds': time.perf_counter() - started})
        for method, data in packets.items():
            with (directory / f'{method}.json').open('xb') as stream:
                stream.write(data)
        write(directory / 'result.json', result)
        cohort.append(result)
        total_cases += 1
        total_states += result['states']
        negative_failures += result['negativeControlAnswerMismatches']
        print(json.dumps({key: result[key] for key in ('case', 'states', 'packetBytes', 'traceCompiler', 'headCompiler', 'sizeScreen')}), flush=True)
    if not negative_failures:
        raise AssertionError('negative control was not falsified')
    if hashes != {path.name: digest(path) for path in files}:
        raise AssertionError('source changed during the frozen experiment')
    required = [item for item in cohort if item['family'] != 'unconditional']
    finished = {'schema': 'warm.temporal-trace.finished/1', 'completedAt': datetime.now(timezone.utc).isoformat(),
                'cases': total_cases, 'snapshotWithdrawalChecks': total_states,
                'negativeControlAnswerMismatches': negative_failures, 'correctnessPassed': True,
                'sizeScreenPassed': all(item['sizeScreen'] for item in required),
                'sizeCellsPassed': sum(item['sizeScreen'] for item in required), 'sizeCellsRequired': len(required),
                'qualifiedClaim': False, 'freezeSha256': digest(output / 'freeze.json'),
                'filesSha256': {str(path.relative_to(output)): digest(path) for path in sorted(output.rglob('*')) if path.is_file()}}
    write(output / 'finished.json', finished)
    print(json.dumps({key: value for key, value in finished.items() if key != 'filesSha256'}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True, type=Path)
    run(parser.parse_args().output)
