# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Run the full serving lifecycle on synthetic data before qualification admission."""

import argparse
import copy
import json
import os
from pathlib import Path
import platform

import numpy as np

from calibrate import sha, write
from oracle import truth
from qualify import run_job
from verify_evidence import verify_cell


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=('warm', 'hnsw', 'ivfflat', 'faiss', 'lucene'), required=True)
    args = parser.parse_args()
    if not args.output.is_absolute() or platform.system() != 'Linux' or len(os.sched_getaffinity(0)) != 4:
        parser.error('requires an absolute output path and four-CPU Linux affinity')
    args.output.mkdir()
    plan = copy.deepcopy(json.loads((Path(__file__).parent / 'PLAN.json').read_text()))
    dataset = 'synthetic-qualification-9173'
    plan.update({'rows': 1024, 'queries': 12, 'datasets': {dataset: 16}})
    plan['qualification']['warmQueries'] = 8
    engine, build = {
        'warm': ('warm', {}),
        'hnsw': ('pgvector', {'index': 'hnsw', 'm': 16, 'ef_construction': 64}),
        'ivfflat': ('pgvector', {'index': 'ivfflat', 'lists': 100}),
        'faiss': ('opensearch', {'engine': 'faiss', 'm': 16, 'ef_construction': 100}),
        'lucene': ('opensearch', {'engine': 'lucene', 'm': 16, 'ef_construction': 100}),
    }[args.variant]
    generator = np.random.default_rng(9173)
    train = generator.normal(size=(1024, 16)).astype('<f4')
    queries = generator.normal(size=(12, 16)).astype('<f4')
    train.tofile(args.output / 'train.f32')
    queries.tofile(args.output / 'queries.f32')
    filters = {}
    for name in plan['filters']:
        filters[name] = {
            'warm': {'route': 'graph' if name == 'all' else 'exact', 'ef': 512},
            'pgvector': {'route': 'iterative', 'ef': 512, 'probes': 100, 'lists': 100},
            'opensearch': {'route': 'knn', 'ef': 512, 'candidateK': 32}}[engine]
    selected = {'build': build, 'filters': filters}
    write(args.output / 'freeze.json', {'synthetic': True, 'benchmarkClaim': False, 'plan': plan,
          'selected': selected, 'trainSha256': sha(args.output / 'train.f32'),
          'queriesSha256': sha(args.output / 'queries.f32')})
    expected = {'baseline': truth(train, queries), 'churn': truth(train, queries, True)}
    write(args.output / 'reference.json', expected)
    result = run_job(engine=engine, dataset=dataset, repetition=0, selected=selected, plan=plan,
        train_path=args.output / 'train.f32', queries=queries, warmup=queries[:8], expected=expected,
        output=args.output / 'job', hardware='synthetic development; no hardware qualification')
    verified = []
    if result['complete']:
        for state in plan['states']:
            churn = state != 'baseline'
            for filter_name in plan['filters']:
                for clients in plan['qualification']['concurrency']:
                    identity = {'engine': engine, 'dataset': dataset, 'repetition': 0,
                                'configuration': selected['filters'][filter_name], 'filter': filter_name,
                                'churn': churn, 'clients': clients}
                    evidence = verify_cell(args.output / f'job/{state}-{filter_name}-{clients}',
                        expected=expected['churn' if churn else 'baseline'][filter_name], queries=queries,
                        train_path=args.output / 'train.f32', shape=train.shape, limits=plan['resources'], identity=identity)
                    verified.append({'state': state, **identity, **evidence})
        write(args.output / 'verification.json', {'cells': verified, 'synthetic': True, 'benchmarkClaim': False})
        if any(not row['qualityPassed'] or not row['resourcesPassed'] for row in verified):
            raise RuntimeError('synthetic raw evidence failed verification')
    write(args.output / 'finished.json', {'complete': result['complete'], 'qualityPassed': result['qualityPassed'],
          'synthetic': True, 'benchmarkClaim': False, 'jobReceiptSha256': sha(args.output / 'job/finished.json')})
    if not result['complete'] or not result['qualityPassed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
