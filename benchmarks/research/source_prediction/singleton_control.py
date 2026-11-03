"""A complete direct control when each observation has exactly one source."""

from fractions import Fraction

from probe import antichain


def signature(profile):
    numerators, total = profile.polynomials()
    if any(mask.bit_count() != 1 for mask in total):
        raise ValueError('singleton control requires exactly one source per observation')
    if not total:
        return 'unknown', profile.outcomes
    support = tuple(sorted(total))
    distributions = [tuple(Fraction(numerator.get(mask, 0), total[mask]) for numerator in numerators)
                     for mask in support]
    if all(value == distributions[0] for value in distributions):
        return 'constant', profile.outcomes, support, distributions[0]
    return 'mixed', profile.outcomes, profile.structural_key()


def equivalent(left, right):
    if (left.sources, left.outcomes) != (right.sources, right.outcomes):
        raise ValueError('profiles must share source and outcome universes')
    return signature(left) == signature(right)


def sufficient_signature(profile):
    if all(mask.bit_count() == 1 for _, mask, _ in profile.records):
        return signature(profile)
    numerators, total = profile.polynomials()
    mask = next(iter(total))
    values = tuple(Fraction(numerator.get(mask, 0), total[mask]) for numerator in numerators)
    if all(all(numerator.get(term, 0) * value.denominator == count * value.numerator
               for term, count in total.items()) for numerator, value in zip(numerators, values, strict=True)):
        return 'constant', profile.outcomes, antichain(total), values
    return 'joint', profile.outcomes, profile.structural_key()
