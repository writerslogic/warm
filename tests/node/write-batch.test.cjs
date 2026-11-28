const assert = require('node:assert/strict')
const { test } = require('node:test')
const { mkdtempSync, rmSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { WarmMemorySystem } = require('../../index.js')

const store = () => mkdtempSync(join(tmpdir(), 'warm-node-batch-'))
const put = (id, text) => ({ op: 'memorize', document: { id, text } })
const del = (id) => ({ op: 'delete', id })

test('writeBatch applies memorize and delete in order and reports each operation', async () => {
  const directory = store()
  try {
    const system = new WarmMemorySystem(1024, directory)
    await system.memorizeDocument({ id: 'old', text: 'old document to remove' })
    const results = await system.writeBatch([
      put('a', 'first document'),
      del('old'),
      put('a', 'first document rewritten'),
      del('missing'),
      del('a'),
      put('a', 'final version of a'),
    ])
    assert.deepEqual(results, [
      { op: 'memorize', chunks: 1 },
      { op: 'delete', existed: true },
      { op: 'memorize', chunks: 1 },
      { op: 'delete', existed: false },
      { op: 'delete', existed: true },
      { op: 'memorize', chunks: 1 },
    ])
    const hits = await system.searchDocuments('final version')
    assert.equal(hits[0].documentId, 'a')
    assert.equal((await system.searchDocuments('old document to remove')).some((h) => h.documentId === 'old'), false)
    system.close()

    const reopened = new WarmMemorySystem(1024, directory)
    assert.equal((await reopened.searchDocuments('final version'))[0].documentId, 'a')
    reopened.close()
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('writeBatch applies nothing when any operation is invalid', async () => {
  const directory = store()
  try {
    const system = new WarmMemorySystem(1024, directory)
    await system.memorizeDocument({ id: 'keep', text: 'untouched document' })
    const reader = WarmMemorySystem.openReadOnly(1024, directory)
    for (const bad of [
      put('', 'no id'),
      del(''),
      { op: 'upsert', id: 'x' },
      { op: 'delete', id: 'x', document: { id: 'x', text: 'both' } },
      { op: 'memorize', id: 'x' },
    ]) {
      await assert.rejects(() => system.writeBatch([put('new', 'would be added'), del('keep'), bad]))
      assert.equal((await system.searchDocuments('untouched document')).length > 0, true)
      assert.equal((await system.searchDocuments('would be added')).some((h) => h.documentId === 'new'), false)
      reader.refresh()
      assert.equal((await reader.searchDocuments('untouched document')).length > 0, true)
    }
    await assert.rejects(() => system.writeBatch([]))
    await assert.rejects(
      () => system.writeBatch(Array.from({ length: 4097 }, (_, i) => del(`x${i}`))),
      /4096/,
    )
    reader.close()
    system.close()
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('a read-only store rejects writeBatch and a reader sees a committed batch whole', async () => {
  const directory = store()
  try {
    const writer = new WarmMemorySystem(1024, directory)
    await writer.memorizeDocument({ id: 'seed', text: 'seed document' })
    const reader = WarmMemorySystem.openReadOnly(1024, directory)
    await assert.rejects(() => reader.writeBatch([del('seed')]), /\[read_only\]/)
    await writer.writeBatch([put('p', 'pair zebra'), put('q', 'pair zebra'), del('seed')])
    assert.equal(reader.refresh(), true)
    const ids = (await reader.searchDocuments('pair zebra')).map((h) => h.documentId).sort()
    assert.deepEqual(ids, ['p', 'q'])
    assert.equal((await reader.searchDocuments('seed document')).some((h) => h.documentId === 'seed'), false)
    reader.close()
    writer.close()
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
