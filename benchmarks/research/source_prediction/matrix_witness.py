"""Strictly positive, entirely withdrawable evidence attaining the 2g bound."""
from fractions import Fraction
from itertools import product
from math import prod

import probe
from witness_tightness import normalized, polynomials

MAX_GROUP = 5
MATRICES = (((4, 16), (1, 19)), ((10, 10), (20, 20)), ((16, 4), (19, 1)))


def record_readout(records, withdrawn, sources):
    probe.integer(sources, 1, 3 * MAX_GROUP, 'sources')
    probe.integer(withdrawn, 0, (1 << sources) - 1, 'withdrawal mask')
    values = [0, 0]
    for outcome, mask, multiplicity in records:
        if mask & withdrawn == 0:
            values[outcome] += multiplicity
    return tuple(values)


def family(group):
    probe.integer(group, 1, MAX_GROUP, 'group width')
    records = [[], []]
    for choices in product(range(3), repeat=group):
        mask = sum(1 << (3 * index + choice) for index, choice in enumerate(choices))
        for start in range(2):
            for outcome in range(2):
                multiplicity = 0
                for middle in product(range(2), repeat=group - 1):
                    path = (start, *middle, outcome)
                    multiplicity += prod(MATRICES[choice][path[index]][path[index + 1]]
                                         for index, choice in enumerate(choices))
                records[start].append((outcome, mask, multiplicity))
    return 3 * group, tuple(tuple(sorted(rows)) for rows in records)


def matrix_readout(group, withdrawn):
    probe.integer(group, 1, MAX_GROUP, 'group width')
    probe.integer(withdrawn, 0, (1 << (3 * group)) - 1, 'withdrawal mask')
    p, q, r, s = 1, 0, 0, 1
    for index in range(group):
        a, b, c = (int(not (withdrawn & (1 << (3 * index + choice)))) for choice in range(3))
        left, top = 4 * a + 10 * b + 16 * c, 16 * a + 10 * b + 4 * c
        bottom, right = a + 20 * b + 19 * c, 19 * a + 20 * b + c
        p, q, r, s = p * left + q * bottom, p * top + q * right, r * left + s * bottom, r * top + s * right
    return (p, q), (r, s)


def expected_polynomial(group):
    polynomial = {0: 1}
    for index in range(group):
        a, b, c = (1 << (3 * index + choice) for choice in range(3))
        terms = {a: 60, c: -60, a | b: -60, b | c: 60}
        polynomial = {left | right: first * second for left, first in polynomial.items()
                      for right, second in terms.items()}
    return polynomial


def audit(group, emit=None):
    sources, (left, right) = family(group)
    for rows in (left, right):
        assert len(rows) == 2 * 3 ** group
        assert all(mask != 0 and mask.bit_count() == group and 0 < multiplicity <= probe.MAX_COUNT
                   for _, mask, multiplicity in rows)
    left_values, left_total = polynomials(left)
    right_values, right_total = polynomials(right)
    difference = probe.cross_difference(left_values[0], right_total, right_values[0], left_total, {'pairs': 0})
    assert difference == expected_polynomial(group)
    witness = probe.witness(difference)
    assert witness.bit_count() == 2 * group
    distinguishing, unknown = [], 0
    for withdrawn in range(1 << sources):
        values = record_readout(left, withdrawn, sources), record_readout(right, withdrawn, sources)
        assert values == matrix_readout(group, withdrawn)
        predictions = tuple(normalized(value) for value in values)
        groups = [(withdrawn >> (3 * index)) & 7 for index in range(group)]
        joint_unknown = 7 in groups
        differs = all(mask in (3, 6) for mask in groups)
        assert (predictions == (None, None)) == joint_unknown
        assert (predictions[0] != predictions[1]) == differs
        if joint_unknown:
            unknown += 1
        else:
            assert all(value > 0 for pair in values for value in pair)
        expected_determinant = prod(60 * (int(not (mask & 1)) - int(not (mask & 4))) * int(bool(mask & 2))
                                    for mask in groups)
        assert values[0][0] * values[1][1] - values[0][1] * values[1][0] == expected_determinant
        if differs:
            distinguishing.append(withdrawn)
            assert withdrawn.bit_count() == 2 * group
        if emit is not None:
            emit({'groupWidth': group, 'withdrawn': withdrawn, 'counts': values,
                  'predictions': [None if prediction is None else
                                 [(value.numerator, value.denominator) for value in prediction]
                                 for prediction in predictions], 'equal': not differs})
    assert len(distinguishing) == 2 ** group and witness in distinguishing
    assert unknown == 8 ** group - 7 ** group
    existing = None
    if group <= probe.MAX_WIDTH:
        existing = probe.equivalent(probe.Profile(sources, 2, left), probe.Profile(sources, 2, right))
        assert existing['reason'] == 'distribution' and existing['withdrawn'] in distinguishing
    probability = Fraction(len(distinguishing), 1 << sources)
    assert probability == Fraction(1, 1 << (2 * group))
    return {'groupWidth': group, 'sourceCount': sources, 'exactSupportWidth': group,
            'unconditionalRecords': 0, 'records': [len(left), len(right)],
            'comparisons': 1 << sources, 'unknownMasks': unknown,
            'distinguishingMasks': distinguishing, 'minimumWithdrawals': 2 * group,
            'uniformDetectionProbability': [probability.numerator, probability.denominator],
            'polynomialTerms': len(difference), 'witness': witness, 'existingChecker': existing}
