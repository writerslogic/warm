// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { getEventListeners } from 'node:events'
import { setTimeout as delay } from 'node:timers/promises'
import test from 'node:test'
import { WarmClient, WarmError } from '../dist/index.js'
import { API_KEY, BYTE_LIMIT, deadline, deferred, httpFixture, json } from './support.mjs'

const localError = error => error instanceof TypeError || error instanceof RangeError
const hasCode = code => error => error instanceof WarmError && error.code === code
const stats = { tenant: 'fixture', documents: 0, vectors: 0, documentIndexTrained: false,
  vectorIndex: { rows: 0, graph: false, current: true, staleMutations: 0, rebuildRecommended: false },
  usedBytes: 0, capacityBytes: 0 }

test('HTTP requests retain a base prefix, isolate headers and encode document IDs once', async t => {
  const fixture = await httpFixture(t, (_req, res, seen) => {
    if (seen.url.endsWith('/stats')) json(res, stats)
    else if (seen.url.includes('/chunks/')) json(res, { revision: 'r', document: { id: 'doc', version: 'v', metadata: {}, chunkCount: 1 }, index: 0, id: 'chunk', startByte: 0, endByte: 0, text: null })
    else json(res, { deleted: true })
  })
  const first = new WarmClient({ url: `${fixture.url}/gateway/`, apiKey: API_KEY, tenantId: 'tenant_one' })
  const second = new WarmClient({ url: `${fixture.url}/gateway`, tenantId: 'tenant_two' })
  assert.deepEqual(await first.stats(), stats)
  await second.stats()
  assert.equal(fixture.requests[0].url, '/gateway/api/v1/stats')
  assert.equal(fixture.requests[0].headers.authorization, `Bearer ${API_KEY}`)
  assert.equal(fixture.requests[0].headers['x-tenant-id'], 'tenant_one')
  assert.equal(fixture.requests[1].headers.authorization, undefined)
  assert.equal(fixture.requests[1].headers['x-tenant-id'], 'tenant_two')
  for (const id of ['folder/doc #1?x=y%20z', '%2e', '%2e%2e', 'a/../b', 'café🙂']) {
    await first.deleteDocument(id)
    assert.equal(fixture.requests.at(-1).url, `/gateway/api/v1/documents/${encodeURIComponent(id)}`)
    await first.readDocumentChunk(id)
    assert.equal(fixture.requests.at(-1).url, `/gateway/api/v1/documents/${encodeURIComponent(id)}/chunks/0`)
  }
  const before = fixture.requests.length
  for (const id of ['.', '..']) {
    await assert.rejects(() => first.deleteDocument(id), localError)
    await assert.rejects(() => first.readDocumentChunk(id), localError)
  }
  for (const index of [-1, true, 0.5, '0', Number.MAX_SAFE_INTEGER + 1]) {
    await assert.rejects(() => first.readDocumentChunk('valid', index), localError)
  }
  assert.equal(fixture.requests.length, before)
})

test('base URLs cannot carry credentials, query secrets, fragments or non-HTTP schemes', () => {
  const secret = 'fixture-url-secret'
  for (const url of [
    `http://user:${secret}@127.0.0.1/`, `http://127.0.0.1/?key=${secret}`,
    `http://127.0.0.1/#${secret}`, 'file:///private/example', 'data:text/plain,example',
    'javascript:void(0)', '/relative',
  ]) {
    assert.throws(() => new WarmClient({ url }), error => localError(error) && !String(error).includes(secret))
  }
  for (const timeoutMs of [0, -1, NaN, Infinity, 0.5, '100']) {
    assert.throws(() => new WarmClient({ timeoutMs }), localError)
  }
  for (const tenantId of ['', 'a/b', 'tenant\r\nx-other: leaked', 'x'.repeat(65)]) {
    assert.throws(() => new WarmClient({ tenantId }), localError)
  }
  assert.throws(() => new WarmClient({ apiKey: 'fixture\r\nx-other: leaked' }), localError)
})

test('redirects never forward credentials or replay writes to another origin', async t => {
  const destination = await httpFixture(t, (_req, res) => json(res, stats))
  let status = 301
  const origin = await httpFixture(t, (_req, res) => {
    res.writeHead(status, { location: `${destination.url}/credentials-must-not-arrive` })
    res.end()
  })
  const client = new WarmClient({ url: origin.url, apiKey: API_KEY, tenantId: 'fixture' })
  for (status of [301, 302, 303, 307, 308]) {
    await assert.rejects(() => client.memory.remember('Redirect must not replay this mutation'), hasCode('redirect_error'))
  }
  assert.equal(origin.requests.length, 5)
  assert.equal(destination.requests.length, 0)
})

test('core and memory payloads follow their different wire conventions and maintenance defaults', async t => {
  const fixture = await httpFixture(t, (_req, res, seen) => {
    if (seen.url.endsWith('/documents/batch')) json(res, { added: 1 })
    else if (seen.url.endsWith('/query')) json(res, { matches: [] })
    else if (seen.url.endsWith('/consolidate')) json(res, { created: [], forgotten: [], dryRun: true })
    else if (seen.url.endsWith('/remember')) json(res, { remembered: [{ id: 'fact', created: true, supersedes: [], supersededBy: [], ingestTime: 1000 }] })
    else json(res, { forgotten: [], dryRun: JSON.parse(seen.body).dryRun, purged: false })
  })
  const client = new WarmClient({ url: fixture.url })
  const identity = 'a'.repeat(64)
  await client.addDocuments([{ id: 'doc', text: null, vector: [1], metadata: { warm_import_identity: identity },
    importIdentity: identity, importAdoptLegacy: true }])
  assert.deepEqual(JSON.parse(fixture.requests.at(-1).body), [{ id: 'doc', text: null, vector: [1], metadata: { warm_import_identity: identity },
    import_identity: identity, import_adopt_legacy: true }])
  await client.addDocuments([{ id: 'nullable', vector: [1], metadata: null }])
  assert.deepEqual(JSON.parse(fixture.requests.at(-1).body), [{ id: 'nullable', vector: [1], metadata: null }])
  await client.query({ queryVector: [1], documentIds: ['doc'], filter: { tag: 'kept' }, index: 'exact', ef: 1 })
  assert.deepEqual(JSON.parse(fixture.requests.at(-1).body), {
    query_vector: [1], top_k: 3, document_ids: ['doc'], filter: { tag: 'kept' }, index: 'exact', ef: 1,
  })
  await client.memory.remember('Fact', { id: 'fact', validFrom: -1000, validTo: null, sourceUri: null })
  assert.deepEqual(JSON.parse(fixture.requests.at(-1).body), {
    text: 'Fact', id: 'fact', validFrom: -1000, validTo: null, sourceUri: null,
  })
  await client.memory.forget({ kind: 'ids', ids: ['fact'] })
  assert.deepEqual(JSON.parse(fixture.requests.at(-1).body), {
    scope: { kind: 'ids', ids: ['fact'] }, includeChain: false, dryRun: false, purge: false,
  })
  await client.memory.applyForgetting()
  assert.deepEqual(JSON.parse(fixture.requests.at(-1).body), { includeChain: false, dryRun: true, purge: false })
  await client.memory.consolidate()
  assert.deepEqual(JSON.parse(fixture.requests.at(-1).body), { dryRun: true, allowDestructive: false })
})

test('invalid JSON values fail before a network request, including nested metadata and vectors', async t => {
  const fixture = await httpFixture(t, (_req, res) => json(res, { added: 1 }))
  const client = new WarmClient({ url: fixture.url })
  const circular = {}
  circular.self = circular
  for (const value of [NaN, Infinity, -Infinity, 1n, circular]) {
    await assert.rejects(() => client.addDocuments([{ id: 'doc', vector: [1], metadata: { nested: [value] } }]), localError)
    await assert.rejects(() => client.memory.remember('Fact', { metadata: { nested: [value] } }), localError)
    await assert.rejects(() => client.query({ queryVector: [1], filter: { nested: [value] } }), localError)
  }
  for (const queryVector of [[NaN], [Infinity], [-Infinity], [], [0, 0]]) {
    await assert.rejects(() => client.query({ queryVector }), localError)
  }
  for (const validFrom of [NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1, '1000', new Date(0)]) {
    await assert.rejects(() => client.memory.remember('Fact', { validFrom }), localError)
  }
  assert.equal(fixture.requests.length, 0)
})

test('UTF-8 ID, text and metadata limits reject only beyond the declared byte boundary', async t => {
  const fixture = await httpFixture(t, (_req, res, seen) => {
    json(res, seen.method === 'DELETE' ? { deleted: true } : { added: 1 })
  })
  const client = new WarmClient({ url: fixture.url })
  const sized = bytes => 'é'.repeat(Math.floor(bytes / 2)) + (bytes % 2 ? 'x' : '')
  for (const bytes of [255, 256]) await client.deleteDocument(sized(bytes))
  await assert.rejects(() => client.deleteDocument(sized(257)), localError)
  for (const bytes of [65535, 65536]) {
    await client.addDocuments([{ id: 'text', text: sized(bytes), vector: [1] }])
    await client.addDocuments([{ id: 'metadata', vector: [1], metadata: { pad: sized(bytes - 10) } }])
  }
  const count = fixture.requests.length
  await assert.rejects(() => client.addDocuments([{ id: 'text', text: sized(65537), vector: [1] }]), localError)
  await assert.rejects(() => client.addDocuments([{ id: 'metadata', vector: [1], metadata: { pad: sized(65537 - 10) } }]), localError)
  assert.equal(fixture.requests.length, count)
})

test('serialized request limits cover limit−1, limit and limit+1 without sending oversized data', { timeout: 30000 }, async t => {
  const fixture = await httpFixture(t, (_req, res) => json(res, { added: 256 }))
  const client = new WarmClient({ url: fixture.url })
  for (const bytes of [BYTE_LIMIT - 1, BYTE_LIMIT, BYTE_LIMIT + 1]) {
    const documents = Array.from({ length: 256 }, (_, i) => ({ id: `doc-${i}`, text: i < 255 ? 'x'.repeat(65536) : '', vector: [1] }))
    const remaining = bytes - Buffer.byteLength(JSON.stringify(documents))
    assert.ok(remaining >= 0 && remaining <= 65536)
    documents[255].text = 'x'.repeat(remaining)
    if (bytes <= BYTE_LIMIT) {
      await client.addDocuments(documents)
      assert.equal(Buffer.byteLength(fixture.requests.at(-1).body), bytes)
    } else {
      await assert.rejects(() => client.addDocuments(documents), hasCode('request_too_large'))
    }
  }
  assert.equal(fixture.requests.length, 2)
})

test('structured service errors retain code/status/request ID but redact credentials', async t => {
  const secret = 'fixture.token+[secret]/%?='
  const fixture = await httpFixture(t, (_req, res) => json(res, {
    type: 'https://example.invalid/problems/quota_exceeded', status: 429, title: 'Tenant quota exceeded',
    code: 'quota_exceeded', detail: `Denied credential ${secret}`, request_id: 'fixture-request-42',
    error: { code: 'quota_exceeded', message: `Denied credential ${secret}` },
  }, 429, { 'content-type': 'application/problem+json', 'x-request-id': 'fixture-request-42' }))
  const client = new WarmClient({ url: fixture.url, apiKey: secret })
  await assert.rejects(() => client.stats(), error => {
    assert.ok(error instanceof WarmError)
    assert.equal(error.status, 429)
    assert.equal(error.statusCode, 429)
    assert.equal(error.code, 'quota_exceeded')
    assert.equal(error.requestId, 'fixture-request-42')
    assert.ok(!String(error).includes(secret))
    assert.ok(!JSON.stringify(error).includes(secret))
    assert.equal('response' in error, false)
    assert.equal('cause' in error, false)
    return true
  })
})

test('legacy service errors parse while HTML errors and malformed successful JSON stay bounded', async t => {
  let mode = 'legacy'
  const fixture = await httpFixture(t, (_req, res) => {
    if (mode === 'legacy') json(res, { error: { code: 'not_found', message: 'fixture missing' } }, 404)
    else if (mode === 'html') {
      res.writeHead(502, { 'content-type': 'text/html' })
      res.end('<html>fixture-private-upstream-body</html>')
    } else {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end('{malformed')
    }
  })
  const client = new WarmClient({ url: fixture.url })
  await assert.rejects(() => client.stats(), error => error instanceof WarmError && error.status === 404 && error.code === 'not_found')
  mode = 'html'
  await assert.rejects(() => client.stats(), error => error instanceof WarmError && error.status === 502 && !String(error).includes('fixture-private-upstream-body'))
  mode = 'invalid'
  await assert.rejects(() => client.stats(), hasCode('invalid_response'))
})

test('documented null and absent fact fields remain distinct, and wrong response shapes reject', async t => {
  const fact = { id: 'fact', status: 'current', effectiveFrom: 1000, entities: [], supersedes: [], supersededBy: [], metadata: {} }
  let body = { fact }
  const fixture = await httpFixture(t, (_req, res) => json(res, body))
  const client = new WarmClient({ url: fixture.url })
  const absent = await client.memory.get('fact')
  assert.deepEqual(absent, fact)
  assert.equal(Object.hasOwn(absent, 'text'), false)
  body = { fact: { ...fact, text: null, validTo: null } }
  assert.deepEqual(await client.memory.get('fact'), body.fact)
  body = { fact: null }
  assert.equal(await client.memory.get('missing'), null)
  for (body of [{}, { fact: { ...fact, effectiveFrom: '1000' } }, { fact: [] }]) {
    await assert.rejects(() => client.memory.get('fact'), hasCode('invalid_response'))
  }
})

test('response byte limit accepts exact valid JSON and cancels a chunked overrun before EOF', { timeout: 30000 }, async t => {
  let size = BYTE_LIMIT - 1
  const fixture = await httpFixture(t, (_req, res) => {
    const prefix = '{"matches":[{"id":"doc","score":1,"metadata":{},"text":"'
    const suffix = '"}]}'
    const body = prefix + 'x'.repeat(size - prefix.length - suffix.length) + suffix
    res.writeHead(200, { 'content-type': 'application/json', 'content-length': String(size) })
    res.end(body)
  })
  const client = new WarmClient({ url: fixture.url })
  for (size of [BYTE_LIMIT - 1, BYTE_LIMIT]) {
    const result = await client.query({ queryVector: [1] })
    assert.equal(Buffer.byteLength(JSON.stringify(result)), size)
  }
  size = BYTE_LIMIT + 1
  await assert.rejects(() => client.query({ queryVector: [1] }), hasCode('response_too_large'))

  const closed = deferred()
  const stream = await httpFixture(t, (_req, res) => {
    res.once('close', () => closed.resolve(res.writableEnded))
    res.writeHead(200, { 'content-type': 'application/json' })
    res.write('"' + 'x'.repeat(BYTE_LIMIT))
  })
  const controller = new AbortController()
  const streamed = new WarmClient({ url: stream.url, timeoutMs: 5000 })
  await assert.rejects(() => streamed.stats({ signal: controller.signal }), hasCode('response_too_large'))
  assert.equal(await deadline(closed.promise, 2000, 'oversized response cancellation'), false)
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0)
})

test('timeouts cover headers and streaming bodies and release caller abort listeners', { timeout: 10000 }, async t => {
  for (const sendHeaders of [false, true]) {
    await t.test(sendHeaders ? 'streaming body' : 'headers', async t => {
      const closed = deferred()
      const fixture = await httpFixture(t, (_req, res) => {
        res.once('close', () => closed.resolve(res.writableEnded))
        if (sendHeaders) {
          res.writeHead(200, { 'content-type': 'application/json' })
          res.write('{"tenant":')
        }
      })
      const controller = new AbortController()
      const client = new WarmClient({ url: fixture.url, timeoutMs: 150 })
      await assert.rejects(() => client.stats({ signal: controller.signal }), hasCode('timeout'))
      assert.equal(await deadline(closed.promise, 2000, 'timed-out connection cancellation'), false)
      assert.equal(controller.signal.aborted, false)
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0)
    })
  }
})

test('caller cancellation is distinct from timeout and already-aborted calls send nothing', async t => {
  const started = deferred()
  const closed = deferred()
  const fixture = await httpFixture(t, (_req, res) => {
    res.once('close', () => closed.resolve())
    started.resolve()
  })
  const client = new WarmClient({ url: fixture.url, timeoutMs: 5000 })
  const controller = new AbortController()
  const pending = client.stats({ signal: controller.signal })
  const rejected = assert.rejects(pending, hasCode('aborted'))
  await deadline(started.promise, 2000, 'request start')
  controller.abort(new Error('fixture caller cancellation reason'))
  await rejected
  await deadline(closed.promise, 2000, 'aborted connection cancellation')
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0)
  await assert.rejects(() => client.stats({ signal: controller.signal }), hasCode('aborted'))
  assert.equal(fixture.requests.length, 1)
})

test('successful and failed calls clear deadline timers and caller listeners', async t => {
  let status = 200
  const fixture = await httpFixture(t, (_req, res) => json(res, status === 200 ? stats : { error: { code: 'not_found', message: 'missing' } }, status))
  const internalSignals = []
  const client = new WarmClient({ url: fixture.url, timeoutMs: 400, fetch: (url, options) => {
    internalSignals.push(options.signal)
    return fetch(url, options)
  } })
  const controller = new AbortController()
  await client.stats({ signal: controller.signal })
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0)
  status = 404
  await assert.rejects(() => client.stats({ signal: controller.signal }), hasCode('not_found'))
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0)
  await delay(450)
  assert.ok(internalSignals.every(signal => !signal.aborted), 'completed requests retained an active deadline timer')
})

test('a dropped connection reports a transport error without replaying a mutation', async t => {
  const fixture = await httpFixture(t, (req, _res) => req.socket.destroy())
  const client = new WarmClient({ url: fixture.url, apiKey: API_KEY })
  await assert.rejects(() => client.memory.remember('Do not replay this write'), error =>
    error instanceof WarmError && error.code === 'network_error' && error.status === undefined && !String(error).includes(API_KEY))
  assert.equal(fixture.requests.length, 1)
})
