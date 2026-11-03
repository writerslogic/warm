# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import json
from pathlib import Path
import tempfile
import threading
import unittest

import numpy as np

from adapters import payload
from measure import audit_scores, partitions, payload_digest, query_order, run_cell
from verify_evidence import verify_cell


class Requests(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        with self.server.lock:
            self.server.requests += 1
        status = self.server.response_status
        hits = [{**payload(i), 'score': 1.} for i in range(10)]
        if self.server.bad_payload:
            hits[-1]['metadata']['epoch'] = 1
        if self.server.bad_score:
            hits[-1]['score'] = .2
        if self.server.bad_type:
            hits[-1]['metadata']['eligible10'] = 1
        result = {'matches': hits} if status == 200 else {'error': 'injected server failure'}
        if len(request['query_vector']) != 4:
            status = 400
        body = json.dumps(result).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class FixtureServer(ThreadingHTTPServer):
    request_queue_size = 32


class MeasureTests(unittest.TestCase):
    def test_order_and_partition_cover_every_query_once(self):
        for count in (999, 1000):
            order = query_order('fixture', 0, count)
            self.assertEqual(sorted(order), list(range(count)))
            self.assertEqual(order, query_order('fixture', 0, count))
            self.assertNotEqual(order, query_order('fixture', 1, count))
            for clients in (1, 8):
                slices = partitions(order, clients)
                self.assertEqual([i for group in slices for i in group], order)
                self.assertLessEqual(max(map(len, slices)) - min(map(len, slices)), 1)
        for count in (0, 1001, True):
            with self.assertRaises(ValueError):
                query_order('fixture', 0, count)
        for clients in (0, 7, 9, True):
            with self.assertRaises(ValueError):
                partitions(list(range(1000)), clients)
        with self.assertRaises(ValueError):
            partitions([0, 0], 1)

    def test_score_audit_checks_changed_vector_and_scale(self):
        train = np.array([[1., 0.], [0., 1.], [1., 1.]])
        query = np.array([0., 1.])
        for engine, score in [('warm', 0.), ('pgvector', 0.), ('opensearch', .5)]:
            self.assertEqual(audit_scores(engine, [{'id': '0', 'score': score}], query, train, False), 0.)
        self.assertEqual(audit_scores('warm', [{'id': '0', 'score': 1.}], query, train, True), 0.)
        with self.assertRaises(ValueError):
            audit_scores('warm', [{'id': '0', 'score': 0.}], query, train, True)
        for offset in (0.000099, 0.0001, 0.000101):
            hit = [{'id': '0', 'score': offset}]
            if offset <= 0.0001:
                audit_scores('warm', hit, query, train, False)
            else:
                with self.assertRaises(ValueError):
                    audit_scores('warm', hit, query, train, False)

    def test_payload_digest_preserves_types_but_ignores_object_key_order(self):
        expected = payload(0)
        reordered = {**expected, 'metadata': dict(reversed(list(expected['metadata'].items())))}
        self.assertEqual(payload_digest([expected]), payload_digest([reordered]))
        wrong = {**expected, 'metadata': {**expected['metadata'], 'eligible10': 1}}
        self.assertEqual(expected['metadata'], wrong['metadata'])
        self.assertNotEqual(payload_digest([expected]), payload_digest([wrong]))

    def measure(self, clients, status=200, bad_payload=False, bad_score=False, bad_type=False):
        server = FixtureServer(('127.0.0.1', 0), Requests)
        server.lock = threading.Lock()
        server.requests = 0
        server.response_status = status
        server.bad_payload, server.bad_score = bad_payload, bad_score
        server.bad_type = bad_type
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                folder = Path(directory)
                train = np.ones((1024, 4), dtype='<f4')
                train.tofile(folder / 'train.f32')
                queries = np.ones((16, 4), dtype='<f4')
                result = run_cell(engine='warm', port=server.server_port,
                    configuration={'route': 'exact'}, filter_name='all', dataset='fixture',
                    repetition=0, clients=clients, queries=queries, warmup=queries[:0],
                    expected=[{'ids': list(range(10))} for _ in queries],
                    train_path=folder / 'train.f32', shape=train.shape, churn=False,
                    output=folder / 'cell', limits={'startupSeconds': 30, 'querySeconds': 30,
                        'sampleSeconds': .05, 'maxServerPssBytes': 2**31, 'maxClientPssBytes': 2**31})
                records = [json.loads(line) for path in (folder / 'cell').glob('client-*.ndjson')
                           for line in path.read_text().splitlines()]
                self.assertEqual(sorted(row['query'] for row in records), list(range(16)))
                self.assertEqual(server.requests, 16, {'result': result,
                    'requestErrors': [row for row in records if row['error']]})
                self.assertEqual(result, json.loads((folder / 'cell/finished.json').read_text()))
                verification = verify_cell(folder / 'cell', expected=[{'ids': list(range(10))} for _ in queries],
                    queries=queries, train_path=folder / 'train.f32', shape=train.shape,
                    limits={'maxServerPssBytes': 2**31, 'maxClientPssBytes': 2**31})
                self.assertTrue(verification['evidenceVerified'])
                self.assertEqual(verification['qualityPassed'], result['pass'])
                for row in records:
                    self.assertGreaterEqual(row['sentAt'], result['commonStart'])
                    self.assertGreaterEqual(row['receivedAt'], row['sentAt'])
                    self.assertGreaterEqual(row['validatedAt'], row['receivedAt'])
                if result['pass']:
                    path = folder / 'cell/client-0.ndjson'
                    original = path.read_text()
                    for field, value in [('expected', [999]), ('payloadSha256', '0' * 64), ('seconds', 999.)]:
                        changed = [json.loads(line) for line in original.splitlines()]
                        changed[0][field] = value
                        path.write_text(''.join(json.dumps(row) + '\n' for row in changed))
                        altered_summary = json.loads((folder / 'cell/finished.json').read_text())
                        altered_summary['rawSha256'][path.name] = hashlib.sha256(path.read_bytes()).hexdigest()
                        (folder / 'cell/finished.json').write_text(json.dumps(altered_summary))
                        with self.assertRaises(ValueError, msg=field):
                            verify_cell(folder / 'cell', expected=[{'ids': list(range(10))} for _ in queries],
                                queries=queries, train_path=folder / 'train.f32', shape=train.shape,
                                limits={'maxServerPssBytes': 2**31, 'maxClientPssBytes': 2**31})
                    path.write_text(original)
                    (folder / 'cell/finished.json').write_text(json.dumps(result))
                return result
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_multiple_processes_share_start_and_do_not_duplicate_requests(self):
        for clients in (1, 8):
            result = self.measure(clients)
            self.assertTrue(result['pass'], result)
            self.assertEqual(len({row['pid'] for row in result['workers']}), clients)
            self.assertEqual(result['completed'], 16)
            self.assertAlmostEqual(result['queriesPerSecond'], 16 / result['elapsedSeconds'])
            self.assertTrue(result['developmentOnly'])

    def test_server_errors_are_retained_without_retries_or_success_credit(self):
        result = self.measure(8, status=503)
        self.assertTrue(result['complete'])
        self.assertFalse(result['pass'])
        self.assertEqual(result['errors'], 16)
        self.assertEqual(result['completed'], 0)
        self.assertEqual(result['queriesPerSecond'], 0.)

    def test_payload_and_score_failures_reject_the_cell(self):
        for kind in ('bad_payload', 'bad_score', 'bad_type'):
            result = self.measure(1, **{kind: True})
            self.assertFalse(result['pass'])
            self.assertEqual(result['errors'], 16)
            self.assertEqual(result['completed'], 0)


if __name__ == '__main__':
    unittest.main()
