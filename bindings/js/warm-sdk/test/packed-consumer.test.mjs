// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
import { cleanEnvironment, DIM, warmServer } from './support.mjs'
import { installConsumer } from './package-support.mjs'

const execute = promisify(execFile)
const guard = fileURLToPath(new URL('./fixtures/import-guard.mjs', import.meta.url))

const consumerBody = `
const check = (condition, message) => { if (!condition) throw new Error(message) }
const client = new WarmClient({ url: process.env.FIXTURE_URL, apiKey: process.env.FIXTURE_KEY, tenantId: process.env.FIXTURE_TENANT })
const capabilities = await client.memory.capabilities()
check(capabilities.contractVersion === 1, 'capabilities contract mismatch')
const remembered = await client.memory.remember('Portable package source evidence', { id: 'packed-fact', metadata: { consumer: true } })
check(remembered.created && remembered.id === 'packed-fact', 'remember failed')
check((await client.memory.get('packed-fact')).text === 'Portable package source evidence', 'get failed')
check((await client.memory.recall('Portable package')).some(item => item.id === 'packed-fact'), 'recall failed')
check((await client.memory.history('packed-fact')).length === 1, 'history failed')
check((await client.memory.explain('Portable package', 'packed-fact')).status === 'current', 'explain failed')
check((await client.memory.applyForgetting()).dryRun === true, 'forgetting default changed')
check((await client.memory.consolidate()).dryRun === true, 'consolidation default changed')
check((await client.memory.forget({ kind: 'ids', ids: ['packed-fact'] }, { dryRun: true })).dryRun, 'dry run failed')
check((await client.memory.get('packed-fact')) !== null, 'dry run mutated state')
await client.memory.forget({ kind: 'ids', ids: ['packed-fact'] })
check((await client.memory.get('packed-fact')) === null, 'forget failed')
const vector = Array.from({ length: ${DIM} }, (_, index) => index === 0 ? 1 : 0)
const id = 'packed/document #1?'
check((await client.addDocuments([{ id, text: 'Packed document source', vector, metadata: { consumer: true } }])).added === 1, 'addDocuments failed')
check((await client.query({ queryVector: vector })).matches[0].id === id, 'query failed')
check((await client.readDocumentChunk(id)).text === 'Packed document source', 'chunk failed')
check((await client.stats()).documents >= 1, 'stats failed')
check(typeof (await client.maintainDocumentIndex()).documentIndexTrained === 'boolean', 'maintenance failed')
check((await client.deleteDocument(id)).deleted, 'delete failed')
try { await client.deleteDocument(id); throw new Error('missing document did not fail') }
catch (error) { check(error instanceof WarmError && error.code === 'not_found' && error.status === 404, 'service error contract failed') }
console.log('packed-consumer-ok')
`

test('installed portable package works outside the source tree in ESM and CommonJS without native or Node imports', { timeout: 300000 }, async t => {
  const { consumer } = await installConsumer(t, process.env.WARM_SDK_PACKAGE)
  const server = await warmServer(t)
  for (const format of ['esm', 'cjs']) {
    await t.test(format, async () => {
      const entry = join(consumer, format === 'esm' ? 'consumer.mjs' : 'consumer.cjs')
      const body = format === 'esm'
        ? `import { WarmClient, WarmError } from 'warm-sdk'\n${consumerBody}`
        : `const { WarmClient, WarmError } = require('warm-sdk')\n;(async () => { ${consumerBody} })().catch(error => { console.error(error); process.exitCode = 1 })\n`
      await writeFile(entry, body)
      const result = await execute(process.execPath, ['--import', guard, entry], {
        cwd: consumer, env: { ...cleanEnvironment(), FIXTURE_URL: server.url, FIXTURE_KEY: server.apiKey,
          FIXTURE_TENANT: `packed_${format}` }, timeout: 30000, maxBuffer: 65536,
      })
      assert.equal(result.stdout.trim(), 'packed-consumer-ok')
    })
  }
})
