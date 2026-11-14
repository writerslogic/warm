# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""A WARM backend for Mem0's own harness (mem0ai/memory-benchmarks, benchmarks/*/run.py).

`WarmClient` has the interface of benchmarks.common.mem0_client.Mem0Client (add, search,
delete_user, close, async context manager), so Mem0's run.py, answerer prompt, judge prompt, top-k
cutoffs and scoring run unchanged; only the memory system behind `add` and `search` differs.
`run_mem0_harness.py` installs it in place of Mem0Client.

Ingest: every message of a chunk is stored as one fact (`"{speaker}: {text}"`, the harness's own
rendering, image tags included), with the session epoch as event time and its date as metadata. No LLM call.
Search: one recall of `top_k` over the user's tenant. Each result is {memory, score, id, created_at}, the shape
format_search_results reads. The answerer sees at most ANSWERER_MEMORY_LIMIT (200) memories, sorted by created_at.

Embeddings (hybrid recall) come from an OpenAI-compatible endpoint given by WARM_EMBED_BASE_URL, with the key in
the env var named by WARM_EMBED_API_KEY_ENV; point it at stub_llm.py to run for free. The server is either an
existing warm-server (WARM_HOST, which must allow --max-top-k of at least the harness top-k, default 200) or one started from WARM_SERVER_BIN.
"""

import asyncio
import time
import hashlib
import json
import os
import re
import subprocess
import tempfile
import urllib.request
from datetime import datetime, timezone

EMBED_MODEL = os.environ.get('WARM_EMBED_MODEL', 'text-embedding-3-small')
EMBED_DIM = int(os.environ.get('WARM_EMBED_DIM', '1536'))


def _post(url, body, headers, timeout=300):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method='POST', headers={'Content-Type': 'application/json', **headers})
    for attempt in range(6):
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code not in (429, 502, 503, 504) or attempt == 5:
                raise
            time.sleep(2 ** attempt)


class WarmClient:
    def __init__(self, mode='warm', host=None, api_key=None, **_ignored):
        self.embed_base = os.environ['WARM_EMBED_BASE_URL'].rstrip('/')
        self.embed_key = os.environ.get(os.environ.get('WARM_EMBED_API_KEY_ENV', 'OPENAI_API_KEY'), 'unset')
        self.proc = None
        self.host = (os.environ.get('WARM_HOST') or '').rstrip('/')
        if not self.host:
            self.dir = tempfile.mkdtemp(prefix='locomo-warm-')
            self.proc = subprocess.Popen([os.environ['WARM_SERVER_BIN'], '--no-auth', '--bind', '127.0.0.1:0', '--data-dir',
                                          os.path.join(self.dir, 'data'), '--input-dim', str(EMBED_DIM), '--max-top-k', os.environ.get('WARM_MAX_TOP_K', '200'),
                                          '--max-queue', os.environ.get('WARM_MAX_QUEUE', '1000')],
                                         stdout=subprocess.PIPE, stderr=open(os.path.join(self.dir, 'server.log'), 'wb'))
            m = re.search(r'listening on (\S+)', self.proc.stdout.readline().decode())
            if not m:
                raise RuntimeError('warm-server did not start')
            self.host = f'http://{m.group(1)}'
        self.counters = {'embed_calls': 0, 'embed_inputs': 0}

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        await self.close()

    async def close(self):
        if self.proc:
            self.proc.terminate()
            self.proc.wait(timeout=60)
            self.proc = None

    def _embed(self, texts):
        out = []
        for s in range(0, len(texts), 256):
            part = texts[s:s + 256]
            r = _post(f'{self.embed_base}/embeddings', {'model': EMBED_MODEL, 'input': part, 'dimensions': EMBED_DIM}
                      if 'stub' in self.embed_base or '127.0.0.1' in self.embed_base else {'model': EMBED_MODEL, 'input': part},
                      {'Authorization': f'Bearer {self.embed_key}'})
            self.counters['embed_calls'] += 1
            self.counters['embed_inputs'] += len(part)
            out.extend(d['embedding'] for d in sorted(r['data'], key=lambda d: d['index']))
        return out

    def _memory(self, tenant, op, body):
        return _post(f'{self.host}/api/v1/memory/{op}', body, {'X-Tenant-ID': tenant})

    async def add(self, messages, user_id, observation_date=None, timestamp=None, custom_instructions=None, metadata=None):
        facts = []
        for m in messages:
            text = m['content']
            fid = 'w:' + hashlib.sha256(f'{user_id}\x1f{timestamp}\x1f{text}'.encode()).hexdigest()[:24]
            facts.append({'id': fid, 'text': text, 'eventTime': int(timestamp or 0) * 1000,
                          'metadata': {'timestamp': str(timestamp or '')}})
        embs = await asyncio.to_thread(self._embed, [f['text'] for f in facts])
        for f, e in zip(facts, embs):
            f['embedding'] = e
        await asyncio.to_thread(self._memory, user_id, 'remember', {'facts': facts})
        return {'results': [{'id': f['id'], 'memory': f['text'], 'event': 'ADD'} for f in facts]}

    async def search(self, query, user_id, top_k=200, rerank=False, score_debug=False):
        emb = (await asyncio.to_thread(self._embed, [query]))[0]
        r = await asyncio.to_thread(self._memory, user_id, 'recall', {'query': query, 'k': top_k, 'embedding': emb})
        out = []
        for h in r['results']:
            created = ''
            if h.get('eventTime'):
                created = datetime.fromtimestamp(h['eventTime'] / 1000, tz=timezone.utc).isoformat()
            out.append({'id': h['id'], 'memory': h['text'], 'score': h.get('score', 0.0), 'created_at': created})
        return {'results': out}

    async def delete_user(self, user_id):
        return True
