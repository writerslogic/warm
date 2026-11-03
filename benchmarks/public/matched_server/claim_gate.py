# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Compare a complete matrix of already-verified serving measurements."""

from itertools import product
import math


ENGINES = ('warm', 'pgvector', 'opensearch')
DIMENSIONS = ('hardware', 'dataset', 'repetition', 'state', 'filter', 'clients')


def key(row):
    return tuple(row[name] for name in DIMENSIONS)


def quality(row, plan):
    gate = plan['qualification']['quality']
    numbers = [row.get(name) for name in ('meanRecall', 'worstRecall', 'median', 'p95', 'p99', 'queriesPerSecond')]
    if not all(type(value) in (int, float) and math.isfinite(value) and value >= 0 for value in numbers):
        return False
    if not all(type(row.get(name)) is int for name in ('queries', 'completed', 'errors', 'unresolvedExactMembership')):
        return False
    return (row.get('complete') is True and row.get('evidenceVerified') is True
            and row.get('resourcesPassed') is True and row.get('isolationPassed') is True
            and row.get('developmentOnly') is False and row.get('resourceAccountingAvailable') is True
            and row.get('queries') == plan['queries'] and row.get('completed') == plan['queries']
            and row.get('errors') == 0 and row.get('unresolvedExactMembership') == 0
            and row['meanRecall'] >= gate['meanRecallAt10'] and row['worstRecall'] >= gate['worstRecallAt10']
            and row['worstRecall'] <= row['meanRecall'] <= 1
            and 0 < row['median'] <= row['p95'] <= row['p99'] and row['queriesPerSecond'] > 0)


def evaluate(rows, plan):
    expected = set(product(plan['resources']['physicalHosts'], plan['datasets'],
                           range(plan['qualification']['repetitions']), plan['states'], plan['filters'],
                           plan['qualification']['concurrency']))
    lookup, invalid = {}, []
    for row in rows:
        identity = (row.get('engine'), key(row))
        if identity[0] not in ENGINES or identity[1] not in expected:
            invalid.append({'reason': 'undeclared cell', 'identity': identity})
        elif identity in lookup:
            invalid.append({'reason': 'duplicate cell', 'identity': identity})
        else:
            lookup[identity] = row
    missing = [(engine, cell) for cell in sorted(expected) for engine in ENGINES if (engine, cell) not in lookup]
    result = {'complete': not invalid and not missing, 'invalid': invalid, 'missing': missing,
              'latencyPassed': False, 'advantagePassed': False, 'cells': [],
              'scope': 'Numeric gates only. evidenceVerified/resourcesPassed/isolationPassed require separate raw-evidence validation.'}
    if not result['complete']:
        return result
    latency_pass, advantage_pass = True, True
    for cell in sorted(expected):
        warm = lookup['warm', cell]
        candidates = [lookup[engine, cell] for engine in ENGINES[1:] if quality(lookup[engine, cell], plan)]
        row = dict(zip(DIMENSIONS, cell))
        row['warmQualityPassed'] = quality(warm, plan)
        row['ineligibleComparators'] = [engine for engine in ENGINES[1:] if not quality(lookup[engine, cell], plan)]
        row['eligibleComparators'] = [entry['engine'] for entry in candidates]
        row['latencyPassed'] = False
        target = row['state'] == 'churn-after-maintenance' and row['filter'] == 'all' and row['clients'] == 8
        if row['warmQualityPassed'] and candidates:
            row['latencyComparisons'] = [{
                'engine': comparator['engine'],
                'medianRatio': warm['median'] / comparator['median'],
                'p95Ratio': warm['p95'] / comparator['p95'],
                'p99Ratio': warm['p99'] / comparator['p99'],
                'pass': warm['median'] <= .9 * comparator['median']
                        and warm['p95'] <= .9 * comparator['p95']
                        and warm['p99'] <= 1.05 * comparator['p99']}
                for comparator in candidates]
            row['latencyPassed'] = all(entry['pass'] for entry in row['latencyComparisons'])
            if target:
                fastest = max(candidates, key=lambda comparator: comparator['queriesPerSecond'])
                row['throughputComparator'] = fastest['engine']
                row['throughputRatio'] = warm['queriesPerSecond'] / fastest['queriesPerSecond']
                row['advantagePassed'] = warm['queriesPerSecond'] >= 2 * fastest['queriesPerSecond']
        elif target:
            row['advantagePassed'] = False
        latency_pass &= row['latencyPassed']
        if target:
            advantage_pass &= row.get('advantagePassed', False)
        result['cells'].append(row)
    result.update({'latencyPassed': latency_pass, 'advantagePassed': advantage_pass,
                   'advantageTargetCells': sum('advantagePassed' in row for row in result['cells'])})
    return result
