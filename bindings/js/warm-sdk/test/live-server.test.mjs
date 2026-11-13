// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { setTimeout as delay } from 'node:timers/promises'
import test from 'node:test'
import { WarmClient, WarmError } from '../dist/index.js'
import { API_KEY, ids, vector, warmServer } from './support.mjs'

const serviceError = (status, code) => error => error instanceof WarmError &&
  error.status === status && error.statusCode === status && error.code === code &&
  typeof error.requestId === 'string' && error.requestId.length > 0

test('portable client against the actual warm-server HTTP API', { timeout: 60000 }, async t => {
  const server = await warmServer(t)
  const client = tenantId => new WarmClient({ ...server, tenantId })

  await t.test('document query defaults, filtering, maintenance, deletion and service errors', async () => {
    const warm = client('documents')
    const documents = Array.from({ length: 6 }, (_, i) => ({
      id: `doc-${i}`, text: `Document ${i}`, vector: vector(i), metadata: { even: i % 2 === 0 },
    }))
    assert.deepEqual(await warm.addDocuments(documents), { added: 6 })
    assert.deepEqual(await warm.addDocuments([]), { added: 0 })
    const initial = await warm.query({ queryVector: vector(0) })
    assert.equal(initial.matches.length, 3)
    assert.equal(initial.matches[0].id, 'doc-0')
    assert.deepEqual(initial.matches[0].metadata, { even: true })
    assert.ok(initial.matches.every((match, i, all) => i === 0 || all[i - 1].score >= match.score))
    const filtered = await warm.query({ queryVector: vector(0), topK: 6, filter: { even: true }, index: 'exact' })
    assert.deepEqual(ids(filtered.matches), ['doc-0', 'doc-2', 'doc-4'])
    const restricted = await warm.query({ queryVector: vector(0), topK: 6, documentIds: ['doc-1', 'doc-3'] })
    assert.deepEqual(ids(restricted.matches), ['doc-1', 'doc-3'])
    const before = await warm.stats()
    assert.equal(before.tenant, 'documents')
    assert.equal(before.documents, 6)
    assert.equal(before.vectors, 6)
    const maintained = await warm.maintainDocumentIndex()
    assert.equal(typeof maintained.documentIndexTrained, 'boolean')
    assert.equal(typeof maintained.vectorIndex.current, 'boolean')
    assert.equal((await warm.query({ queryVector: vector(0), topK: 1 })).matches[0].id, 'doc-0')
    assert.deepEqual(await warm.deleteDocument('doc-0'), { deleted: true })
    assert.ok(!(await warm.query({ queryVector: vector(0), topK: 6 })).matches.some(item => item.id === 'doc-0'))
    await assert.rejects(() => warm.deleteDocument('doc-0'), serviceError(404, 'not_found'))
    await assert.rejects(() => warm.query({ queryVector: [1, 0] }), serviceError(422, 'invalid_request'))
    assert.equal((await warm.stats()).documents, 5)
  })

  await t.test('encoded document IDs, UTF-8 chunk boundaries and revision changes', async () => {
    const warm = client('chunks')
    const id = 'folder/doc #1?x=y%20z'
    await warm.addDocuments([{ id, text: '  café🙂\tβ  ', vector: vector(0), metadata: { kind: 'unicode' } }])
    const chunk = await warm.readDocumentChunk(id)
    assert.equal(chunk.document.id, id)
    assert.deepEqual(chunk.document.metadata, { kind: 'unicode' })
    assert.equal(chunk.document.chunkCount, 1)
    assert.equal(chunk.index, 0)
    assert.equal(chunk.startByte, 2)
    assert.equal(chunk.endByte, 14)
    assert.equal(chunk.text, 'café🙂\tβ')
    assert.equal((await warm.readDocumentChunk(id)).revision, chunk.revision)
    await assert.rejects(() => warm.readDocumentChunk(id, 1), serviceError(404, 'not_found'))
    await assert.rejects(() => client('other_chunks').readDocumentChunk(id), serviceError(404, 'not_found'))
    await warm.addDocuments([{ id, text: 'Replacement source', vector: vector(1) }])
    const replacement = await warm.readDocumentChunk(id)
    assert.equal(replacement.text, 'Replacement source')
    assert.notEqual(replacement.revision, chunk.revision)
    assert.notEqual(replacement.document.version, chunk.document.version)
    await warm.addDocuments([{ id: 'vector-only', vector: vector(2) }])
    assert.equal((await warm.readDocumentChunk('vector-only')).text, null)
    await warm.deleteDocument(id)
    await assert.rejects(() => warm.readDocumentChunk(id), serviceError(404, 'not_found'))
  })

  await t.test('authentication and tenant isolation cover both document and memory APIs', async () => {
    const alice = client('tenant_alice')
    const bob = client('tenant_bob')
    await alice.addDocuments([{ id: 'shared-id', text: 'Alice source', vector: vector(0) }])
    await bob.addDocuments([{ id: 'shared-id', text: 'Bob source', vector: vector(1) }])
    assert.equal((await alice.readDocumentChunk('shared-id')).text, 'Alice source')
    assert.equal((await bob.readDocumentChunk('shared-id')).text, 'Bob source')
    assert.deepEqual((await client('tenant_empty').query({ queryVector: vector(0) })).matches, [])
    await alice.memory.remember('Alice private evidence', { id: 'private' })
    assert.equal(await bob.memory.get('private'), null)
    assert.deepEqual(await bob.memory.recall('Alice', { history: 'all' }), [])
    for (const apiKey of [undefined, 'deliberately-wrong-fixture-key']) {
      const unauthorized = new WarmClient({ url: server.url, apiKey, tenantId: 'tenant_alice' })
      await assert.rejects(() => unauthorized.stats(), serviceError(401, 'unauthorized'))
      await assert.rejects(() => unauthorized.memory.get('private'), serviceError(401, 'unauthorized'))
    }
    assert.equal((await alice.memory.get('private')).text, 'Alice private evidence')
  })

  await t.test('memory preserves source, temporal axes, supersession, explanation and raw optional fields', async () => {
    const warm = client('temporal')
    const memory = warm.memory
    const capabilities = await memory.capabilities()
    assert.equal(capabilities.contractVersion, 1)
    assert.equal(capabilities.temporalQueries, true)
    await assert.rejects(() => memory.capabilities(2), serviceError(422, 'invalid_request'))
    const first = await memory.remember('Alice city Paris', { id: 'paris', subject: 'alice/city', validFrom: 1000,
      sourceUri: 'urn:fixture:paris', metadata: { verified: true } })
    const expires = Date.now() + 2000
    while (Date.now() <= first.ingestTime) {
      assert.ok(Date.now() < expires, 'server ingest clock exceeds local fixture clock')
      await delay(1)
    }
    const second = await memory.remember('Alice city Berlin', { id: 'berlin', subject: 'alice/city', validFrom: 2000,
      supersedes: ['paris'], sourceUri: 'urn:fixture:berlin', metadata: { verified: true } })
    assert.ok(second.ingestTime > first.ingestTime)
    assert.deepEqual(second.supersedes, ['paris'])
    const options = { k: 10, subject: 'alice/city', lexicalWeight: 1, semanticWeight: 0 }
    assert.deepEqual(ids(await memory.recall('Alice city', { ...options, asOf: 1500 })), ['paris'])
    const current = await memory.recall('Alice city', { ...options, asOf: 2500, explain: true })
    assert.deepEqual(ids(current), ['berlin'])
    assert.equal(current[0].sourceUri, 'urn:fixture:berlin')
    assert.deepEqual(current[0].metadata, { verified: true })
    assert.equal(current[0].explanation.status, 'current')
    assert.deepEqual(ids(await memory.recall('Alice city', { ...options, asOf: 2500, knownAt: first.ingestTime })), ['paris'])
    assert.deepEqual(await memory.recall('Alice city', { ...options, asOf: 2500, knownAt: first.ingestTime - 1 }), [])
    const all = await memory.recall('Alice city', { ...options, asOf: 2500, history: 'all' })
    assert.deepEqual(Object.fromEntries(all.map(item => [item.id, item.status])), { paris: 'superseded', berlin: 'current' })
    assert.deepEqual((await memory.history('berlin')).map(item => item.id), ['paris', 'berlin'])
    const explanation = await memory.explain('Alice city', 'paris', { asOf: 2500 })
    assert.equal(explanation.status, 'superseded')
    assert.ok(explanation.chain.some(link => link.id === 'berlin' && link.relation === 'successor'))
    const raw = await fetch(`${server.url}/api/v1/memory/get`, { method: 'POST',
      headers: { authorization: `Bearer ${API_KEY}`, 'x-tenant-id': 'temporal', 'content-type': 'application/json' },
      body: JSON.stringify({ id: 'berlin' }), signal: AbortSignal.timeout(5000) })
    assert.equal(raw.status, 200)
    assert.deepEqual(await memory.get('berlin'), (await raw.json()).fact)
    assert.equal(await memory.get('missing'), null)
    const dry = await memory.forget({ kind: 'ids', ids: ['berlin'] }, { includeChain: true, dryRun: true })
    assert.deepEqual(dry.forgotten.sort(), ['berlin', 'paris'])
    assert.equal(dry.dryRun, true)
    assert.notEqual(await memory.get('paris'), null)
    const purged = await memory.forget({ kind: 'ids', ids: ['berlin'] }, { includeChain: true, purge: true })
    assert.deepEqual(purged.forgotten.sort(), ['berlin', 'paris'])
    assert.equal(purged.purged, true)
    assert.equal(await memory.get('paris'), null)
    assert.equal(await memory.get('berlin'), null)
  })

  await t.test('memory batches are atomic and repeatable, validity is half-open, maintenance defaults are safe', async () => {
    const memory = client('batch').memory
    const facts = Array.from({ length: 12 }, (_, i) => ({ id: `batch-${i}`, text: `Batch evidence ${i}`, metadata: { ordinal: i } }))
    assert.ok((await memory.rememberMany(facts)).every(item => item.created))
    assert.ok((await memory.rememberMany(facts)).every(item => !item.created))
    assert.equal((await memory.recall('Batch evidence')).length, 10)
    assert.equal((await memory.recall('Batch evidence', { k: 12 })).length, 12)
    await assert.rejects(() => memory.rememberMany([
      { id: 'uncommitted', text: 'Must not be partially stored' },
      { id: 'batch-0', text: 'Conflicting replacement' },
    ]), serviceError(409, 'conflict'))
    assert.equal(await memory.get('uncommitted'), null)
    assert.equal((await memory.get('batch-0')).text, facts[0].text)
    await memory.remember('Window evidence', { id: 'window', subject: 'window', validFrom: 1000, validTo: 2000 })
    for (const [asOf, expected] of [[999, []], [1000, ['window']], [1999, ['window']], [2000, []], [2001, []]]) {
      assert.deepEqual(ids(await memory.recall('Window', { subject: 'window', asOf })), expected)
    }
    assert.deepEqual(await memory.applyForgetting(), { forgotten: [], dryRun: true, purged: false })
    assert.deepEqual(await memory.consolidate(), { created: [], forgotten: [], dryRun: true })
    assert.notEqual(await memory.get('batch-0'), null)
    const removed = await memory.forget({ kind: 'ids', ids: ['batch-0'] })
    assert.equal(removed.dryRun, false)
    assert.equal(removed.purged, false)
    assert.deepEqual(removed.forgotten, ['batch-0'])
    assert.equal(await memory.get('batch-0'), null)
    assert.deepEqual((await memory.forget({ kind: 'ids', ids: ['batch-0'] })).forgotten, [])
  })
})
