# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Prune a source certificate using proofs and exact withdrawal counterexamples."""

from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from source_lift.probe import (Basis, deletion_sets, lift_certificate,
                               provenance_basis, reference_space, surviving, validate)


def linear_proof(records, target, prime):
    basis = {}
    for record in records:
        row, coefficients = list(record.values), {record.ordinal: 1}
        for pivot, (base, provenance) in sorted(basis.items()):
            scale = row[pivot]
            if scale:
                row = [(value - scale * other) % prime for value, other in zip(row, base)]
                for identifier, weight in provenance.items():
                    coefficients[identifier] = (coefficients.get(identifier, 0) - scale * weight) % prime
                coefficients = {identifier: weight for identifier, weight in coefficients.items() if weight}
        pivot = next((i for i, value in enumerate(row) if value), None)
        if pivot is not None:
            inverse = pow(row[pivot], -1, prime)
            basis[pivot] = ([value * inverse % prime for value in row],
                            {identifier: weight * inverse % prime for identifier, weight in coefficients.items()})
        remainder, proof = list(target), {}
        for pivot, (base, provenance) in sorted(basis.items()):
            scale = remainder[pivot]
            if scale:
                remainder = [(value - scale * other) % prime for value, other in zip(remainder, base)]
                for identifier, weight in provenance.items():
                    proof[identifier] = (proof.get(identifier, 0) + scale * weight) % prime
        if not any(remainder):
            return {identifier: weight for identifier, weight in proof.items() if weight}
    return {} if not any(target) else None


def separating_withdrawal(records, target, prime, budget, statistics):
    protected = frozenset(target.sources)
    ordered = sorted(records, key=lambda row: (len(set(row.sources) - protected), row.ordinal))
    identities = {row.ordinal: row for row in ordered}
    visited = set()

    def search(deleted):
        if deleted in visited:
            return None
        visited.add(deleted)
        statistics['proofSearchStates'] += 1
        proof = linear_proof((row for row in ordered if not deleted.intersection(row.sources)), target.values, prime)
        if proof is None:
            return deleted
        if len(deleted) == budget:
            return None
        choices = {source for identifier in proof for source in identities[identifier].sources} - protected - deleted
        for source in sorted(choices):
            witness = search(deleted | {source})
            if witness is not None:
                return witness
        return None

    return search(frozenset())


def refine(records, width, prime, budget, fanin):
    validate(records, width, prime, budget, fanin)
    retained = sorted(records, key=lambda row: row.ordinal)
    statistics = {'proofSearchStates': 0, 'testedRows': 0, 'removedRows': 0}
    witnesses = {}
    for target in list(reversed(retained)):
        others = [row for row in retained if row.ordinal != target.ordinal]
        statistics['testedRows'] += 1
        witness = separating_withdrawal(others, target, prime, budget, statistics)
        if witness is None:
            retained = others
            statistics['removedRows'] += 1
        else:
            witnesses[target.ordinal] = sorted(witness)
    return retained, {'statistics': statistics, 'witnesses': witnesses}


def optimized_union(records, width, prime, budget):
    compressed = provenance_basis(records, prime)
    full_rank = len(reference_space([row.values for row in compressed], width, prime))
    sources = {source for row in compressed for source in row.sources}
    retained = {}
    for deleted in deletion_sets(sources, budget):
        basis = Basis(prime)
        for row in compressed:
            if not deleted.intersection(row.sources) and basis.add(row.values):
                retained[row.ordinal] = row
                if len(basis.rows) == full_rank:
                    break
    return sorted(retained.values(), key=lambda row: row.ordinal)


def compile_method(name, records, width, prime, budget, fanin):
    validate(records, width, prime, budget, fanin)
    if name == 'provenance':
        return provenance_basis(records, prime), {}
    if name == 'lift-refine':
        kernel = lift_certificate(records, width, prime, budget, fanin)
    elif name in ('union', 'union-refine'):
        kernel = optimized_union(records, width, prime, budget)
    elif name == 'direct-refine':
        kernel = provenance_basis(records, prime)
    else:
        raise ValueError('unknown certificate compiler')
    if name == 'union':
        return kernel, {}
    retained, details = refine(kernel, width, prime, budget, fanin)
    return retained, {'kernelRows': len(kernel), **details}


def verify_witnesses(records, details, width, prime, budget):
    if set(details['witnesses']) != {row.ordinal for row in records}:
        raise AssertionError('essentiality witness coverage differs from the retained rows')
    for row in records:
        deleted = frozenset(details['witnesses'][row.ordinal])
        if len(deleted) > budget or deleted.intersection(row.sources):
            raise AssertionError('essentiality witness withdraws its own evidence or exceeds budget')
        others = [other.values for other in surviving(records, deleted, budget) if other.ordinal != row.ordinal]
        if reference_space(others, width, prime) == reference_space([*others, row.values], width, prime):
            raise AssertionError('retained row is not essential under its witness')


def verify_variants(original, variants, width, prime, budget):
    identities = {row.ordinal: row for row in original}
    for rows, details in variants.values():
        if len({row.ordinal for row in rows}) != len(rows) or any(identities.get(row.ordinal) != row for row in rows):
            raise AssertionError('compiler changed, duplicated or invented an evidence row')
        if 'witnesses' in details:
            verify_witnesses(rows, details, width, prime, budget)
    if 'provenance' in variants:
        for sources in {row.sources for row in original}:
            left = [row.values for row in original if row.sources == sources]
            right = [row.values for row in variants['provenance'][0] if row.sources == sources]
            if reference_space(left, width, prime) != reference_space(right, width, prime):
                raise AssertionError('per-provenance control changed its local span')
    full_rank = len(reference_space([row.values for row in original], width, prime))
    queried = {name: pair for name, pair in variants.items() if name != 'provenance'}
    sources_used = {name: {source for row in rows for source in row.sources} for name, (rows, _) in queried.items()}
    caches = {name: {} for name in queried}
    for deleted in deletion_sets({source for row in original for source in row.sources}, budget):
        spans = {}
        for name, (rows, _) in queried.items():
            key = deleted.intersection(sources_used[name])
            if key not in caches[name]:
                caches[name][key] = reference_space([row.values for row in surviving(rows, key, budget)], width, prime)
            spans[name] = caches[name][key]
        lower = [space for space in spans.values() if len(space) != full_rank]
        if lower:
            expected = reference_space([row.values for row in surviving(original, deleted, budget)], width, prime)
            if any(space != expected for space in spans.values()):
                raise AssertionError('certificate differs after withdrawal: ' + str(sorted(deleted)))
        yield {'deleted': sorted(deleted), 'ranks': {name: len(space) for name, space in spans.items()},
               'fullReferenceRequired': bool(lower), 'provenanceControlVerifiedGroupwise': 'provenance' in variants}
