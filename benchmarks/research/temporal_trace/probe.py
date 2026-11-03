# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Bounded withdrawal certificates for deterministic temporal read programs."""

from dataclasses import asdict, dataclass
from itertools import combinations
import json


@dataclass(frozen=True)
class Record:
    ordinal: int
    slot: int
    value: int
    valid_from: int
    valid_to: int | None
    recorded_at: int
    sources: tuple
    text: str


def integer(value, lower, upper):
    return type(value) is int and lower <= value <= upper


def expression_slots(expression, depth=0, count=None):
    if count is None:
        count = [0]
    count[0] += 1
    if count[0] > 256 or depth > 16 or not isinstance(expression, tuple) or not expression:
        raise ValueError('invalid or oversized expression')
    operator = expression[0]
    if not isinstance(operator, str):
        raise ValueError('invalid expression operator')
    if operator == 'eq' and len(expression) == 3:
        if not integer(expression[1], 0, 255) or not integer(expression[2], 0, 2):
            raise ValueError('invalid slot comparison')
        return {expression[1]}
    arity = {'not': 1, 'and': 2, 'or': 2, 'if': 3}.get(operator)
    if arity is None or len(expression) != arity + 1:
        raise ValueError('invalid expression operator')
    result = set()
    for child in expression[1:]:
        result.update(expression_slots(child, depth + 1, count))
    return result


def validate(records, programs, snapshots, budget):
    if not integer(budget, 0, 3) or len(records) > 4096:
        raise ValueError('invalid budget or record count')
    if not 1 <= len(programs) <= 32 or not 1 <= len(snapshots) <= 32:
        raise ValueError('invalid program or snapshot count')
    for program in programs:
        expression_slots(program)
    for snapshot in snapshots:
        if not isinstance(snapshot, tuple) or len(snapshot) != 2 or any(
                not integer(value, -10**9, 10**9) for value in snapshot):
            raise ValueError('invalid temporal snapshot')
    seen, sources = set(), set()
    for row in records:
        if not isinstance(row, Record) or not integer(row.ordinal, 0, 2**32 - 1) or row.ordinal in seen:
            raise ValueError('invalid or duplicate record identity')
        seen.add(row.ordinal)
        if not integer(row.slot, 0, 255) or not integer(row.value, 0, 2):
            raise ValueError('invalid record slot or value')
        if any(not integer(value, -10**9, 10**9) for value in (row.valid_from, row.recorded_at)):
            raise ValueError('invalid record time')
        if row.valid_to is not None and (not integer(row.valid_to, -10**9, 10**9) or row.valid_to <= row.valid_from):
            raise ValueError('invalid half-open interval')
        if not isinstance(row.sources, tuple) or not 1 <= len(row.sources) <= 2 or any(
                not integer(source, 1, 65535) for source in row.sources):
            raise ValueError('invalid provenance')
        if tuple(sorted(set(row.sources))) != row.sources:
            raise ValueError('provenance must be sorted and unique')
        sources.update(row.sources)
        if not isinstance(row.text, str) or len(row.text.encode('utf-8')) > 4096:
            raise ValueError('invalid evidence text')
    if len(sources) > 256:
        raise ValueError('too many source identities')


def withdrawals(records, budget):
    sources = sorted({source for row in records for source in row.sources})
    for size in range(budget + 1):
        for values in combinations(sources, size):
            yield frozenset(values)


class Index:
    def __init__(self, records):
        self.slots = {}
        for row in sorted(records, key=lambda item: (item.valid_from, item.recorded_at, item.ordinal), reverse=True):
            self.slots.setdefault(row.slot, []).append(row)

    def head(self, slot, snapshot, deleted):
        valid, known = snapshot
        for row in self.slots.get(slot, ()):
            if (row.valid_from <= valid and (row.valid_to is None or valid < row.valid_to)
                    and row.recorded_at <= known and not deleted.intersection(row.sources)):
                return row
        return None


def execute(index, programs, snapshot, deleted):
    trace, heads = [], {}

    def evaluate(expression):
        operator = expression[0]
        if operator == 'eq':
            row = index.head(expression[1], snapshot, deleted)
            trace.append((expression[1], None if row is None else row.ordinal))
            if row is not None:
                heads[row.ordinal] = row
            return row is not None and row.value == expression[2]
        if operator == 'not':
            return not evaluate(expression[1])
        if operator == 'and':
            return evaluate(expression[1]) and evaluate(expression[2])
        if operator == 'or':
            return evaluate(expression[1]) or evaluate(expression[2])
        if operator == 'if':
            return evaluate(expression[2] if evaluate(expression[1]) else expression[3])
        raise ValueError('unvalidated expression')

    answers = tuple(evaluate(program) for program in programs)
    return (answers, tuple(trace)), list(heads.values())


def reference(records, programs, snapshot, deleted):
    valid, known = snapshot
    heads = {}
    for row in records:
        if row.recorded_at > known or row.valid_from > valid:
            continue
        if row.valid_to is not None and row.valid_to <= valid:
            continue
        if any(source in deleted for source in row.sources):
            continue
        prior = heads.get(row.slot)
        if prior is None or (row.valid_from, row.recorded_at, row.ordinal) > (
                prior.valid_from, prior.recorded_at, prior.ordinal):
            heads[row.slot] = row
    trace = []

    def read(expression):
        row = heads.get(expression[1])
        trace.append((expression[1], row.ordinal if row else None))
        return bool(row and row.value == expression[2])

    def evaluate(expression):
        operator, *arguments = expression
        if operator == 'eq':
            return read(expression)
        if operator == 'not':
            return not evaluate(arguments[0])
        if operator in ('and', 'or'):
            return (all if operator == 'and' else any)(evaluate(child) for child in arguments)
        branch = 1 if evaluate(arguments[0]) else 2
        return evaluate(arguments[branch])

    answers = tuple(map(evaluate, programs))
    return answers, tuple(trace)


def compile_certificate(records, programs, snapshots, budget, method='trace'):
    validate(records, programs, snapshots, budget)
    if method not in ('trace', 'heads', 'positive-only'):
        raise ValueError('unknown compiler')
    index, retained = Index(records), {}
    slots = sorted(set().union(*(expression_slots(program) for program in programs)))
    nodes = 0

    def collect(snapshot, deleted, visited, slot):
        nonlocal nodes
        if deleted in visited:
            return
        visited.add(deleted)
        nodes += 1
        if nodes > 100000:
            raise ValueError('oracle node bound exceeded')
        if method == 'heads':
            row = index.head(slot, snapshot, deleted)
            heads = [] if row is None else [row]
        else:
            _, heads = execute(index, programs, snapshot, deleted)
            if method == 'positive-only':
                heads = [row for row in heads if row.value == 1]
        retained.update((row.ordinal, row) for row in heads)
        if len(deleted) < budget:
            for source in sorted({source for row in heads for source in row.sources}):
                collect(snapshot, deleted | {source}, visited, slot)

    for snapshot in snapshots:
        for slot in slots if method == 'heads' else [None]:
            collect(snapshot, frozenset(), set(), slot)
    return sorted(retained.values(), key=lambda row: row.ordinal), {'oracleCalls': nodes}


def packet(records, programs, snapshots, budget):
    validate(records, programs, snapshots, budget)
    payload = {'schema': 'warm.temporal-trace/1', 'policy': 'latest-eligible',
               'records': [asdict(row) for row in sorted(records, key=lambda row: row.ordinal)],
               'programs': programs, 'snapshots': snapshots, 'budget': budget}
    return json.dumps(payload, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode('utf-8')


class Certificate:
    def __init__(self, records, programs, snapshots, budget):
        validate(records, programs, snapshots, budget)
        self.index, self.programs = Index(records), tuple(programs)
        self.snapshots, self.budget = frozenset(snapshots), budget

    def evaluate(self, snapshot, deleted):
        if not isinstance(snapshot, tuple) or len(snapshot) != 2 or any(
                not integer(value, -10**9, 10**9) for value in snapshot) or snapshot not in self.snapshots:
            raise ValueError('snapshot outside the certified contract')
        if not isinstance(deleted, frozenset) or len(deleted) > self.budget or any(
                not integer(source, 1, 65535) for source in deleted):
            raise ValueError('withdrawal outside the certified contract')
        return execute(self.index, self.programs, snapshot, deleted)[0]


def verify(records, programs, snapshots, budget, selected):
    original = {row.ordinal: row for row in records}
    if any(original.get(row.ordinal) != row for row in selected):
        raise AssertionError('certificate changed an original record')
    certificate, union, states = Certificate(selected, programs, snapshots, budget), set(), 0
    for deleted in withdrawals(records, budget):
        for snapshot in snapshots:
            expected = reference(records, programs, snapshot, deleted)
            actual = certificate.evaluate(snapshot, deleted)
            if actual != expected:
                raise AssertionError(f'counterfactual changed: {sorted(deleted)}, {snapshot}, {expected}, {actual}')
            union.update(ordinal for _, ordinal in expected[1] if ordinal is not None)
            states += 1
    return states, sorted(union)
