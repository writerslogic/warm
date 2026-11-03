# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Exact observability closure for a declared family of memory mutations."""

import argparse
from fractions import Fraction
import hashlib
from itertools import product
import json
from pathlib import Path
import random
import time


def dot(left, right):
    if len(left) != len(right):
        raise ValueError('incompatible vector dimensions')
    return sum((a * b for a, b in zip(left, right)), Fraction(0))


def apply(matrix, vector):
    return tuple(dot(row, vector) for row in matrix)


class Basis:
    def __init__(self, width):
        if type(width) is not int or not 1 <= width <= 256:
            raise ValueError('screen dimension must be in 1..256')
        self.width, self.rows = width, []

    def insert(self, values):
        if len(values) != self.width:
            raise ValueError('wrong observation dimension')
        row = list(map(Fraction, values))
        for pivot, base in self.rows:
            scale = row[pivot]
            row = [v - scale * b for v, b in zip(row, base)]
        pivot = next((i for i, value in enumerate(row) if value), None)
        if pivot is None:
            return False
        scale = row[pivot]
        row = [value / scale for value in row]
        self.rows = [(p, tuple(value - old[pivot] * b for value, b in zip(old, row)))
                     for p, old in self.rows]
        self.rows.append((pivot, tuple(row)))
        self.rows.sort()
        return True

    def coordinates(self, row):
        if len(row) != self.width:
            raise ValueError('wrong target dimension')
        coefficients = tuple(Fraction(row[pivot]) for pivot, _ in self.rows)
        rebuilt = tuple(sum((coefficient * base[i] for coefficient, (_, base) in zip(coefficients, self.rows)), Fraction(0))
                        for i in range(self.width))
        if rebuilt != tuple(row):
            raise ValueError('observation is outside the encoded row space')
        return coefficients

    @property
    def matrix(self):
        return tuple(row for _, row in self.rows)


def contrasts(rows, outputs):
    if type(rows) is not int or type(outputs) is not int or rows < 1 or outputs < 2 or rows * outputs > 256:
        raise ValueError('invalid association-table shape')
    result = []
    for row in range(rows):
        for output in range(1, outputs):
            contrast = [0] * (rows * outputs)
            contrast[row * outputs], contrast[row * outputs + output] = -1, 1
            result.append(tuple(contrast))
    return tuple(result)


def masks(rows, outputs, contract):
    contrasts(rows, outputs)
    if contract not in ('row', 'cell'):
        raise ValueError('unknown mutation contract')
    groups = [list(range(row * outputs, (row + 1) * outputs)) for row in range(rows)] if contract == 'row' else [[i] for i in range(rows * outputs)]
    return tuple(tuple(int(i not in group) for i in range(rows * outputs)) for group in groups)


class Compiler:
    def __init__(self, observations, operations):
        if not observations:
            raise ValueError('missing observations')
        self.basis = Basis(len(observations[0]))
        self.operations = tuple(tuple(mask) for mask in operations)
        if any(len(mask) != self.basis.width or any(type(v) is not int or v not in (0, 1) for v in mask) for mask in self.operations):
            raise ValueError('invalid deletion mask')
        for row in observations:
            self.basis.insert(row)
        self.rank_trace = [len(self.basis.rows)]
        while True:
            before = len(self.basis.rows)
            for row in self.basis.matrix:
                for mask in self.operations:
                    self.basis.insert([value * keep for value, keep in zip(row, mask)])
            self.rank_trace.append(len(self.basis.rows))
            if len(self.basis.rows) == before:
                break
        self.readout = tuple(self.basis.coordinates(row) for row in observations)
        self.updates = tuple(tuple(self.basis.coordinates([value * keep for value, keep in zip(row, mask)])
                                   for row in self.basis.matrix) for mask in self.operations)

    def encode(self, state):
        return apply(self.basis.matrix, state)

    def update(self, encoded, operation, supplied=None):
        result = apply(self.updates[operation], encoded)
        if supplied is None:
            return result
        return tuple(a + b for a, b in zip(result, self.encode(supplied)))

    def observe(self, encoded):
        return apply(self.readout, encoded)


def winner_from_contrasts(values, cue, outputs):
    scores = [Fraction(0)]
    scores += [sum((cue[row] * values[row * (outputs - 1) + output - 1] for row in range(len(cue))), Fraction(0))
               for output in range(1, outputs)]
    return max(range(outputs), key=lambda index: (scores[index], -index))


def screen(plan):
    spec = plan['screen']
    rows, outputs = spec['rows'], spec['outputs']
    observations = contrasts(rows, outputs)
    results = []
    for contract in ['row', 'cell']:
        operations = masks(rows, outputs, contract)
        compiler = Compiler(observations, operations)
        states = updates = readouts = 0
        for state in product(spec['weights'], repeat=rows * outputs):
            encoded = compiler.encode(state)
            observed = compiler.observe(encoded)
            assert observed == apply(observations, state)
            states += 1
            for numerator in range(spec['cueGridDenominator'] + 1):
                u = Fraction(numerator, spec['cueGridDenominator'])
                cue = (u, 1 - u)
                scores = [sum((cue[row] * state[row * outputs + output] for row in range(rows)), Fraction(0))
                          for output in range(outputs)]
                expected = max(range(outputs), key=lambda i: (scores[i], -i))
                assert winner_from_contrasts(observed, cue, outputs) == expected
                readouts += 1
            for operation, mask in enumerate(operations):
                for replacement in [0, 1, 2]:
                    supplied = tuple((1 - keep) * replacement for keep in mask)
                    changed = tuple(value * keep + new for value, keep, new in zip(state, mask, supplied))
                    assert compiler.update(encoded, operation, supplied) == compiler.encode(changed)
                    updates += 1
        assert states == spec['allInitialStates']
        rng = random.Random(spec['seed'])
        state = tuple(rng.choice(spec['weights']) for _ in range(rows * outputs))
        encoded = compiler.encode(state)
        for _ in range(spec['mutationSequenceLength']):
            operation = rng.randrange(len(operations))
            mask = operations[operation]
            supplied = tuple(rng.choice(spec['weights']) * (1 - keep) for keep in mask)
            state = tuple(value * keep + new for value, keep, new in zip(state, mask, supplied))
            encoded = compiler.update(encoded, operation, supplied)
            assert encoded == compiler.encode(state)
            assert compiler.observe(encoded) == apply(observations, state)
        results.append({'contract': contract, 'stateCoordinates': rows * outputs,
                        'encodedCoordinates': len(compiler.basis.rows), 'rankTrace': compiler.rank_trace,
                        'initialStates': states, 'mutationChecks': updates, 'cueReadouts': readouts,
                        'sequenceChecks': spec['mutationSequenceLength'],
                        'denseEncodingCoefficients': len(compiler.basis.rows) * rows * outputs,
                        'denseUpdateCoefficients': len(operations) * len(compiler.basis.rows) ** 2})
    ranks = []
    for rows, outputs in spec['scaleRankCases']:
        for contract in ['row', 'cell']:
            compiler = Compiler(contrasts(rows, outputs), masks(rows, outputs, contract))
            ranks.append({'rows': rows, 'outputs': outputs, 'contract': contract,
                          'rank': len(compiler.basis.rows), 'rankTrace': compiler.rank_trace})
    return {'checksPassed': True, 'results': results, 'ranks': ranks,
            'claim': 'Exact restricted-contract identities only; no invention, byte savings, speed or product-memory claim.'}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    here = Path(__file__).resolve().parent
    plan = json.loads((here / 'PLAN.json').read_text())
    args.output.mkdir()
    sources = {}
    for path in [here / 'PLAN.json', Path(__file__), here / 'test_probe.py']:
        content = path.read_bytes()
        (args.output / path.name).write_bytes(content)
        sources[path.name] = hashlib.sha256(content).hexdigest()
    (args.output / 'freeze.json').write_text(json.dumps({'plan': plan, 'sources': sources,
        'started': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}, indent=2) + '\n')
    result = screen(plan)
    result['freezeSha256'] = hashlib.sha256((args.output / 'freeze.json').read_bytes()).hexdigest()
    (args.output / 'finished.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
