# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

import asyncio
import hashlib
import json
import os
from pathlib import Path
import tempfile


async def main():
    binary = Path(os.environ['WARM_MCP_BIN']).resolve()
    evidence = Path(__file__).resolve().parent
    env = {k: v for k, v in os.environ.items() if not k.startswith(('WARM_MCP_', 'HMS_MCP_'))}
    findings = []
    with tempfile.TemporaryDirectory(prefix='warm-export-cap-') as directory:
        with (evidence / 'export-cap-server.log').open('wb') as stderr:
            process = await asyncio.create_subprocess_exec(
                str(binary), '--data-dir', directory, '--dim', '256', '--max-export-bytes', '1024',
                env=env, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=stderr,
            )
            request_id = 0

            async def request(method, params):
                nonlocal request_id
                request_id += 1
                message = dict(jsonrpc='2.0', id=request_id, method=method, params=params)
                process.stdin.write(json.dumps(message).encode() + b'\n')
                await process.stdin.drain()
                while True:
                    line = await asyncio.wait_for(process.stdout.readline(), 20)
                    assert line, 'server exited before response'
                    reply = json.loads(line)
                    if reply.get('id') == request_id:
                        assert 'error' not in reply, reply
                        return reply['result']

            try:
                await request('initialize', dict(protocolVersion='2025-03-26', capabilities={},
                    clientInfo=dict(name='local-export-cap-probe', version='1')))
                process.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
                await process.stdin.drain()
                remembered = await request('tools/call', dict(name='remember', arguments=dict(
                    id='large', text='word ' * 600)))
                assert not remembered.get('isError', False), remembered
                for format in ('json', 'jsonl', 'markdown'):
                    result = await request('tools/call', dict(name='export', arguments=dict(format=format)))
                    assert not result.get('isError', False), result
                    output = result['structuredContent']
                    texts = [item['text'] for item in result['content'] if item['type'] == 'text']
                    assert len(texts) == 1, result
                    payload_bytes = len(texts[0].encode('utf-8'))
                    assert payload_bytes == output['bytes'], result
                    assert output['count'] == 1 and output['bytes'] > 1024, output
                    findings.append(dict(format=format, configured_cap=1024, reported_payload_bytes=output['bytes'],
                        measured_payload_bytes=payload_bytes, count=output['count'], is_error=result.get('isError', False)))
            finally:
                process.stdin.close()
                try:
                    await asyncio.wait_for(process.wait(), 10)
                except TimeoutError:
                    process.kill()
                    await process.wait()
    result = dict(binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        source='6174370d3ba03c31700efa92bb5d6f6141b96713', findings=findings,
        scope='Local stdio baseline: one 3000-byte fixture, no models or providers; reproduces oversized successful exports.')
    (evidence / 'export-cap-baseline.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


asyncio.run(main())
