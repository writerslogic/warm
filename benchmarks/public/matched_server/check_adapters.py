# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Local synthetic integration and acknowledged-write process-death recovery check."""

import argparse
import json
import os
from pathlib import Path
import signal
import time

import numpy as np

from adapters import CLIENTS, FILTERS, SYSTEMS, payload, validate_hits
from oracle import summarize, truth


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--engine', choices=SYSTEMS, required=True)
    parser.add_argument('--candidate-k', type=int, choices=[10, 32], default=10)
    parser.add_argument('--opensearch-engine', choices=['faiss', 'lucene'], default='faiss')
    parser.add_argument('--pg-index', choices=['hnsw', 'ivfflat'], default='hnsw')
    args = parser.parse_args()
    args.output.mkdir()
    rng = np.random.default_rng(9173)
    train = rng.normal(size=(1024, 16)).astype('<f4')
    queries = rng.normal(size=(12, 16)).astype('<f4')
    builds = {'warm': {}, 'pgvector': {'index': 'hnsw', 'm': 16, 'ef_construction': 64},
              'opensearch': {'engine': args.opensearch_engine, 'm': 16, 'ef_construction': 100}}
    if args.pg_index == 'ivfflat':
        builds['pgvector'] = {'index': 'ivfflat', 'lists': 100}
    system = SYSTEMS[args.engine](args.output / 'server', 4, 'adapter-check', dim=16)
    cells, details = [], []
    failure = None
    try:
        system.start()
        system.create(builds[args.engine])
        system.ingest(train, builds[args.engine])
        system.wait_queryable(builds[args.engine])
        details.append(system.inspect())
        for churn in [False, True]:
            if churn:
                for i in range(100):
                    system.replace_one(i, train[i + 1])
                for i in range(100, 200):
                    system.delete_one(i)
                ack = args.output / 'ack.json'
                with ack.open('x') as stream:
                    json.dump({'replacements': list(range(100)), 'deletions': list(range(100, 200))}, stream)
                    stream.flush()
                    os.fsync(stream.fileno())
                system.close()
                if not system.server.alive():
                    raise RuntimeError('server died before injected crash')
                os.killpg(system.server.proc.pid, signal.SIGKILL)
                system.server.proc.wait(timeout=30)
                system.server.stop(grace=1)
                start = time.monotonic()
                system.start()
                for i in range(100):
                    actual = system.fetch(i)
                    expected = payload(i, 1)
                    if actual is None or any(actual.get(k) != expected[k] for k in expected):
                        raise RuntimeError(f'acknowledged replacement not recovered: {i}')
                for i in range(100, 200):
                    if system.fetch(i) is not None:
                        raise RuntimeError(f'acknowledged deletion not recovered: {i}')
                details.append({'recoverySeconds': time.monotonic() - start, 'acknowledgedReplacements': 100,
                                'acknowledgedDeletions': 100})
            expected = truth(train, queries, churn)
            for name in FILTERS:
                for route in ['exact', 'candidate']:
                    config = {'route': 'exact'} if route == 'exact' else {
                        'warm': {'route': 'graph' if name == 'all' else 'exact', 'ef': 512},
                        'pgvector': {'route': 'iterative', 'ef': 512, 'probes': 100, 'lists': 100},
                        'opensearch': {'route': 'knn', 'ef': 512, 'candidateK': args.candidate_k}}[args.engine]
                    client = CLIENTS[args.engine](system.port, config, name)
                    records = []
                    try:
                        for i, query in enumerate(queries):
                            prepared = client.prepare(query)
                            start = time.perf_counter()
                            hits = client.query(prepared)
                            elapsed = time.perf_counter() - start
                            actual = validate_hits(hits, len(train), churn, name)
                            records.append({'query': i, 'seconds': elapsed, 'hits': sorted(actual),
                                            'expected': expected[name][i]['ids']})
                    finally:
                        client.close()
                    cell = {'churn': churn, 'filter': name, 'configuration': config,
                            **summarize(records, len(queries), config['route'] == 'exact')}
                    cells.append(cell)
                    write(args.output / f'{int(churn)}-{name}-{route}.json', {'cell': cell, 'records': records})
                    print(json.dumps(cell), flush=True)
                    if not cell['pass']:
                        raise RuntimeError('adapter check failed query quality')
    except Exception as error:
        failure = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        try:
            system.stop()
        finally:
            write(args.output / 'finished.json', {'complete': failure is None, 'engine': args.engine,
                  'synthetic': True, 'benchmarkClaim': False, 'failure': failure, 'cells': cells, 'details': details})


if __name__ == '__main__':
    main()
