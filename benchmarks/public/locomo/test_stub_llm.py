# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import json
import urllib.request

import stub_llm


def post(url, path, body):
    req = urllib.request.Request(url + path, data=json.dumps(body).encode(), method='POST', headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())


def test_schema_fill_follows_the_schema_and_counts_prompt_tokens():
    server, tally, url = stub_llm.start(items=2)
    try:
        schema = {'type': 'object', 'properties': {'extracted_entities': {'type': 'array', 'items': {'$ref': '#/$defs/E'}}},
                  '$defs': {'E': {'type': 'object', 'properties': {'name': {'type': 'string'}, 'entity_type_id': {'type': 'integer'}}}}}
        r = post(url, '/chat/completions', {'model': 'm', 'messages': [{'role': 'user', 'content': 'hello world'}],
                                            'response_format': {'type': 'json_schema', 'json_schema': {'schema': schema}}})
        body = json.loads(r['choices'][0]['message']['content'])
        assert [e['entity_type_id'] for e in body['extracted_entities']] == [0, 1]
        r = post(url, '/responses', {'model': 'm', 'input': [{'role': 'user', 'content': 'hello world'}],
                                     'text': {'format': {'type': 'json_schema', 'schema': schema}}})
        assert json.loads(r['output'][0]['content'][0]['text']) == body
        stats = tally.summary()
        assert sum(s['calls'] for s in stats.values()) == 2
        assert all(s['prompt_tokens'] >= 2 for s in stats.values())
    finally:
        server.shutdown()


def test_embeddings_are_deterministic_unit_vectors_of_the_requested_size():
    server, tally, url = stub_llm.start()
    try:
        a = post(url, '/embeddings', {'model': 'm', 'input': ['x', 'y'], 'dimensions': 8})['data']
        b = post(url, '/embeddings', {'model': 'm', 'input': ['x'], 'dimensions': 8})['data']
        assert a[0]['embedding'] == b[0]['embedding'] != a[1]['embedding']
        assert len(a[0]['embedding']) == 8 and abs(sum(v * v for v in a[0]['embedding']) - 1) < 1e-9
    finally:
        server.shutdown()
