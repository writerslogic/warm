# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Read-only scoring inspection of the preserved synthetic OpenSearch failure."""
import argparse
import json
from pathlib import Path

import numpy as np

from adapters import Search, SearchClient
from oracle import truth


parser = argparse.ArgumentParser()
parser.add_argument('--store', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
generator = np.random.default_rng(9173)
train = generator.normal(size=(1024, 16)).astype('<f4')
queries = generator.normal(size=(12, 16)).astype('<f4')
query = queries[0]
expected = truth(train, queries, True)['all']
train[:100] = train[1:101].copy()
system = Search(args.store, 4, 'scoring-diagnosis', dim=16)
results = {}
try:
    system.start()
    for route in ['exact', 'knn']:
        client = SearchClient(system.port, {'route': route, 'ef': 512}, 'all')
        try:
            body = client.prepare(query)
            result = client.http.call('POST', '/idx/_search?request_cache=false&explain=true', body=body)
            for hit in result['hits']['hits']:
                row = train[int(hit['_id'])].astype(float)
                q = query.astype(float)
                hit['independentCosine'] = float(np.dot(row, q) / np.sqrt(np.dot(row, row) * np.dot(q, q)))
            results[route] = result
        finally:
            client.close()
    results['settings'] = system.inspect()
    results['candidateCounts'] = []
    client = SearchClient(system.port, {'route': 'knn', 'ef': 512}, 'all')
    try:
        for count in [10, 32, 64]:
            rows = []
            for i, q in enumerate(queries):
                body = json.loads(client.prepare(q))
                body['query']['knn']['vec']['k'] = count
                response = client.http.call('POST', '/idx/_search?request_cache=false', body=json.dumps(body).encode())
                hits = response['hits']['hits']
                identities = [int(hit['_id']) for hit in hits]
                rows.append({'query': i, 'hits': identities, 'expected': expected[i]['ids'],
                             'recall': len(set(identities) & set(expected[i]['ids'])) / 10})
            results['candidateCounts'].append({'k': count, 'rows': rows,
                'meanRecall': sum(row['recall'] for row in rows) / len(rows),
                'worstRecall': min(row['recall'] for row in rows)})
    finally:
        client.close()
finally:
    system.stop()
with args.output.open('x') as stream:
    json.dump(results, stream, indent=2, allow_nan=False)
    stream.write('\n')
