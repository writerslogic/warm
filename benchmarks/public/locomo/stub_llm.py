# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""A local OpenAI-compatible stub for the LoCoMo head-to-head adapters. It costs nothing.

It answers /v1/chat/completions and /v1/embeddings without any model, records every call
(endpoint, kind, prompt tokens in o200k, completion tokens) and writes the tally to --stats on
shutdown. Chat replies are synthetic but well-formed, so the systems under test run their whole
write and read paths:

* /v1/responses with a json_schema format, and chat response_format json_schema (Graphiti): an object built from the schema, arrays of objects
  filled with `--items` elements (integer fields take the element index).
* response_format json_object (mem0ai): {"memory": [...]} with `--items` synthetic facts.
* chat with logprobs (Graphiti's cross-encoder): a "True" token with a fixed logprob.
* anything else: the text "stub".

The tally counts what the stub's synthetic outputs cause. Real model output changes how many
entities and edges are extracted, so call counts are a lower bound; prompt sizes per call are exact
for the prompts the systems build.
"""

import argparse
import hashlib
import json
import math
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import tiktoken

ENC = tiktoken.get_encoding('o200k_base')


def fill(schema, defs, items, idx=0, depth=0):
    if '$ref' in schema:
        return fill(defs[schema['$ref'].split('/')[-1]], defs, items, idx, depth)
    for key in ('anyOf', 'oneOf'):
        if key in schema:
            options = [o for o in schema[key] if o.get('type') != 'null'] or schema[key]
            return fill(options[0], defs, items, idx, depth)
    if 'enum' in schema:
        return schema['enum'][0]
    kind = schema.get('type')
    if kind == 'object' or 'properties' in schema:
        return {k: fill(v, defs, items, idx, depth + 1) for k, v in schema.get('properties', {}).items()}
    if kind == 'array':
        item = schema.get('items', {})
        n = items if depth < 3 else 0
        return [fill(item, defs, items, i, depth + 1) for i in range(n)]
    if kind == 'integer':
        return idx
    if kind == 'number':
        return 0.5
    if kind == 'boolean':
        return False
    return f'stub {idx}'


class Tally:
    def __init__(self):
        self.lock = threading.Lock()
        self.calls = []

    def add(self, **row):
        with self.lock:
            self.calls.append(row)

    def summary(self):
        out = {}
        with self.lock:
            for c in self.calls:
                s = out.setdefault(c['key'], {'calls': 0, 'prompt_tokens': 0, 'completion_tokens': 0, 'max_prompt_tokens': 0})
                s['calls'] += 1
                s['prompt_tokens'] += c['prompt_tokens']
                s['completion_tokens'] += c['completion_tokens']
                s['max_prompt_tokens'] = max(s['max_prompt_tokens'], c['prompt_tokens'])
        return out


def make_handler(tally, items):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def reply(self, obj, code=200):
            data = json.dumps(obj).encode()
            self.send_response(code)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            self.reply(tally.summary() if self.path == '/stats' else {'ok': True})

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))))
            if self.path.endswith('/embeddings'):
                texts = body['input'] if isinstance(body['input'], list) else [body['input']]
                dim = int(body.get('dimensions') or 1536)
                toks = sum(len(ENC.encode(t)) for t in texts if isinstance(t, str))
                tally.add(key='embeddings', prompt_tokens=toks, completion_tokens=0)
                self.reply({'object': 'list', 'model': body.get('model', 'stub'), 'usage': {'prompt_tokens': toks, 'total_tokens': toks},
                            'data': [{'object': 'embedding', 'index': i, 'embedding': vec(t, dim)} for i, t in enumerate(texts)]})
                return
            if self.path.endswith('/responses'):
                # Graphiti's OpenAIClient uses responses.parse() for structured output
                inp = body.get('input', [])
                prompt = inp if isinstance(inp, str) else '\n'.join(m['content'] if isinstance(m['content'], str) else json.dumps(m['content']) for m in inp)
                fmt = (body.get('text') or {}).get('format') or {}
                sch = fmt.get('schema')
                content = json.dumps(fill(sch, sch.get('$defs', {}), items)) if sch else 'stub'
                first = prompt[:48].replace('\n', ' ')
                ptoks, ctoks = len(ENC.encode(prompt)), len(ENC.encode(content))
                tally.add(key=f'responses:{"json_schema" if sch else "text"}:{first}', prompt_tokens=ptoks, completion_tokens=ctoks)
                self.reply({'id': 'stub', 'object': 'response', 'created_at': 0, 'model': body.get('model', 'stub'), 'status': 'completed',
                            'parallel_tool_calls': False, 'tool_choice': 'auto', 'tools': [],
                            'output': [{'type': 'message', 'id': 'm', 'role': 'assistant', 'status': 'completed',
                                        'content': [{'type': 'output_text', 'text': content, 'annotations': []}]}],
                            'usage': {'input_tokens': ptoks, 'output_tokens': ctoks, 'total_tokens': ptoks + ctoks,
                                      'input_tokens_details': {'cached_tokens': 0}, 'output_tokens_details': {'reasoning_tokens': 0}}})
                return
            msgs = body.get('messages', [])
            prompt = '\n'.join(m['content'] if isinstance(m['content'], str) else json.dumps(m['content']) for m in msgs)
            rf = (body.get('response_format') or {}).get('type')
            first = (msgs[0]['content'] if msgs and isinstance(msgs[0]['content'], str) else '')[:48].replace('\n', ' ')
            if rf == 'json_schema':
                sch = body['response_format']['json_schema']['schema']
                content = json.dumps(fill(sch, sch.get('$defs', {}), items))
            elif rf == 'json_object':
                n = int(hashlib.sha256(prompt.encode()).hexdigest()[:2], 16) % 3 + 1 if items else 0
                facts = [{'id': str(i), 'text': f'stub memory {hashlib.sha256((prompt + str(i)).encode()).hexdigest()[:8]}',
                          'attributed_to': 'user', 'linked_memory_ids': []} for i in range(min(n, items))]
                content = json.dumps({'memory': facts, 'facts': [f['text'] for f in facts]})
            else:
                content = 'stub'
            ptoks, ctoks = len(ENC.encode(prompt)), len(ENC.encode(content))
            tally.add(key=f'chat:{rf or "text"}:{"logprobs" if body.get("logprobs") else "plain"}:{first}', prompt_tokens=ptoks, completion_tokens=ctoks)
            msg = {'role': 'assistant', 'content': content}
            choice = {'index': 0, 'finish_reason': 'stop', 'message': msg}
            if body.get('logprobs'):
                choice['logprobs'] = {'content': [{'token': 'True', 'logprob': math.log(0.9), 'bytes': [84], 'top_logprobs': [
                    {'token': 'True', 'logprob': math.log(0.9), 'bytes': [84]}, {'token': 'False', 'logprob': math.log(0.1), 'bytes': [70]}]}]}
            self.reply({'id': 'stub', 'object': 'chat.completion', 'created': 0, 'model': body.get('model', 'stub'), 'choices': [choice],
                        'usage': {'prompt_tokens': ptoks, 'completion_tokens': ctoks, 'total_tokens': ptoks + ctoks}})
    return Handler


def vec(text, dim):
    """Deterministic unit vector from the text hash."""
    out, i = [], 0
    while len(out) < dim:
        h = hashlib.sha256(f'{i}\x1f{text}'.encode()).digest()
        out.extend((b - 127.5) / 127.5 for b in h)
        i += 1
    out = out[:dim]
    norm = math.sqrt(sum(x * x for x in out)) or 1.0
    return [x / norm for x in out]


def start(items=2, port=0):
    """Start in a thread; returns (server, tally, base_url)."""
    tally = Tally()
    server = ThreadingHTTPServer(('127.0.0.1', port), make_handler(tally, items))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, tally, f'http://127.0.0.1:{server.server_address[1]}/v1'


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=0)
    ap.add_argument('--items', type=int, default=2)
    ap.add_argument('--stats', default=None)
    args = ap.parse_args()
    srv, tally, url = start(args.items, args.port)
    print(f'stub listening on {url}', flush=True)
    try:
        threading.Event().wait()
    except KeyboardInterrupt:
        pass
    finally:
        if args.stats:
            with open(args.stats, 'w') as f:
                json.dump(tally.summary(), f, indent=2)
