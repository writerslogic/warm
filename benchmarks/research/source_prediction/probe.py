"""Exact source-withdrawal equivalence of empirical predictive distributions."""

from dataclasses import dataclass
from fractions import Fraction
from math import gcd


MAX_SOURCES = 32
MAX_OUTCOMES = 32
MAX_RECORDS = 4096
MAX_COUNT = 2147483647
MAX_WIDTH = 3
MAX_TERMS = 250000
MAX_PAIRS = 10000000


def integer(value, low, high, name):
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f'{name} must be an integer in {low}..{high}')
    return value


@dataclass(frozen=True)
class Profile:
    sources: int
    outcomes: int
    records: tuple

    def __post_init__(self):
        integer(self.sources, 1, MAX_SOURCES, 'sources')
        integer(self.outcomes, 1, MAX_OUTCOMES, 'outcomes')
        if not isinstance(self.records, tuple) or len(self.records) > MAX_RECORDS:
            raise ValueError('records must be a bounded tuple')
        for record in self.records:
            if not isinstance(record, tuple) or len(record) != 3:
                raise ValueError('record must be an outcome, source mask and count tuple')
            outcome, mask, count = record
            integer(outcome, 0, self.outcomes - 1, 'outcome')
            integer(mask, 0, (1 << self.sources) - 1, 'source mask')
            integer(count, 1, MAX_COUNT, 'count')
            if mask.bit_count() > MAX_WIDTH:
                raise ValueError('source dependency exceeds the width limit')

    def read(self, withdrawn):
        integer(withdrawn, 0, (1 << self.sources) - 1, 'withdrawal mask')
        values = [0] * self.outcomes
        for outcome, mask, count in self.records:
            if not mask & withdrawn:
                values[outcome] += count
        total = sum(values)
        return tuple(Fraction(value, total) for value in values) if total else None

    def polynomials(self):
        result = [{} for _ in range(self.outcomes)]
        total = {}
        for outcome, mask, count in self.records:
            result[outcome][mask] = result[outcome].get(mask, 0) + count
            total[mask] = total.get(mask, 0) + count
        return result, total

    def structural_key(self):
        numerators, _ = self.polynomials()
        divisor = 0
        for numerator in numerators:
            for count in numerator.values():
                divisor = gcd(divisor, count)
        return tuple(tuple((mask, count // divisor) for mask, count in sorted(numerator.items()))
                     for numerator in numerators)


def antichain(masks):
    minimal = []
    for mask in sorted(set(masks), key=lambda item: (item.bit_count(), item)):
        if not any(prior & mask == prior for prior in minimal):
            minimal.append(mask)
    return tuple(sorted(minimal))


def add_term(polynomial, mask, value):
    coefficient = polynomial.get(mask, 0) + value
    if coefficient:
        polynomial[mask] = coefficient
        if len(polynomial) > MAX_TERMS:
            raise ValueError('polynomial term limit exceeded')
    else:
        polynomial.pop(mask, None)


def cross_difference(left, right_total, right, left_total, meter):
    pairs = len(left) * len(right_total) + len(right) * len(left_total)
    if meter['pairs'] + pairs > MAX_PAIRS:
        raise ValueError('polynomial product budget exceeded')
    meter['pairs'] += pairs
    result = {}
    for first, second, sign in ((left, right_total, 1), (right, left_total, -1)):
        for a_mask, a_value in first.items():
            for b_mask, b_value in second.items():
                add_term(result, a_mask | b_mask, sign * a_value * b_value)
    return result


def witness(polynomial):
    if not polynomial or any(not value for value in polynomial.values()):
        raise ValueError('witness requires a nonzero canonical polynomial')
    remaining = dict(polynomial)
    withdrawn = 0
    while True:
        variables = 0
        for mask in remaining:
            variables |= mask
        if not variables:
            if not remaining.get(0):
                raise AssertionError('nonzero restriction became zero')
            return withdrawn
        bit = variables & -variables
        surviving = {}
        for mask, coefficient in remaining.items():
            add_term(surviving, mask & ~bit, coefficient)
        if surviving:
            remaining = surviving
        else:
            withdrawn |= bit
            remaining = {mask: coefficient for mask, coefficient in remaining.items() if not mask & bit}
            if not remaining:
                raise AssertionError('both Boolean restrictions are zero')


def equivalent(left, right, check_support=True):
    if (left.sources, left.outcomes) != (right.sources, right.outcomes):
        raise ValueError('profiles must share source and outcome universes')
    left_values, left_total = left.polynomials()
    right_values, right_total = right.polynomials()
    meter = {'pairs': 0}
    left_support, right_support = antichain(left_total), antichain(right_total)
    if check_support and left_support != right_support:
        for own, other in ((left_support, right_support), (right_support, left_support)):
            for mask in own:
                if not any(term & mask == term for term in other):
                    withdrawn = ((1 << left.sources) - 1) ^ mask
                    return {'equivalent': False, 'reason': 'unknown-support',
                            'withdrawn': withdrawn, 'productPairs': 0}
        raise AssertionError('different antichains define the same upward closure')
    for left_value, right_value in zip(left_values, right_values, strict=True):
        difference = cross_difference(left_value, right_total, right_value, left_total, meter)
        if difference:
            withdrawn = witness(difference)
            degree = max(mask.bit_count() for mask in difference)
            if withdrawn.bit_count() > degree:
                raise AssertionError('withdrawal witness exceeded the degree bound')
            return {'equivalent': False, 'reason': 'distribution', 'withdrawn': withdrawn,
                    'degreeBound': degree, 'productPairs': meter['pairs']}
    return {'equivalent': True, 'reason': 'polynomial-identity', 'withdrawn': None,
            'productPairs': meter['pairs']}


def multiplied_profile(sources, outcomes):
    original = tuple((source % outcomes, 1 << source, 1 << source) for source in range(sources))
    multiplied = tuple((outcome, mask | (1 << other), count)
                       for outcome, mask, count in original for other in range(sources))
    return Profile(sources, outcomes, original), Profile(sources, outcomes, multiplied)
