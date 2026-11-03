# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Exact source-withdrawal certificates using finite-field exterior lifts."""

import argparse
from dataclasses import dataclass
import hashlib
from itertools import combinations, product
import json
from math import comb, isqrt
from pathlib import Path
import random
import struct
import time
import tracemalloc


@dataclass(frozen=True)
class Record:
    ordinal: int
    sources: tuple
    values: tuple


def validate(records, width, prime, budget, fanin):
    for value, lower, upper in ((width, 1, 64), (prime, 2, 65521), (budget, 0, 4), (fanin, 1, 3)):
        if type(value) is not int or not lower <= value <= upper:
            raise ValueError('invalid finite-screen parameter')
    if any(prime % divisor == 0 for divisor in range(2, isqrt(prime) + 1)):
        raise ValueError('field modulus must be prime')
    if width * comb(budget + fanin, fanin) > 4096 or len(records) > 4096:
        raise ValueError('screen allocation bound exceeded')
    seen = set()
    for row in records:
        if not isinstance(row, Record) or type(row.ordinal) is not int or not 0 <= row.ordinal < 2**32 or row.ordinal in seen:
            raise ValueError('invalid or duplicate record identity')
        seen.add(row.ordinal)
        if not isinstance(row.sources, tuple) or not 1 <= len(row.sources) <= fanin:
            raise ValueError('invalid provenance width')
        if any(type(source) is not int or not 1 <= source <= prime - budget - fanin for source in row.sources):
            raise ValueError('source ID outside the reserved field universe')
        if tuple(sorted(set(row.sources))) != row.sources:
            raise ValueError('provenance must be sorted and unique')
        if not isinstance(row.values, tuple) or len(row.values) != width:
            raise ValueError('invalid equation width')
        if any(type(value) is not int or not 0 <= value < prime for value in row.values):
            raise ValueError('coefficient outside the prime field')


class Basis:
    def __init__(self, prime):
        self.prime, self.rows = prime, {}

    def add(self, values):
        row = list(values)
        for pivot, base in sorted(self.rows.items()):
            scale = row[pivot]
            if scale:
                row = [(value - scale * other) % self.prime for value, other in zip(row, base)]
        pivot = next((i for i, value in enumerate(row) if value), None)
        if pivot is None:
            return False
        inverse = pow(row[pivot], -1, self.prime)
        self.rows[pivot] = tuple(value * inverse % self.prime for value in row)
        return True


def reference_space(rows, width, prime):
    matrix = [list(row) for row in rows]
    pivot_row = 0
    for column in range(width):
        candidate = next((i for i in range(pivot_row, len(matrix)) if matrix[i][column]), None)
        if candidate is None:
            continue
        matrix[candidate], matrix[pivot_row] = matrix[pivot_row], matrix[candidate]
        inverse = pow(matrix[pivot_row][column], -1, prime)
        matrix[pivot_row] = [value * inverse % prime for value in matrix[pivot_row]]
        for index in range(len(matrix)):
            if index != pivot_row:
                scale = matrix[index][column]
                if scale:
                    matrix[index] = [(value - scale * base) % prime
                                     for value, base in zip(matrix[index], matrix[pivot_row])]
        pivot_row += 1
    return tuple(tuple(row) for row in matrix[:pivot_row])


def determinant(matrix, prime):
    result, rows = 1, [list(row) for row in matrix]
    for column in range(len(rows)):
        pivot = next((i for i in range(column, len(rows)) if rows[i][column]), None)
        if pivot is None:
            return 0
        if pivot != column:
            rows[pivot], rows[column] = rows[column], rows[pivot]
            result = -result
        value = rows[column][column]
        result = result * value % prime
        inverse = pow(value, -1, prime)
        for index in range(column + 1, len(rows)):
            scale = rows[index][column] * inverse % prime
            rows[index] = [(a - scale * b) % prime for a, b in zip(rows[index], rows[column])]
    return result % prime


def exterior(sources, budget, fanin, prime):
    padded = (*sources, *range(prime - fanin + 1, prime - fanin + 1 + fanin - len(sources)))
    return tuple(determinant([[pow(source, degree, prime) for source in padded]
                              for degree in degrees], prime)
                 for degrees in combinations(range(budget + fanin), fanin))


def lift_certificate(records, width, prime, budget, fanin):
    validate(records, width, prime, budget, fanin)
    basis, retained, codes = Basis(prime), [], {}
    for row in records:
        if row.sources not in codes:
            codes[row.sources] = exterior(row.sources, budget, fanin, prime)
        code = codes[row.sources]
        lifted = tuple(value * weight % prime for weight in code for value in row.values)
        if basis.add(lifted):
            retained.append(row)
    return retained


def deletion_sets(sources, budget):
    for count in range(budget + 1):
        for values in combinations(sorted(sources), count):
            yield frozenset(values)


def surviving(records, deleted, budget):
    if not isinstance(deleted, frozenset) or any(type(value) is not int or value < 1 for value in deleted):
        raise ValueError('invalid source-withdrawal set')
    if len(deleted) > budget:
        raise ValueError('withdrawal exceeds the certified budget')
    return [row for row in records if not deleted.intersection(row.sources)]


def plain_basis(records, prime):
    basis, retained = Basis(prime), []
    for row in records:
        if basis.add(row.values):
            retained.append(row)
    return retained


def provenance_basis(records, prime):
    bases, retained = {}, []
    for row in records:
        basis = bases.setdefault(row.sources, Basis(prime))
        if basis.add(row.values):
            retained.append(row)
    return retained


def union_basis(records, prime, budget):
    sources = {source for row in records for source in row.sources}
    retained = {}
    for deleted in deletion_sets(sources, budget):
        for row in plain_basis(surviving(records, deleted, budget), prime):
            retained[row.ordinal] = row
    return sorted(retained.values(), key=lambda row: row.ordinal)


def quotient_repairs(records, prime):
    if any(len(row.sources) != 1 for row in records):
        raise ValueError('single-withdrawal repair control requires one source per row')
    primary = plain_basis(records, prime)
    retained = {row.ordinal: row for row in primary}
    for source in sorted({row.sources[0] for row in primary}):
        basis = Basis(prime)
        for row in primary:
            if row.sources[0] != source:
                basis.add(row.values)
        for row in records:
            if row.sources[0] != source and basis.add(row.values):
                retained[row.ordinal] = row
    return sorted(retained.values(), key=lambda row: row.ordinal)


def packed(records, width, prime, budget, fanin):
    validate(records, width, prime, budget, fanin)
    data = bytearray(struct.pack('<4sHBBB I', b'SRC1', prime, width, budget, fanin, len(records)))
    for row in records:
        data.extend(struct.pack('<IB', row.ordinal, len(row.sources)))
        data.extend(struct.pack('<' + 'H' * len(row.sources), *row.sources))
        data.extend(struct.pack('<' + 'H' * width, *row.values))
    return bytes(data)


def checked_case(records, width, prime, budget, fanin, *, controls=True, variants=None):
    original = {row.ordinal: row for row in records}
    if variants is None:
        variants = {'lift': lift_certificate(records, width, prime, budget, fanin)}
        if controls:
            variants['provenance'] = provenance_basis(records, prime)
            variants['union'] = union_basis(records, prime, budget)
            if budget == 1 and fanin == 1:
                variants['quotient-repair'] = quotient_repairs(records, prime)
    rank = len(reference_space([row.values for row in records], width, prime))
    bound = rank * comb(budget + fanin, fanin)
    if len(variants['lift']) > bound:
        raise AssertionError('lift violates the proposed retained-row bound')
    for name, retained in variants.items():
        if any(original.get(row.ordinal) != row for row in retained):
            raise AssertionError('certificate changed original evidence: ' + name)
    states = []
    for deleted in deletion_sets({source for row in records for source in row.sources}, budget):
        expected = reference_space([row.values for row in surviving(records, deleted, budget)], width, prime)
        for name, retained in variants.items():
            actual = reference_space([row.values for row in surviving(retained, deleted, budget)], width, prime)
            if actual != expected:
                raise AssertionError(f'changed row span: {name}, {sorted(deleted)}')
        states.append({'deleted': sorted(deleted), 'span': expected})
    return {'rank': rank, 'bound': bound, 'retained': {name: [row.ordinal for row in rows] for name, rows in variants.items()},
            'states': states, 'packedBytes': {name: len(packed(rows, width, prime, budget, fanin)) for name, rows in variants.items()}}


def spaces(prime):
    return [(), ((0, 1),), *[((1, value),) for value in range(prime)], ((1, 0), (0, 1))]


def records_from_groups(groups, supports):
    rows = []
    for group, support in zip(groups, supports):
        start = len(rows)
        rows.extend(Record(start + index, support, tuple(values)) for index, values in enumerate(group))
    return rows


def generated(family, seed, plan):
    rng = random.Random(seed)
    prime, width, sources = plan['field'], plan['width'], plan['sources']
    hidden = [rng.randrange(prime) for _ in range(width - 1)]
    result = []
    for bundle in range(plan['bundles']):
        fanin = 2 if family == 'redundant-source-pairs' else 1
        support = tuple(sorted(rng.sample(range(1, sources + 1), fanin)))
        for _ in range(plan['rowsPerBundle']):
            coefficients = [rng.randrange(prime) for _ in range(width - 1)]
            if family == 'rare-direction':
                coefficients[-1] = 0
            value = sum(a * b for a, b in zip(coefficients, hidden)) % prime
            result.append(Record(len(result), support, (*coefficients, value)))
    if family == 'rare-direction':
        result.append(Record(len(result), (sources + 1,), (*([0] * (width - 2)), 1, hidden[-1])))
    elif family == 'contradiction':
        result.append(Record(len(result), (sources + 1,), (*([0] * (width - 1)), 1)))
    return result


def measured(function):
    tracemalloc.start()
    started = time.perf_counter()
    try:
        value = function()
        elapsed, peak = time.perf_counter() - started, tracemalloc.get_traced_memory()[1]
    finally:
        tracemalloc.stop()
    return value, {'secondsWithTracing': elapsed, 'peakTracedBytes': peak}


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.output.is_absolute():
        parser.error('output must be absolute')
    args.output.mkdir()
    here = Path(__file__).parent
    plan = json.loads((here / 'PLAN.json').read_text())
    freeze = {'plan': plan, 'files': {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
              for path in [here / 'PLAN.json', here / 'probe.py', here / 'test_probe.py']}}
    write(args.output / 'freeze.json', freeze)
    cases, states = 0, 0
    started = time.perf_counter()
    with (args.output / 'exhaustive.ndjson').open('w') as log:
        small = plan['exhaustive']
        for mode in small['provenanceModes']:
            supports = [(1,), (2,), (3,)] if mode == 'single-source' else [(1, 2), (1, 3), (2, 3)]
            fanin = len(supports[0])
            for groups in product(spaces(small['field']), repeat=small['sourceGroups']):
                records = records_from_groups(groups, supports)
                for budget in small['withdrawalBudgets']:
                    result = checked_case(records, small['width'], small['field'], budget, fanin)
                    log.write(json.dumps({'case': cases, 'mode': mode, 'groups': groups, 'budget': budget, **result}) + '\n')
                    cases += 1
                    states += len(result['states'])
    tight_cases = []
    for rank, budget, fanin in product(plan['tightCases']['ranks'], plan['tightCases']['withdrawalBudgets'], plan['tightCases']['provenanceWidths']):
        records = []
        for coordinate in range(rank):
            for support in combinations(range(1, budget + fanin + 1), fanin):
                records.append(Record(len(records), support, tuple(int(i == coordinate) for i in range(rank))))
        result = checked_case(records, rank, 257, budget, fanin)
        if len(result['retained']['lift']) != len(records):
            raise AssertionError('tight construction was not tight')
        tight_cases.append({'rank': rank, 'budget': budget, 'fanin': fanin, **result})
    write(args.output / 'tight.json', tight_cases)
    larger, screen = [], True
    settings = plan['larger']
    for family, seed in product(settings['families'], settings['seeds']):
        rows = generated(family, seed, settings)
        width, prime, budget = settings['width'], settings['field'], settings['withdrawalBudget']
        fanin = 2 if family == 'redundant-source-pairs' else 1
        folder = args.output / f'{family}-{seed}'
        folder.mkdir()
        write(folder / 'input.json', [{'ordinal': row.ordinal, 'sources': row.sources, 'values': row.values} for row in rows])
        variants, costs = {}, {}
        for name, fn in [('lift', lambda: lift_certificate(rows, width, prime, budget, fanin)),
                         ('provenance', lambda: provenance_basis(rows, prime)),
                         ('union', lambda: union_basis(rows, prime, budget))]:
            variants[name], costs[name] = measured(fn)
            (folder / f'{name}.bin').write_bytes(packed(variants[name], width, prime, budget, fanin))
        result = checked_case(rows, width, prime, budget, fanin, variants=variants)
        sizes = result['packedBytes']
        target = sizes['lift'] <= .5 * sizes['provenance'] and sizes['lift'] <= 1.1 * sizes['union']
        if family.startswith('redundant-'):
            screen = screen and target
        receipt = {'family': family, 'seed': seed, 'originalRows': len(rows), 'originalBytes': len(packed(rows, width, prime, budget, fanin)),
                   'costs': costs, 'mechanismScreen': target, **result}
        write(folder / 'result.json', receipt)
        larger.append({key: value for key, value in receipt.items() if key not in ('states', 'retained')})
        print(json.dumps(larger[-1]), flush=True)
    hashes = {str(path.relative_to(args.output)): hashlib.sha256(path.read_bytes()).hexdigest()
              for path in sorted(args.output.rglob('*')) if path.is_file()}
    finished = {'complete': True, 'correctnessPassed': True, 'exhaustiveCases': cases, 'exhaustiveStates': states,
                'tightCases': len(tight_cases), 'larger': larger, 'mechanismScreenPassed': screen,
                'elapsedSeconds': time.perf_counter() - started, 'files': hashes, 'benchmarkClaim': False}
    write(args.output / 'finished.json', finished)


if __name__ == '__main__':
    main()
