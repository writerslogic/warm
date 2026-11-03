# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Independent float64 cosine reference and strict evidence gates."""

import math

import numpy as np


def normalize(values):
    values = np.asarray(values, dtype=np.float64)
    scales = np.max(np.abs(values), axis=1, keepdims=True)
    if not np.isfinite(values).all() or np.any(scales == 0):
        raise ValueError('invalid cosine input')
    result = values / scales
    result /= np.sqrt(np.sum(result * result, axis=1, keepdims=True))
    return result


def truth(train, queries, churn=False):
    rows = normalize(train)
    if churn:
        if len(rows) < 200:
            raise ValueError('churn requires at least 200 rows')
        rows[:100] = rows[1:101].copy()
    questions = normalize(queries)
    identities = np.arange(len(rows))
    live = np.ones(len(rows), dtype=bool)
    if churn:
        live[100:200] = False
    eligible = {'all': identities[live], '10pct': identities[live & (identities % 100 < 10)],
                '1pct': identities[live & (identities % 100 == 0)], 'empty': np.array([], dtype=int)}
    result = {name: [] for name in eligible}
    for lo in range(0, len(questions), 16):
        scores = rows @ questions[lo:lo + 16].T
        for column in range(scores.shape[1]):
            for name, ids in eligible.items():
                values = scores[ids, column]
                count = min(10, len(ids))
                if count:
                    cutoff = np.partition(values, len(values) - count)[len(values) - count]
                    candidates = np.flatnonzero(values >= cutoff)
                    order = np.lexsort((ids[candidates], -values[candidates]))[:count]
                    selected = candidates[order]
                    result[name].append({'ids': ids[selected].tolist(), 'scores': values[selected].tolist(),
                                         'eligible': len(ids), 'cutoff': float(cutoff)})
                else:
                    result[name].append({'ids': [], 'scores': [], 'eligible': 0, 'cutoff': None})
    return result


def summarize(records, expected_count, exact):
    if len(records) != expected_count or {row['query'] for row in records} != set(range(expected_count)):
        raise ValueError('incomplete or duplicate query evidence')
    latencies = []
    recall = []
    errors = 0
    unresolved = 0
    for row in records:
        errors += bool(row.get('error'))
        hits = row.get('hits', [])
        expected = row['expected']
        score = len(set(hits) & set(expected)) / len(expected) if expected else int(not hits)
        recall.append(score)
        unresolved += exact and set(hits) != set(expected)
        elapsed = row['seconds']
        if not math.isfinite(elapsed) or elapsed < 0:
            raise ValueError('invalid latency evidence')
        latencies.append(elapsed)
    ordered = sorted(latencies)
    percentiles = {name: ordered[max(0, math.ceil(probability * len(ordered)) - 1)]
                   for name, probability in [('median', .5), ('p95', .95), ('p99', .99)]}
    mean = math.fsum(recall) / len(recall)
    worst = min(recall)
    return {'queries': len(records), 'meanRecall': mean, 'worstRecall': worst, 'errors': errors,
            'unresolvedExactMembership': unresolved, **percentiles,
            'pass': mean >= .99 and worst >= .9 and errors == 0 and unresolved == 0}
