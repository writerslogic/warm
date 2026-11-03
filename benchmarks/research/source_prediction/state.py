"""Uniform predictive-state quotients over every source-withdrawal assignment."""

from dataclasses import dataclass

from probe import MAX_PAIRS, MAX_RECORDS, MAX_SOURCES, Profile, equivalent, integer
from symbolic_control import Diagrams
from singleton_control import sufficient_signature


def canonical(values):
    labels = {}
    return tuple(labels.setdefault(value, len(labels)) for value in values)


@dataclass(frozen=True)
class Model:
    sources: int
    actions: int
    labels: tuple
    records: tuple

    def __post_init__(self):
        integer(self.sources, 1, MAX_SOURCES, 'sources')
        integer(self.actions, 1, 8, 'actions')
        if not isinstance(self.labels, tuple) or not 1 <= len(self.labels) <= 32:
            raise ValueError('model needs 1..32 state labels')
        for label in self.labels:
            integer(label, 0, 31, 'state label')
        if not isinstance(self.records, tuple) or len(self.records) > MAX_RECORDS:
            raise ValueError('model records must be a bounded tuple')
        observations = []
        for record in self.records:
            if not isinstance(record, tuple) or len(record) != 5:
                raise ValueError('record must contain state, action, target, source mask and count')
            state, action, target, mask, count = record
            integer(state, 0, len(self.labels) - 1, 'state')
            integer(action, 0, self.actions - 1, 'action')
            observations.append((target, mask, count))
        Profile(self.sources, len(self.labels), tuple(observations))

    def profiles(self, partition):
        if not isinstance(partition, tuple) or len(partition) != len(self.labels):
            raise ValueError('partition must cover every model state')
        for block in partition:
            integer(block, 0, len(self.labels) - 1, 'partition block')
        if canonical(partition) != partition:
            raise ValueError('partition must have canonical contiguous block identities')
        grouped = [[[] for _ in range(self.actions)] for _ in self.labels]
        for state, action, target, mask, count in self.records:
            grouped[state][action].append((partition[target], mask, count))
        blocks = max(partition) + 1
        return tuple(tuple(Profile(self.sources, blocks, tuple(row)) for row in state) for state in grouped)

    def withdraw(self, removed):
        integer(removed, 0, (1 << self.sources) - 1, 'withdrawal mask')
        return Model(self.sources, self.actions, self.labels,
                     tuple(record for record in self.records if not record[3] & removed))


def minimize(model, method='polynomial', descending=False):
    if method not in ('polynomial', 'diagram', 'exhaustive', 'structural', 'source-normalized', 'current'):
        raise ValueError('unknown state-equivalence method')
    if method == 'exhaustive' and model.sources > 8:
        raise ValueError('exhaustive control is limited to eight sources')
    partition = canonical(model.labels)
    comparisons = product_pairs = 0
    diagram_stats = []
    for iteration in range(1, len(model.labels) + 1):
        profiles = model.profiles(partition)
        diagrams = Diagrams(descending) if method == 'diagram' else None
        cache = {}

        def key(profile):
            structural = profile.structural_key()
            if structural not in cache:
                if method == 'diagram':
                    cache[structural] = diagrams.profile(profile)
                elif method == 'exhaustive':
                    cache[structural] = tuple(profile.read(mask) for mask in range(1 << model.sources))
                elif method == 'current':
                    cache[structural] = profile.read(0)
                elif method == 'source-normalized':
                    cache[structural] = sufficient_signature(profile)
                else:
                    cache[structural] = structural
            return cache[structural]

        def compatible(left, right):
            nonlocal comparisons, product_pairs
            for a, b in zip(profiles[left], profiles[right], strict=True):
                comparisons += 1
                if comparisons > 100000:
                    raise ValueError('state-refinement comparison budget exceeded')
                if method == 'polynomial':
                    result = equivalent(a, b)
                    product_pairs += result['productPairs']
                    if product_pairs > MAX_PAIRS:
                        raise ValueError('state-refinement polynomial budget exceeded')
                    same = result['equivalent']
                else:
                    same = key(a) == key(b)
                if not same:
                    return False
            return True

        groups, refined = [], []
        for state in range(len(model.labels)):
            for block, representative in enumerate(groups):
                if partition[state] == partition[representative] and compatible(state, representative):
                    refined.append(block)
                    break
            else:
                refined.append(len(groups))
                groups.append(state)
        refined = tuple(refined)
        if diagrams is not None:
            diagram_stats.append(diagrams.stats())
        if refined == partition:
            return {'partition': partition, 'iterations': iteration, 'comparisons': comparisons,
                    'productPairs': product_pairs, 'diagramStats': diagram_stats}
        if len(set(refined)) <= len(set(partition)):
            raise AssertionError('nontrivial partition refinement did not split a block')
        partition = refined
    raise AssertionError('partition refinement failed to terminate')


def quotient(model, partition):
    profiles = model.profiles(partition)
    representatives = {}
    for state, block in enumerate(partition):
        representative = representatives.setdefault(block, state)
        if model.labels[state] != model.labels[representative]:
            raise ValueError('quotient merges distinct observable state labels')
        if state != representative:
            for left, right in zip(profiles[state], profiles[representative], strict=True):
                if not equivalent(left, right)['equivalent']:
                    raise ValueError('quotient is not valid under every source withdrawal')
    selected = set(representatives.values())
    return Model(model.sources, model.actions,
                 tuple(model.labels[representatives[block]] for block in range(len(representatives))),
                 tuple((partition[state], action, partition[target], mask, count)
                       for state, action, target, mask, count in model.records if state in selected))
