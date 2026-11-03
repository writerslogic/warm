import argparse
import hashlib
import json
import math
from pathlib import Path
import platform
import runpy
import warnings

import numpy as np


def check(name, queries, rows, normalize=False):
    left, right = queries.astype(np.float64), rows.astype(np.float64)
    if normalize:
        right = right / np.linalg.norm(right, axis=1, keepdims=True)
    assert np.isfinite(left).all() and np.isfinite(right).all()
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter('always')
        actual = left @ right.T
    expected = np.einsum('ik,jk->ij', left, right, optimize=False)
    scale = np.einsum('ik,jk->ij', np.abs(left), np.abs(right), optimize=False)
    bound = 4 * left.shape[1] * np.finfo(np.float64).eps * scale
    assert np.isfinite(actual).all() and np.isfinite(expected).all()
    assert np.all(np.abs(actual - expected) <= bound)
    actual_ids = np.argsort(-actual, axis=1, kind='stable')[:, :10]
    expected_ids = np.argsort(-expected, axis=1, kind='stable')[:, :10]
    assert np.array_equal(actual_ids, expected_ids)
    scalar_error = 0.0
    for ordinal in range(min(64, actual.size)):
        i, j = ordinal % len(left), (ordinal * 97) % len(right)
        scalar = math.fsum(float(a) * float(b) for a, b in zip(left[i], right[j]))
        scalar_error = max(scalar_error, abs(float(actual[i, j]) - scalar))
        assert abs(float(actual[i, j]) - scalar) <= float(bound[i, j])
    return {'name': name, 'scoreComparisons': int(actual.size), 'queries': len(left),
            'maximumDifference': float(np.max(np.abs(actual - expected))),
            'maximumSampledFsumDifference': scalar_error,
            'identicalTop10': True, 'allFinite': True,
            'warnings': [str(item.message) for item in caught],
            'scoresSha256': hashlib.sha256(actual.tobytes()).hexdigest()}


def main(source):
    module = runpy.run_path(str(source))
    clustered = module['clustered']
    rows, queries = clustered(4000, 48, 7), clustered(200, 48, 8)
    results = [check('angular', queries, rows, True)]
    for seed in (9, 10):
        scaled = rows * np.random.default_rng(seed).uniform(.2, 5., (len(rows), 1)).astype(np.float32)
        results.append(check('scaled-angular-' + str(seed), queries, scaled, True))
        if seed == 10:
            results.append(check('inner-product', queries, scaled))
    rng = np.random.default_rng(47)
    rows = clustered(256, 5, 48) * rng.uniform(.1, 7., (256, 1)).astype(np.float32)
    results.append(check('training-query-inner-product', clustered(16, 5, 50), rows))
    for size in (14, 15, 784):
        matrix = np.identity(size)
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter('always')
            result = matrix @ matrix
        assert np.array_equal(result, matrix)
        results.append({'name': 'identity-' + str(size), 'exactIdentity': True,
                        'warnings': [str(item.message) for item in caught]})
    return {'complete': True, 'python': platform.python_version(), 'numpy': np.__version__,
            'sourceSha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'cases': results,
            'reference': 'Non-BLAS einsum over every score plus sampled Python math.fsum; exact top-10 identities required',
            'upstream': 'https://github.com/numpy/numpy/pull/29223'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = main(args.source)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
