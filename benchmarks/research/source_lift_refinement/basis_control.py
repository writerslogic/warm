# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""A stronger exact union control that visits only withdrawals hitting a basis."""

from math import comb

from refine import refine
from source_lift.probe import Basis, provenance_basis, reference_space, validate


def proof_union(records, width, prime, budget, fanin, minimize=False):
    validate(records, width, prime, budget, fanin)
    compressed = provenance_basis(records, prime)
    rank = len(reference_space([row.values for row in compressed], width, prime))
    sources = {source for row in compressed for source in row.sources}
    statistics = {'basisBuilds': 0, 'evidenceRowsExamined': 0,
                  'allPossibleWithdrawalSets': sum(comb(len(sources), size) for size in range(budget + 1))}
    visited, retained = set(), {}

    def visit(deleted):
        if deleted in visited:
            return
        visited.add(deleted)
        statistics['basisBuilds'] += 1
        basis, chosen = Basis(prime), []
        for row in compressed:
            statistics['evidenceRowsExamined'] += 1
            if not deleted.intersection(row.sources) and basis.add(row.values):
                chosen.append(row)
                retained[row.ordinal] = row
                if len(basis.rows) == rank:
                    break
        if len(deleted) < budget:
            for source in sorted({source for row in chosen for source in row.sources}):
                visit(deleted | {source})

    visit(frozenset())
    result = sorted(retained.values(), key=lambda row: row.ordinal)
    details = {'basisOracle': statistics}
    if minimize:
        kernel_size = len(result)
        result, refinement = refine(result, width, prime, budget, fanin)
        details.update({'kernelRows': kernel_size, **refinement})
    return result, details
