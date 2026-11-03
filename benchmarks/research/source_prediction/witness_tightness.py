"""Positive-count witnesses for the source-prediction deletion bound."""
from collections import defaultdict
from fractions import Fraction

import probe

MAX_GROUP = 6


def family(group, anchored=False):
    probe.integer(group, 1, MAX_GROUP, 'group width')
    if type(anchored) is not bool:
        raise ValueError('anchored must be a boolean')
    even_x, odd_x, even_y, odd_y = [], [], [], []
    for mask in range(1 << group):
        (odd_x if mask.bit_count() % 2 else even_x).append(mask)
        (odd_y if mask.bit_count() % 2 else even_y).append(mask << group)
    anchor = (1 << (2 * group)) if anchored else 0
    profiles = []
    for raw in ((even_x + odd_y, odd_x + odd_y), (even_x + even_y, odd_x + even_y)):
        records = defaultdict(int)
        for old_outcome, masks in enumerate(raw):
            for mask in masks:
                for outcome in range(2):
                    records[outcome, mask | anchor] += 2 if outcome == old_outcome else 1
        profiles.append(tuple((outcome, mask, count) for (outcome, mask), count in sorted(records.items())))
    return 2 * group + int(anchored), tuple(profiles)


def count(records, withdrawn, sources):
    probe.integer(sources, 1, 2 * MAX_GROUP + 1, 'sources')
    probe.integer(withdrawn, 0, (1 << sources) - 1, 'withdrawal mask')
    values = [0, 0]
    for outcome, mask, multiplicity in records:
        if mask & withdrawn == 0:
            values[outcome] += multiplicity
    return tuple(values)


def normalized(values):
    total = sum(values)
    return tuple(Fraction(value, total) for value in values) if total else None


def closed_form(group, withdrawn, anchored=False):
    sources = 2 * group + int(anchored)
    probe.integer(withdrawn, 0, (1 << sources) - 1, 'withdrawal mask')
    if anchored and withdrawn & (1 << (2 * group)):
        return (0, 0), (0, 0)
    low = (1 << group) - 1
    active_x = group - (withdrawn & low).bit_count()
    active_y = group - ((withdrawn >> group) & low).bit_count()
    p, q = (1 << (active_x - 1),) * 2 if active_x else (1, 0)
    s, r = (1 << (active_y - 1),) * 2 if active_y else (1, 0)
    a, b, c, d = p + r, q + r, p + s, q + s
    return (2 * a + b, a + 2 * b), (2 * c + d, c + 2 * d)


def polynomials(records):
    values, total = ({}, {}), {}
    for outcome, mask, multiplicity in records:
        probe.add_term(values[outcome], mask, multiplicity)
        probe.add_term(total, mask, multiplicity)
    return values, total


def audit(group, anchored=False, emit=None):
    sources, (left, right) = family(group, anchored)
    width = group + int(anchored)
    assert max(mask.bit_count() for records in (left, right) for _, mask, _ in records) == width
    assert all(multiplicity > 0 and (not anchored or mask != 0)
               for records in (left, right) for _, mask, multiplicity in records)
    left_values, left_total = polynomials(left)
    right_values, right_total = polynomials(right)
    difference = probe.cross_difference(left_values[0], right_total, right_values[0], left_total, {'pairs': 0})
    anchor = (1 << (2 * group)) if anchored else 0
    expected = {mask | anchor: 3 * (-1 if mask.bit_count() % 2 else 1) for mask in range(1 << (2 * group))}
    assert difference == expected
    expected_witness = (1 << (2 * group)) - 1
    assert probe.witness(difference) == expected_witness
    differing, unknown, comparisons = [], 0, 0
    for withdrawn in range(1 << sources):
        values = count(left, withdrawn, sources), count(right, withdrawn, sources)
        assert values == closed_form(group, withdrawn, anchored)
        predictions = tuple(normalized(value) for value in values)
        if anchored and withdrawn & anchor:
            assert predictions == (None, None)
            unknown += 1
        else:
            assert all(value > 0 for pair in values for value in pair)
            assert (values[0][0] * values[1][1] - values[1][0] * values[0][1]) == (3 if withdrawn == expected_witness else 0)
        if predictions[0] != predictions[1]:
            differing.append(withdrawn)
        if emit is not None:
            emit({'groupWidth': group, 'anchored': anchored, 'withdrawn': withdrawn,
                  'counts': values, 'predictions': [None if prediction is None else
                   [(value.numerator, value.denominator) for value in prediction] for prediction in predictions],
                  'equal': predictions[0] == predictions[1]})
        comparisons += 1
    assert differing == [expected_witness]
    assert normalized(count(left, expected_witness, sources)) == (Fraction(2, 3), Fraction(1, 3))
    assert normalized(count(right, expected_witness, sources)) == (Fraction(5, 9), Fraction(4, 9))
    existing = None
    if width <= probe.MAX_WIDTH:
        existing = probe.equivalent(probe.Profile(sources, 2, left), probe.Profile(sources, 2, right))
        assert existing['reason'] == 'distribution' and existing['withdrawn'] == expected_witness
    return {'groupWidth': group, 'sourceCount': sources, 'maximumSupportWidth': width,
            'anchored': anchored, 'records': [len(left), len(right)], 'comparisons': comparisons,
            'unknownMasks': unknown, 'differingMasks': differing, 'minimumWithdrawals': 2 * group,
            'polynomialTerms': len(difference), 'existingChecker': existing}
