"""Exact singleton-source refinement for larger sparse empirical models."""

from collections import Counter, defaultdict
from math import gcd
from functools import reduce


def canonical(values):
    groups = {}
    return tuple(groups.setdefault(value, len(groups)) for value in values)


def normalized(counts):
    scale = reduce(gcd, counts.values())
    return tuple((target, count // scale) for target, count in sorted(counts.items()))


def signature(records, partition, current=False):
    by_action = defaultdict(lambda: defaultdict(Counter))
    for action, target, source, count in records:
        by_action[action][source][partition[target]] += count
    result = []
    for action, sources in sorted(by_action.items()):
        if current:
            total = Counter()
            for counts in sources.values():
                total.update(counts)
            value = ('current', normalized(total))
        else:
            distributions = [normalized(counts) for _, counts in sorted(sources.items())]
            if all(distribution == distributions[0] for distribution in distributions):
                value = ('constant', tuple(sorted(sources)), distributions[0])
            else:
                raw = {(source, target): count for source, counts in sources.items() for target, count in counts.items()}
                value = ('mixed', normalized(raw))
        result.append((action, value))
    return tuple(result)


def validate(states, actions, sources, records):
    for value, limit in ((states, 50000), (actions, 20000), (sources, 128)):
        if type(value) is not int or not 1 <= value <= limit:
            raise ValueError('model dimension outside bound')
    if not isinstance(records, (list, tuple)) or len(records) > 200000:
        raise ValueError('record count outside bound')
    for row in records:
        if not isinstance(row, (list, tuple)) or len(row) != 5:
            raise ValueError('record needs state, action, target, source and count')
        for value, limit in zip(row, (states - 1, actions - 1, states - 1, sources - 1, 2147483647), strict=True):
            if type(value) is not int or not 0 <= value <= limit:
                raise ValueError('record field outside bound')
        if row[-1] == 0:
            raise ValueError('record count must be positive')


def minimize(states, actions, sources, records, current=False):
    validate(states, actions, sources, records)
    grouped = [[] for _ in range(states)]
    for state, action, target, source, count in records:
        grouped[state].append((action, target, source, count))
    partition = (0,) * states
    for iteration in range(1, min(states, 1000) + 1):
        refined = canonical((partition[state], signature(grouped[state], partition, current)) for state in range(states))
        if refined == partition:
            return {'partition': partition, 'iterations': iteration}
        if len(set(refined)) <= len(set(partition)):
            raise AssertionError('refinement failed to split a block')
        partition = refined
    raise ValueError('refinement iteration budget exhausted')


def quotient(states, actions, sources, records, partition):
    validate(states, actions, sources, records)
    if len(partition) != states or canonical(partition) != tuple(partition):
        raise ValueError('partition is not canonical')
    representatives, grouped = {}, [[] for _ in range(states)]
    for state, action, target, source, count in records:
        grouped[state].append((action, target, source, count))
    for state, block in enumerate(partition):
        representative = representatives.setdefault(block, state)
        if signature(grouped[state], partition) != signature(grouped[representative], partition):
            raise ValueError('partition is not withdrawal preserving')
    selected = set(representatives.values())
    compact = Counter()
    for state, action, target, source, count in records:
        if state in selected:
            compact[(partition[state], action, partition[target], source)] += count
    return [list(row) + [count] for row, count in sorted(compact.items())]


def withdraw(records, removed):
    return [row for row in records if row[3] not in removed]
