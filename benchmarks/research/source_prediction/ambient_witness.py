"""Exact source-count tradeoff for entirely withdrawable count profiles."""
from itertools import product
from math import prod

import probe
from matrix_witness import MATRICES, record_readout
from witness_tightness import normalized, polynomials

MAX_GROUP = 4
MAX_SOURCES = 14


def sizes(group, sources):
    probe.integer(group, 1, MAX_GROUP, 'group width')
    probe.integer(sources, group, MAX_SOURCES, 'source count')
    if sources > 3 * group:
        return (sources - 3 * group + 3,) + (3,) * (group - 1)
    triples, doubles = divmod(sources - group, 2)
    return (3,) * triples + (2,) * doubles + (1,) * (group - triples - doubles)


def factors(group, sources):
    result = []
    offset = 0
    for count in sizes(group, sources):
        if count == 1:
            matrices = (MATRICES[0],)
        elif count == 2:
            matrices = MATRICES[0], MATRICES[2]
        else:
            scale = count - 2
            endpoints = tuple(tuple(tuple(scale * value for value in row) for row in matrix)
                              for matrix in (MATRICES[0], MATRICES[2]))
            matrices = endpoints[0], *((MATRICES[1],) * scale), endpoints[1]
        result.append(tuple((1 << (offset + index), matrix) for index, matrix in enumerate(matrices)))
        offset += count
    assert offset == sources
    return tuple(result)


def family(group, sources):
    records = [[], []]
    for chosen in product(*factors(group, sources)):
        mask = sum(bit for bit, _ in chosen)
        for start in range(2):
            for outcome in range(2):
                multiplicity = 0
                for middle in product(range(2), repeat=group - 1):
                    path = start, *middle, outcome
                    multiplicity += prod(matrix[path[index]][path[index + 1]]
                                         for index, (_, matrix) in enumerate(chosen))
                records[start].append((outcome, mask, multiplicity))
    return tuple(tuple(sorted(rows)) for rows in records)


def scalar_readout(group, sources, withdrawn):
    shape = sizes(group, sources)
    probe.integer(withdrawn, 0, (1 << sources) - 1, 'withdrawal mask')
    p, q, r, s = 1, 0, 0, 1
    offset = 0
    determinant = 1
    unknown = False
    for count in shape:
        alive = [int(not (withdrawn & (1 << (offset + index)))) for index in range(count)]
        a = alive[0]
        if count == 1:
            x, y, z, w = 4 * a, 16 * a, a, 19 * a
            determinant *= 60 * a
        elif count == 2:
            c = alive[-1]
            x, y, z, w = 4 * a + 16 * c, 16 * a + 4 * c, a + 19 * c, 19 * a + c
            determinant *= 60 * (a - c)
        else:
            c, total, scale = alive[-1], sum(alive[1:-1]), count - 2
            x, y = 4 * scale * a + 10 * total + 16 * scale * c, 16 * scale * a + 10 * total + 4 * scale * c
            z, w = scale * a + 20 * total + 19 * scale * c, 19 * scale * a + 20 * total + scale * c
            determinant *= 60 * scale * (a - c) * (scale - total)
        unknown |= not any(alive)
        p, q, r, s = p * x + q * z, p * y + q * w, r * x + s * z, r * y + s * w
        offset += count
    return ((p, q), (r, s)), determinant, unknown


def expected_polynomial(group, sources):
    polynomial = {0: 1}
    offset = 0
    for count in sizes(group, sources):
        a, c = 1 << offset, 1 << (offset + count - 1)
        if count == 1:
            terms = {a: 60}
        elif count == 2:
            terms = {a: 60, c: -60}
        else:
            scale = count - 2
            terms = {a: 60 * scale**2, c: -60 * scale**2}
            for index in range(1, count - 1):
                b = 1 << (offset + index)
                terms[a | b] = -60 * scale
                terms[b | c] = 60 * scale
        polynomial = {left | right: first * second for left, first in polynomial.items()
                      for right, second in terms.items()}
        offset += count
    return polynomial


def audit(group, sources, emit=None):
    shape = sizes(group, sources)
    left, right = family(group, sources)
    for rows in (left, right):
        assert len(rows) == 2 * prod(shape)
        assert all(mask.bit_count() == group and 0 < multiplicity <= probe.MAX_COUNT
                   for _, mask, multiplicity in rows)
    left_values, left_total = polynomials(left)
    right_values, right_total = polynomials(right)
    difference = probe.cross_difference(left_values[0], right_total, right_values[0], left_total, {'pairs': 0})
    assert difference == expected_polynomial(group, sources)
    witness = probe.witness(difference)
    bound = min(2 * group, sources - group)
    first = sources + 1
    distinguishing = unknown_count = 0
    for withdrawn in range(1 << sources):
        counts = record_readout(left, withdrawn, sources), record_readout(right, withdrawn, sources)
        expected, determinant, unknown = scalar_readout(group, sources, withdrawn)
        assert counts == expected
        predictions = tuple(normalized(values) for values in counts)
        assert (predictions == (None, None)) == unknown
        if unknown:
            unknown_count += 1
        else:
            assert all(value > 0 for values in counts for value in values)
        assert counts[0][0] * counts[1][1] - counts[0][1] * counts[1][0] == determinant
        differs = predictions[0] != predictions[1]
        assert differs == (determinant != 0)
        if differs:
            first = min(first, withdrawn.bit_count())
            distinguishing += 1
        if emit is not None:
            emit({'groupWidth': group, 'sourceCount': sources, 'withdrawn': withdrawn, 'counts': counts,
                  'predictions': [None if values is None else [(x.numerator, x.denominator) for x in values]
                                  for values in predictions], 'equal': not differs})
    assert first == bound and witness.bit_count() == bound
    assert unknown_count == (1 << sources) - prod((1 << size) - 1 for size in shape)
    expected_distinguishing = prod(1 if size == 1 else 2 if size == 2 else 2 * ((1 << (size - 2)) - 1)
                                  for size in shape)
    assert distinguishing == expected_distinguishing
    witness_values, determinant, _ = scalar_readout(group, sources, witness)
    assert determinant and normalized(witness_values[0]) != normalized(witness_values[1])
    existing = None
    if group <= probe.MAX_WIDTH:
        existing = probe.equivalent(probe.Profile(sources, 2, left), probe.Profile(sources, 2, right))
        assert existing['reason'] == 'distribution' and existing['withdrawn'].bit_count() == bound
    return {'groupWidth': group, 'sourceCount': sources, 'factorSizes': shape,
            'comparisons': 1 << sources, 'minimumWithdrawals': first, 'bound': bound,
            'distinguishingMasks': distinguishing, 'jointUnknownMasks': unknown_count,
            'recordsPerProfile': len(left), 'polynomialTerms': len(difference),
            'witness': witness, 'existingChecker': existing}
