# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Reproduce the existing exposed synthetic inputs; no stored ground truth is used."""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output', required=True, type=Path)
p.add_argument('--noise', type=float, choices=(.7, 1.5), default=.7)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
rng = np.random.default_rng(1)
centers = rng.normal(0, 1.0, (1000, 100)).astype(np.float32)
base = np.empty((1000000, 100), np.float32)
for index in range(1000):
    base[index * 1000:(index + 1) * 1000] = centers[index] + rng.normal(0, a.noise, (1000, 100)).astype(np.float32)
query_centers = rng.integers(0, 1000, 1000)
queries = (centers[query_centers] + rng.normal(0, a.noise, (1000, 100))).astype(np.float32)
receipts = {}
for name, values in [('base', base), ('queries', queries)]:
    path = a.output / (name + '.npy')
    np.save(path, values)
    sha = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            sha.update(block)
    receipts[name] = sha.hexdigest()
print(json.dumps({'numpy_version': np.__version__, 'noise': a.noise, 'sha256': receipts, 'scope': 'existing exposed development only'}))
