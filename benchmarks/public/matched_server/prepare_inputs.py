# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Seal the preselected performance cohort without running any search."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import time

import h5py
import numpy as np


HERE = Path(__file__).resolve().parent
DATASETS = {'coco-i2i-512-angular': 512, 'deep-image-96-angular': 96}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    with path.open('x', encoding='utf8') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def cohort(dataset, dimensions):
    if dataset.ndim != 2 or dataset.shape[1] != dimensions or dataset.shape[0] < 2000:
        raise ValueError('unexpected held-out query shape')
    with np.errstate(over='ignore', invalid='ignore'):
        values = np.asarray(dataset[1000:2000], dtype='<f4')
    invalid = ~np.isfinite(values).all(axis=1) | ~np.any(values != 0, axis=1)
    if invalid.any():
        raise ValueError('ineligible frozen query ordinals: ' + ','.join(map(str, np.flatnonzero(invalid) + 1000)))
    return values


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--accuracy-inputs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.accuracy_inputs.is_absolute() or not args.output.is_absolute():
        parser.error('input and output paths must be absolute')
    original = json.loads((args.accuracy_inputs / 'finished.json').read_text())
    assert original['complete'] is True
    assert digest(args.accuracy_inputs / 'freeze.json') == original['freezeSha256']
    args.output.mkdir()
    protocol = HERE.parents[2] / 'docs/research/milestones-20261102/PERFORMANCE.md'
    copied = {}
    for source in [Path(__file__), HERE / 'test_prepare_inputs.py', protocol,
                   HERE.parent / 'accuracy_qualification/pyproject.toml', HERE.parent / 'accuracy_qualification/uv.lock']:
        target = args.output / source.name
        shutil.copyfile(source, target)
        copied[source.name] = digest(target)
    freeze = {'datasets': DATASETS, 'rows': 100000, 'calibrationOrdinals': [0, 999],
              'qualificationOrdinals': [1000, 1999], 'queries': 1000,
              'eligibility': 'Fail on any nonfinite/zero selected f32 vector; no replacement or exclusion.',
              'accuracyInputFreezeSha256': original['freezeSha256'], 'sources': copied,
              'started': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'numpy': np.__version__, 'h5py': h5py.__version__,
              'scope': 'Input sealing only; calibration grid and executable run plan still must be frozen before calibration.'}
    write(args.output / 'freeze.json', freeze)
    completed = []
    try:
        for name, dimensions in DATASETS.items():
            source = args.accuracy_inputs / name
            record = json.loads((source / 'inputs.json').read_text())
            assert digest(source / 'inputs.json') == next(item['inputsSha256'] for item in original['completed'] if item['name'] == name)
            assert record['dimensions'] == dimensions
            assert record['training']['retainedOrdinals'] == list(range(100000))
            assert record['queries']['retainedOrdinals'] == list(range(1000))
            assert digest(source / 'source.hdf5') == record['sourceSha256']
            destination = args.output / name
            destination.mkdir()
            for old, new, expected in [('train.f32', 'train.f32', record['training']['sha256']),
                                        ('test.f32', 'calibration.f32', record['queries']['sha256'])]:
                shutil.copyfile(source / old, destination / new)
                assert digest(destination / new) == expected
            with h5py.File(source / 'source.hdf5', 'r') as data:
                values = cohort(data['test'], dimensions)
                source_shape = list(data['test'].shape)
            with (destination / 'test.f32').open('xb') as stream:
                stream.write(values.tobytes())
            entry = {'name': name, 'dimensions': dimensions, 'sourceSha256': record['sourceSha256'],
                     'sourceQueryShape': source_shape, 'queryOrdinals': list(range(1000, 2000)),
                     'files': {p.name: {'sha256': digest(p), 'bytes': p.stat().st_size}
                               for p in sorted(destination.glob('*.f32'))}}
            write(destination / 'inputs.json', entry)
            completed.append({'name': name, 'inputsSha256': digest(destination / 'inputs.json')})
            print(json.dumps({'name': name, 'sealedQueries': len(values)}), flush=True)
    except Exception as error:
        write(args.output / 'finished.json', {'complete': False, 'completed': completed,
              'error': {'type': type(error).__name__, 'message': str(error)}})
        raise
    write(args.output / 'finished.json', {'complete': True, 'completed': completed,
          'freezeSha256': digest(args.output / 'freeze.json')})


if __name__ == '__main__':
    main()
