// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import assert from 'node:assert/strict'

// --8<-- [start:ingest]
import { randomUUID } from 'node:crypto'
import { WarmClient } from '../../../bindings/js/warm-sdk/dist/index.js'

const runId = randomUUID()
const warm = new WarmClient({
  url: process.env.WARM_URL ?? 'http://127.0.0.1:8080',
  apiKey: process.env.WARM_API_KEY,
  tenantId: process.env.WARM_TENANT_ID ?? `js-quickstart-${runId}`,
})
const saved = await warm.memory.remember('Restore deleted documents from verified backups.', {
  id: `backups-${runId}`, metadata: { quickstart: runId },
})
// --8<-- [end:ingest]

assert.equal(saved.created, true)

// --8<-- [start:hit]
const hits = await warm.memory.recall('restore backups', {
  k: 1, filter: { quickstart: runId },
})
console.log(hits[0]?.text)
// --8<-- [end:hit]

assert.equal(hits.length, 1)
assert.equal(hits[0].id, saved.id)
assert.equal(hits[0].text, 'Restore deleted documents from verified backups.')
assert.equal((await warm.memory.get(saved.id))?.text, hits[0].text)

// --8<-- [start:forget]
await warm.memory.forget({ kind: 'ids', ids: [saved.id] }, { purge: true })
// --8<-- [end:forget]

assert.equal(await warm.memory.get(saved.id), null)
assert.deepEqual(await warm.memory.recall('restore backups', {
  k: 1, filter: { quickstart: runId },
}), [])
console.log('JavaScript client quickstart passed: remember, recall, get, forget')
