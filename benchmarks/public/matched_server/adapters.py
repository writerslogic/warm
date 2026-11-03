# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Matched payload adapters; process ownership comes from the existing server harness."""

import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'competitive/warmbench'))
from server_worker import OpenSearchKnn, Postgres, WarmServer


FILTERS = {'all': None, '10pct': {'eligible10': True}, '1pct': {'eligible1': True},
           'empty': {'bucket100': -1}}


def payload(ordinal, epoch=0):
    if type(ordinal) is not int or ordinal < 0 or type(epoch) is not int or epoch < 0:
        raise ValueError('invalid payload identity')
    header = f'record {ordinal:06d} revision {epoch} '
    suffix = hashlib.sha256(header.encode()).hexdigest() + ' '
    text = (header + suffix * 16)[:1024]
    return {'id': str(ordinal), 'text': text,
            'metadata': {'ordinal': ordinal, 'bucket100': ordinal % 100,
                         'eligible10': ordinal % 100 < 10, 'eligible1': ordinal % 100 == 0,
                         'epoch': epoch}}


def encode(value):
    return json.dumps(value, separators=(',', ':'), allow_nan=False).encode()


class HttpError(RuntimeError):
    def __init__(self, status, message):
        super().__init__(message)
        self.status = status


class Http:
    def __init__(self, port, timeout=30):
        self.conn = http.client.HTTPConnection('127.0.0.1', port, timeout=timeout)

    def call(self, method, path, obj=None, body=None, content_type='application/json'):
        if obj is not None:
            body = encode(obj)
        self.conn.request(method, path, body=body,
                          headers={'Content-Type': content_type} if body is not None else {})
        response = self.conn.getresponse()
        raw = response.read()
        if response.status >= 400:
            raise HttpError(response.status, f'{method} {path}: HTTP {response.status}: {raw[:600]!r}')
        return json.loads(raw) if raw else None

    def close(self):
        self.conn.close()


def vector_text(vector):
    return '[' + ','.join(map(repr, vector.tolist())) + ']'


class WarmClient:
    def __init__(self, port, configuration, filter_name):
        self.http = Http(port)
        self.configuration, self.filter_name = configuration, filter_name

    def prepare(self, vector):
        body = {'query_vector': vector.tolist(), 'top_k': 10,
                'index': self.configuration['route'], 'ef': self.configuration.get('ef', 128)}
        if FILTERS[self.filter_name] is not None:
            if body['index'] != 'exact':
                raise ValueError('filtered WARM queries must use exact mode')
            body['filter'] = FILTERS[self.filter_name]
        return encode(body)

    def query(self, prepared):
        return self.http.call('POST', '/api/v1/query', body=prepared)['matches']

    def close(self):
        self.http.close()


class Warm(WarmServer):
    def command(self):
        argv, env, cwd = super().command()
        return argv + ['--max-concurrent-work', '8', '--max-queue', '32'], env, cwd

    def connect(self):
        self.http = Http(self.port, 7200)

    def close(self):
        if getattr(self, 'http', None) is not None:
            self.http.close()

    def ingest(self, train, params):
        for lo in range(0, len(train), 500):
            docs = [{**payload(i), 'vector': train[i].tolist()}
                    for i in range(lo, min(lo + 500, len(train)))]
            result = self.http.call('POST', '/api/v1/documents/batch', docs)
            if result['added'] != len(docs):
                raise RuntimeError('incomplete WARM batch acknowledgement')

    def replace_one(self, ordinal, vector, epoch=1):
        self.http.call('POST', '/api/v1/documents', {**payload(ordinal, epoch), 'vector': vector.tolist()})

    def delete_one(self, ordinal):
        self.http.call('DELETE', f'/api/v1/documents/{ordinal}')

    def inspect(self):
        return self.http.call('GET', '/api/v1/stats')

    def maintain(self, params):
        return self.http.call('POST', '/api/v1/index/maintain', {})

    def fetch(self, ordinal):
        hits = self.http.call('POST', '/api/v1/query', {'query_vector': [1.] * self.dim,
                              'top_k': 1, 'index': 'exact', 'document_ids': [str(ordinal)]})['matches']
        return hits[0] if hits else None


class PgClient:
    def __init__(self, port, configuration, filter_name):
        import psycopg
        self.db = psycopg.connect(host='127.0.0.1', port=port, user='postgres', dbname='postgres',
                                 autocommit=True, connect_timeout=5)
        self.db.prepare_threshold = 0
        self.db.execute('SET statement_timeout = 30000')
        route = configuration['route']
        if route not in ('planner', 'iterative', 'exact'):
            raise ValueError('unknown PostgreSQL route')
        predicate = {'all': 'true', '10pct': 'eligible10', '1pct': 'eligible1', 'empty': 'bucket100 = -1'}[filter_name]
        if route == 'iterative':
            self.db.execute('SET enable_seqscan = off')
            self.db.execute("SET hnsw.iterative_scan = strict_order")
            self.db.execute("SET ivfflat.iterative_scan = relaxed_order")
            self.db.execute('SET hnsw.max_scan_tuples = 100000')
        for name, value in [('hnsw.ef_search', configuration.get('ef', 128)),
                            ('ivfflat.probes', configuration.get('probes', 1)),
                            ('ivfflat.max_probes', configuration.get('lists', 100))]:
            if type(value) is not int or not 1 <= value <= 1000:
                raise ValueError('invalid PostgreSQL search parameter')
            self.db.execute(f'SET {name} = {value}')
        order = 'distance + 0, id' if route == 'exact' else 'distance'
        source = f'SELECT id, text, metadata, v <=> %s::vector AS distance FROM t WHERE {predicate}'
        inner = f'SELECT * FROM ({source}) AS candidates ORDER BY {order} LIMIT 10'
        self.sql = f'WITH hits AS MATERIALIZED ({inner}) SELECT id, text, metadata, 1-distance FROM hits ORDER BY distance + 0, id'

    def prepare(self, vector):
        return (vector_text(vector),)

    def query(self, prepared):
        return [{'id': str(row[0]), 'text': row[1], 'metadata': row[2], 'score': row[3]}
                for row in self.db.execute(self.sql, prepared).fetchall()]

    def close(self):
        self.db.close()


class Pg(Postgres):
    def close(self):
        if getattr(self, 'db', None) is not None:
            self.db.close()

    def command(self):
        argv, env, cwd = super().command()
        return argv + ['-c', 'shared_buffers=1536MB', '-c', 'work_mem=32MB',
                       '-c', 'maintenance_work_mem=2GB', '-c', 'fsync=on',
                       '-c', 'synchronous_commit=on', '-c', 'full_page_writes=on',
                       '-c', 'max_connections=32', '-c', 'autovacuum=on'], env, cwd

    def create(self, params):
        self.db.execute('CREATE EXTENSION vector')
        version = self.db.execute("SELECT extversion FROM pg_extension WHERE extname='vector'").fetchone()[0]
        if version != '0.8.7' or self.db.info.server_version != 180006:
            raise RuntimeError('unexpected PostgreSQL/pgvector version')
        self.db.execute(f'CREATE TABLE t (id integer PRIMARY KEY, v vector({self.dim}), text text NOT NULL, metadata jsonb NOT NULL, bucket100 integer NOT NULL, eligible10 boolean NOT NULL, eligible1 boolean NOT NULL)')
        self.db.execute('CREATE INDEX t_bucket ON t (bucket100)')
        self.db.execute('CREATE INDEX t_eligible10 ON t (id) WHERE eligible10')
        self.db.execute('CREATE INDEX t_eligible1 ON t (id) WHERE eligible1')

    def ingest(self, train, params):
        from psycopg.types.json import Jsonb
        for lo in range(0, len(train), 500):
            with self.db.cursor() as cur, cur.copy('COPY t (id,v,text,metadata,bucket100,eligible10,eligible1) FROM STDIN') as copy:
                for i in range(lo, min(lo + 500, len(train))):
                    doc = payload(i)
                    copy.write_row((i, vector_text(train[i]), doc['text'], Jsonb(doc['metadata']),
                                    i % 100, i % 100 < 10, i % 100 == 0))

    def wait_queryable(self, params):
        if params['index'] == 'hnsw':
            self.db.execute(f"CREATE INDEX t_v ON t USING hnsw (v vector_cosine_ops) WITH (m={params['m']}, ef_construction={params['ef_construction']})")
        else:
            self.db.execute(f"CREATE INDEX t_v ON t USING ivfflat (v vector_cosine_ops) WITH (lists={params['lists']})")
        return self.maintain(params)

    def maintain(self, params):
        self.db.execute('VACUUM ANALYZE t')
        self.checkpoint()

    def replace_one(self, ordinal, vector, epoch=1):
        from psycopg.types.json import Jsonb
        doc = payload(ordinal, epoch)
        result = self.db.execute('UPDATE t SET v=%s::vector,text=%s,metadata=%s WHERE id=%s',
                                (vector_text(vector), doc['text'], Jsonb(doc['metadata']), ordinal))
        if result.rowcount != 1:
            raise RuntimeError('replacement did not affect one row')

    def delete_one(self, ordinal):
        if self.db.execute('DELETE FROM t WHERE id=%s', (ordinal,)).rowcount != 1:
            raise RuntimeError('deletion did not affect one row')

    def inspect(self):
        return {name: self.db.execute(f'SHOW {name}').fetchone()[0]
                for name in ['server_version', 'fsync', 'synchronous_commit', 'full_page_writes',
                             'wal_sync_method', 'shared_buffers', 'work_mem', 'maintenance_work_mem',
                             'max_parallel_workers_per_gather', 'max_parallel_maintenance_workers']}

    def fetch(self, ordinal):
        row = self.db.execute('SELECT id,text,metadata FROM t WHERE id=%s', (ordinal,)).fetchone()
        return None if row is None else dict(zip(('id', 'text', 'metadata'), (str(row[0]), row[1], row[2])))


class SearchClient:
    def __init__(self, port, configuration, filter_name):
        self.http, self.configuration, self.filter_name = Http(port), configuration, filter_name

    def prepare(self, vector):
        predicate = FILTERS[self.filter_name]
        filtered = {'match_all': {}} if predicate is None else {'term': {'metadata.' + key: value for key, value in predicate.items()}}
        if self.configuration['route'] == 'exact':
            query = {'script_score': {'query': filtered, 'script': {'source': 'knn_score', 'lang': 'knn',
                     'params': {'field': 'vec', 'query_value': vector.tolist(), 'space_type': 'cosinesimil'}}}}
        elif self.configuration['route'] == 'knn':
            field = {'vector': vector.tolist(), 'k': self.configuration.get('candidateK', 10),
                     'method_parameters': {'ef_search': self.configuration['ef']}}
            if predicate is not None:
                field['filter'] = filtered
            query = {'knn': {'vec': field}}
        else:
            raise ValueError('unknown OpenSearch route')
        return encode({'size': 10, '_source': ['text', 'metadata'], 'track_total_hits': False,
                       'query': query, 'sort': [{'_score': 'desc'}, {'metadata.ordinal': 'asc'}]})

    def query(self, prepared):
        result = self.http.call('POST', '/idx/_search?request_cache=false', body=prepared)
        if result.get('timed_out') or result['_shards']['failed']:
            raise RuntimeError('incomplete OpenSearch response')
        return [{'id': hit['_id'], 'score': hit['_score'], **hit['_source']} for hit in result['hits']['hits']]

    def close(self):
        self.http.close()


class Search(OpenSearchKnn):
    def command(self):
        self.port, transport = self.ports(2)
        home = Path(os.environ['WARM_MATCHED_OPENSEARCH_HOME'])
        conf = self.workdir / 'config'
        if not conf.exists():
            shutil.copytree(home / 'config', conf)
        settings = {'path.data': self.data, 'path.logs': self.logs, 'http.port': self.port,
                    'transport.port': transport, 'network.host': '127.0.0.1',
                    'discovery.type': 'single-node', 'cluster.name': 'warm-matched',
                    'node.name': 'n1', 'plugins.security.disabled': 'true',
                    'bootstrap.memory_lock': 'false', 'node.processors': '4'}
        return [home / 'bin/opensearch', *(part for key, value in settings.items() for part in ('-E', f'{key}={value}'))], {
            'OPENSEARCH_PATH_CONF': str(conf), 'OPENSEARCH_JAVA_OPTS': '-Xms2g -Xmx2g',
            'LD_LIBRARY_PATH': str(home / 'plugins/opensearch-knn/lib') + ':' + str(home / 'plugins/opensearch-neural-search/lib')}, self.workdir

    def connect(self):
        self.http = Http(self.port, 7200)
        if self.http.call('GET', '/')['version']['number'] != '3.9.0':
            raise RuntimeError('unexpected OpenSearch version')
        self.http.call('PUT', '/_cluster/settings', {'persistent': {'search.concurrent_segment_search.mode': 'none'}})

    def close(self):
        if getattr(self, 'http', None) is not None:
            self.http.close()

    def create(self, params):
        self.http.call('PUT', '/idx', {'settings': {'index': {'knn': True, 'number_of_shards': 1,
            'number_of_replicas': 0, 'refresh_interval': '-1', 'translog.durability': 'request'}},
            'mappings': {'properties': {'vec': {'type': 'knn_vector', 'dimension': self.dim,
                'method': {'name': 'hnsw', 'engine': params['engine'], 'space_type': 'cosinesimil',
                           'parameters': {'m': params['m'], 'ef_construction': params['ef_construction']}}},
                'text': {'type': 'text', 'index': False},
                'metadata': {'properties': {'ordinal': {'type': 'integer'}, 'bucket100': {'type': 'integer'},
                    'eligible10': {'type': 'boolean'}, 'eligible1': {'type': 'boolean'}, 'epoch': {'type': 'integer'}}}}}})

    def ingest(self, train, params):
        for lo in range(0, len(train), 500):
            lines = []
            for i in range(lo, min(lo + 500, len(train))):
                doc = payload(i)
                lines += [json.dumps({'index': {'_index': 'idx', '_id': doc.pop('id')}}),
                          json.dumps({**doc, 'vec': train[i].tolist()}, allow_nan=False)]
            self.bulk(lines)

    def replace_one(self, ordinal, vector, epoch=1):
        doc = payload(ordinal, epoch)
        doc.pop('id')
        self.http.call('PUT', f'/idx/_doc/{ordinal}?refresh=true', {**doc, 'vec': vector.tolist()})

    def delete_one(self, ordinal):
        if self.http.call('DELETE', f'/idx/_doc/{ordinal}?refresh=true')['result'] != 'deleted':
            raise RuntimeError('deletion did not affect one row')

    def maintain(self, params):
        self.finish_index()
        self.warm()

    def inspect(self):
        return {'index': self.http.call('GET', '/idx/_settings?include_defaults=true'),
                'cluster': self.http.call('GET', '/_cluster/settings?include_defaults=true'),
                'segments': self.http.call('GET', '/idx/_segments')}

    def fetch(self, ordinal):
        try:
            result = self.http.call('GET', f'/idx/_doc/{ordinal}')
        except HttpError as error:
            if error.status == 404:
                return None
            raise
        return {'id': result['_id'], **result['_source']}


SYSTEMS = {'warm': Warm, 'pgvector': Pg, 'opensearch': Search}
CLIENTS = {'warm': WarmClient, 'pgvector': PgClient, 'opensearch': SearchClient}


def validate_hits(hits, rows, churn, filter_name):
    expected_count = 0 if filter_name == 'empty' else 10
    if not isinstance(hits, list) or len(hits) != expected_count:
        raise ValueError('incorrect hit count')
    seen = set()
    for hit in hits:
        identity = hit['id']
        if not isinstance(identity, str) or not identity.isascii() or not identity.isdigit() or len(identity) > 12:
            raise ValueError('invalid returned ID')
        ordinal = int(identity)
        if str(ordinal) != identity or not 0 <= ordinal < rows or ordinal in seen:
            raise ValueError('invalid/duplicate returned ID')
        if churn and 100 <= ordinal < 200:
            raise ValueError('deleted result')
        if filter_name == '10pct' and ordinal % 100 >= 10 or filter_name == '1pct' and ordinal % 100 != 0:
            raise ValueError('ineligible result')
        expected = payload(ordinal, int(churn and ordinal < 100))
        if any(hit.get(key) != expected[key] for key in ('text', 'metadata')):
            raise ValueError('payload mismatch')
        if type(hit['score']) not in (int, float) or not math.isfinite(hit['score']):
            raise ValueError('nonfinite returned score')
        seen.add(ordinal)
    return seen
