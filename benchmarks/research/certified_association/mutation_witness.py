# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Exact-arithmetic witness that readout equivalence need not survive deletion."""

from fractions import Fraction
import json


def contrasts(matrix):
    return tuple(row[0] - row[1] for row in matrix)


def scores(matrix, cue):
    return tuple(sum(weight * row[k] for weight, row in zip(cue, matrix)) for k in range(2))


def winner(values):
    return max(range(len(values)), key=values.__getitem__)


def main():
    first = ((8, 8), (4, 0))
    second = ((1, 1), (4, 0))
    assert contrasts(first) == contrasts(second) == (0, 4)
    for numerator in range(101):
        cue = (Fraction(numerator, 100), Fraction(100 - numerator, 100))
        assert winner(scores(first, cue)) == winner(scores(second, cue))
    removed_first = ((0, 8), first[1])
    removed_second = ((0, 1), second[1])
    cue = (Fraction(1, 2), Fraction(1, 2))
    before = [scores(matrix, cue) for matrix in (first, second)]
    after = [scores(matrix, cue) for matrix in (removed_first, removed_second)]
    assert [winner(values) for values in before] == [0, 0]
    assert [winner(values) for values in after] == [1, 0]
    print(json.dumps({
        'complete': True, 'first': first, 'second': second,
        'sharedContrastCoefficients': contrasts(first),
        'allCueProof': 'Equal coefficients imply identical output-score differences for every cue, not only the checked grid.',
        'operation': 'Set the contribution at input row 0, output 0 to zero; command carries no old weight.',
        'cue': list(map(str, cue)), 'beforeScores': [[str(x) for x in row] for row in before],
        'afterScores': [[str(x) for x in row] for row in after],
        'afterWinners': [winner(values) for values in after],
        'scope': 'An analytical counterexample for contrast-only encoding, not a flaw in the prototype that retains raw input or a novelty claim.',
    }, indent=2))


if __name__ == '__main__':
    main()
