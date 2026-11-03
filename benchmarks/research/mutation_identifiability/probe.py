# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Recover a positive association table from masked winner-only queries."""

import argparse
from collections import deque
from fractions import Fraction
import hashlib
from itertools import product
import json
from math import gcd
from pathlib import Path
import platform
import random
import shutil
import time


def candidates(bound):
    if type(bound) is not int or not 1 <= bound <= 255:
        raise ValueError('integer bound must lie in 1..255')
    return sorted({Fraction(numerator, denominator) for numerator in range(1, bound + 1)
                   for denominator in range(1, bound + 1)})


def identify(grid, greater_than):
    lo, hi = 0, len(grid) - 1
    while lo < hi:
        middle = (lo + hi) // 2
        threshold = (grid[middle] + grid[middle + 1]) / 2
        if greater_than(threshold):
            lo = middle + 1
        else:
            hi = middle
    return grid[lo]


class Winner:
    def __init__(self, table, bound, log=None, case=None):
        candidates(bound)
        if not table or not table[0] or any(len(row) != len(table[0]) for row in table):
            raise ValueError('table must be nonempty and rectangular')
        if any(type(value) is not int or not 1 <= value <= bound for row in table for value in row):
            raise ValueError('table values must be positive integers within the declared bound')
        self._table = tuple(tuple(row) for row in table)
        self.rows, self.outputs = len(table), len(table[0])
        self.log, self.case, self.calls = log, case, 0

    def __call__(self, retained, cue):
        retained = tuple(retained)
        if len(set(retained)) != len(retained) or any(type(i) is not int or not 0 <= i < self.rows * self.outputs for i in retained):
            raise ValueError('invalid surviving cell identifiers')
        if len(cue) != self.rows or any(type(value) not in (int, Fraction) or value < 0 for value in cue) or not any(cue):
            raise ValueError('cue must contain nonnegative exact weights and a positive weight')
        scores = [Fraction(0)] * self.outputs
        for cell in retained:
            row, column = divmod(cell, self.outputs)
            scores[column] += cue[row] * self._table[row][column]
        result = max(range(self.outputs), key=lambda column: (scores[column], -column))
        self.calls += 1
        if self.log is not None:
            self.log.write(json.dumps({'case': self.case, 'query': self.calls,
                'retained': retained, 'cue': [str(value) for value in cue], 'winner': result}) + '\n')
        return result


def reconstruct(decide, rows, outputs, bound):
    if type(rows) is not int or type(outputs) is not int or not 2 <= rows <= 64 or not 2 <= outputs <= 64:
        raise ValueError('reconstruction requires 2..64 rows and outputs')
    grid = candidates(bound)

    def ratio(numerator, denominator):
        nr, nc = divmod(numerator, outputs)
        dr, dc = divmod(denominator, outputs)
        if nr == dr or nc == dc:
            raise ValueError('ratio comparison needs distinct rows and outputs')

        def larger(threshold):
            cue = [Fraction(0)] * rows
            cue[nr], cue[dr] = Fraction(1), threshold
            return decide((numerator, denominator), cue) == nc

        return identify(grid, larger)

    if rows == outputs == 2:
        a, d = Fraction(1), 1 / ratio(0, 3)
        b_over_c = ratio(1, 2)
        epsilon = Fraction(1, bound + 1)
        if decide((0, 1, 3), (1, epsilon)) == 0:
            gap = identify(grid, lambda threshold: decide((0, 1, 3), (1, threshold)) == 0)
            b = a - gap * d
        elif decide((0, 1, 2), (1, epsilon)) == 1:
            gap = identify(grid, lambda threshold: decide((0, 1, 2), (1, threshold)) == 1)
            b = a / (1 - gap / b_over_c)
        else:
            b = a
        return (a, b, b / b_over_c, d)

    known, queue = {0: Fraction(1)}, deque([0])
    while queue:
        parent = queue.popleft()
        row, column = divmod(parent, outputs)
        for cell in range(rows * outputs):
            child_row, child_column = divmod(cell, outputs)
            if cell not in known and child_row != row and child_column != column:
                known[cell] = known[parent] * ratio(cell, parent)
                queue.append(cell)
    if len(known) != rows * outputs:
        raise RuntimeError('comparison graph unexpectedly disconnected')
    return tuple(known[cell] for cell in range(rows * outputs))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.output.is_absolute():
        parser.error('output path must be absolute')
    here = Path(__file__).parent
    plan = json.loads((here / 'PLAN.json').read_text())
    args.output.mkdir()
    sources = [here / name for name in ('PLAN.json', 'probe.py', 'test_probe.py')]
    for source in sources:
        shutil.copyfile(source, args.output / source.name)
    freeze = {'plan': plan, 'sources': {p.name: digest(p) for p in sources},
              'python': platform.python_version(), 'started': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'claim': 'Exact finite-domain reconstruction screen, not a product or novelty claim.'}
    write(args.output / 'freeze.json', freeze)
    cells, cases, calls, failure = [], 0, 0, None
    try:
        with (args.output / 'queries.ndjson').open('x') as log, (args.output / 'cases.ndjson').open('x') as records:
            for spec in [*plan['exhaustiveCases'], plan['largerCases']]:
                rows, outputs, bound = (spec[name] for name in ('rows', 'outputs', 'maxWeight'))
                if 'seeds' in spec:
                    values = (tuple(random.Random(seed).choices(range(1, bound + 1), k=rows * outputs)) for seed in spec['seeds'])
                else:
                    values = product(range(1, bound + 1), repeat=rows * outputs)
                count, classes, query_count = 0, set(), 0
                for flat in values:
                    table = tuple(tuple(flat[i:i + outputs]) for i in range(0, len(flat), outputs))
                    oracle = Winner(table, bound, log, cases)
                    recovered = reconstruct(oracle.__call__, rows, outputs, bound)
                    expected = tuple(Fraction(value, flat[0]) for value in flat)
                    records.write(json.dumps({'case': cases, 'rows': rows, 'outputs': outputs,
                        'bound': bound, 'input': flat, 'reconstructed': [str(value) for value in recovered],
                        'queries': oracle.calls, 'pass': recovered == expected}) + '\n')
                    if recovered != expected:
                        raise AssertionError(f'reconstruction mismatch in case {cases}')
                    common = gcd(*flat)
                    classes.add(tuple(value // common for value in flat))
                    count += 1
                    cases += 1
                    calls += oracle.calls
                    query_count += oracle.calls
                cells.append({'rows': rows, 'outputs': outputs, 'bound': bound, 'cases': count,
                              'projectiveClasses': len(classes), 'winnerQueries': query_count,
                              'exhaustive': 'seeds' not in spec, 'pass': True})
                print(json.dumps(cells[-1]), flush=True)
        if {p.name: digest(p) for p in sources} != freeze['sources']:
            raise RuntimeError('probe source changed during the experiment')
    except Exception as error:
        failure = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        write(args.output / 'finished.json', {'complete': failure is None, 'failure': failure,
            'cases': cases, 'winnerQueries': calls, 'cells': cells,
            'freezeSha256': digest(args.output / 'freeze.json'),
            'evidenceSha256': {name: digest(args.output / name) for name in ('queries.ndjson', 'cases.ndjson')
                               if (args.output / name).exists()}})


if __name__ == '__main__':
    main()
