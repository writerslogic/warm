const assert = require('node:assert/strict')
const { test } = require('node:test')
const { mkdtempSync, rmSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { WarmMemorySystem, stem, analyze } = require('../../index.js')

const DIM = 4
const config = { embeddingModel: 'fixture', embeddingRevision: 'rev-1', embeddingDimensions: DIM }

async function corpus(directory) {
  const warm = new WarmMemorySystem(4096, directory, config)
  const rows = [
    ['a', { team: 'red', n: 1, tags: ['x', 'y'] }, [1, 0, 0, 0]],
    ['b', { team: 'blue', n: 2, tags: ['y', 'z'] }, [0, 1, 0, 0]],
    ['c', { team: 'green', n: 3 }, [0, 0, 1, 0]],
  ]
  for (const [id, metadata, vector] of rows) {
    await warm.memorizeDocument({ id, text: `memo ${id}`, metadata, embeddings: [vector] })
  }
  return warm
}

const ids = async (warm, filter) => (await warm.searchDocuments('memo', { filter, k: 10 })).map(hit => hit.documentId).sort()

test('$eq and $in select documents and list fields match when any element matches', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'warm-filter-'))
  try {
    const warm = await corpus(directory)
    assert.deepEqual(await ids(warm, { team: 'red' }), ['a'])
    assert.deepEqual(await ids(warm, { team: { $eq: 'red' } }), ['a'])
    assert.deepEqual(await ids(warm, { team: { $in: ['red', 'green'] } }), ['a', 'c'])
    assert.deepEqual(await ids(warm, { n: { $in: [2, 3] } }), ['b', 'c'])
    assert.deepEqual(await ids(warm, { team: { $in: [] } }), [])
    assert.deepEqual(await ids(warm, { tags: 'y' }), ['a', 'b'])
    assert.deepEqual(await ids(warm, { tags: { $in: ['x', 'z'] } }), ['a', 'b'])
    assert.deepEqual(await ids(warm, { tags: 'q' }), [])
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('an unsupported filter operator is an error naming it', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'warm-filter-'))
  try {
    const warm = await corpus(directory)
    await assert.rejects(ids(warm, { n: { $gt: 1 } }), /\$gt/)
    await assert.rejects(ids(warm, { $or: [{ team: 'red' }] }), /\$or/)
    await assert.rejects(ids(warm, { team: { $in: 'red' } }), /\$in/)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('a hit with a stored vector carries its exact cosine as a number', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'warm-filter-'))
  try {
    const warm = await corpus(directory)
    const hits = await warm.searchDocuments('memo', { embedding: [1, 1, 0, 0], k: 3, minSemanticScore: -1 })
    const byId = Object.fromEntries(hits.map(hit => [hit.documentId, hit.semanticScore]))
    assert.ok(Math.abs(byId.a - Math.SQRT1_2) < 1e-6)
    assert.ok(Math.abs(byId.b - Math.SQRT1_2) < 1e-6)
    assert.ok(Math.abs(byId.c) < 1e-6)
    for (const hit of hits) assert.equal(typeof hit.semanticScore, 'number')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('the store reports its embedding model', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'warm-filter-'))
  try {
    const warm = await corpus(directory)
    const model = warm.embeddingModel()
    assert.equal(model.model, 'fixture')
    assert.equal(model.dimensions, DIM)
    assert.equal(typeof model.revision, 'string')
    assert.equal(new WarmMemorySystem(4096, join(directory, 'plain')).embeddingModel(), null)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('stem and analyze return the terms the indexer counts', () => {
  assert.equal(stem('running'), analyze('running')[0])
  assert.equal(analyze('harbors harbor').length, 2)
  assert.equal(analyze('harbors harbor')[0], analyze('harbors harbor')[1])
})
