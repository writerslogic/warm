# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Exercise the new client runner against local servers using synthetic inputs only."""

import argparse
import json
import os
from pathlib import Path
import platform
import traceback

import numpy as np

from adapters import FILTERS, SYSTEMS
from calibrate import sha, write
from measure import run_cell
from oracle import truth


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', required=True, choices=('warm', 'hnsw', 'ivfflat', 'faiss', 'lucene'))
    args = parser.parse_args()
    if not args.output.is_absolute() or platform.system() != 'Linux' or len(os.sched_getaffinity(0)) != 4:
        parser.error('requires an absolute output directory and four-CPU Linux affinity')
    args.output.mkdir()
    plan = json.loads((Path(__file__).parent / 'PLAN.json').read_text())
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
    write(args.output / 'inputs.json', {'synthetic': True, 'seed': 9173,
          'trainSha256': sha(args.output / 'train.f32'), 'queriesSha256': sha(args.output / 'queries.f32'),
          'rows': len(train), 'queries': len(queries), 'dimensions': 16})
    system = SYSTEMS[engine](args.output / 'server', 4, 'measurement-development', dim=16)
    cells, failure = [], None
    try:
        system.start()
        system.create(build)
        system.ingest(train, build)
        system.wait_queryable(build)
        for churn in (False, True):
            if churn:
                system.close()
                system.connect()
                for ordinal in range(100):
                    system.replace_one(ordinal, train[ordinal + 1])
                for ordinal in range(100, 200):
                    system.delete_one(ordinal)
            expected = truth(train, queries, churn)
            for name in FILTERS:
                configuration = {
                    'warm': {'route': 'graph' if name == 'all' else 'exact', 'ef': 512},
                    'pgvector': {'route': 'iterative', 'ef': 512, 'probes': 100, 'lists': 100},
                    'opensearch': {'route': 'knn', 'ef': 512, 'candidateK': 32}}[engine]
                for clients in (1, 8):
                    system.server.sampling(False)
                    try:
                        result = run_cell(engine=engine, port=system.port, configuration=configuration,
                            filter_name=name, dataset='synthetic-measurement-9173', repetition=0,
                            clients=clients, queries=queries, warmup=queries[:8], expected=expected[name],
                            train_path=args.output / 'train.f32', shape=train.shape, churn=churn,
                            output=args.output / f'{int(churn)}-{name}-{clients}', limits=plan['resources'],
                            server_pgid=system.server.proc.pid)
                    finally:
                        system.server.sampling(True)
                    cells.append(result)
                    print(json.dumps({'churn': churn, 'filter': name, 'clients': clients,
                                      'complete': result['complete'], 'pass': result['pass'],
                                      'failure': result['failure']}), flush=True)
                    if not result['pass']:
                        raise RuntimeError('synthetic concurrent client check failed')
    except Exception as error:
        failure = {'type': type(error).__name__, 'message': str(error), 'traceback': traceback.format_exc()}
        raise
    finally:
        try:
            system.stop()
        finally:
            write(args.output / 'finished.json', {'complete': failure is None, 'failure': failure,
                  'engine': engine, 'build': build, 'synthetic': True, 'benchmarkClaim': False, 'cells': cells})


if __name__ == '__main__':
    main()
