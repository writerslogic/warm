// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import test from 'node:test'
import { topology } from './navigation-analysis.mjs'
import { prepare, extend, policies } from './information-navigation.mjs'
import { readRows } from './information-input.mjs'

const offer = (id, distance) => ({ kind: 'offer', id, stored_id: id, distance })

test('selection cannot read oracle and can choose a distant access point', () => {
  const graph = topology([[1, 2], [3], [4, 5, 6], [], [], [], []])
  const row = { events: [offer(0, 0), offer(1, 1), offer(2, 2), { kind: 'expand', id: 0 }], evaluated: [0, 1, 2],
    get oracle() { throw new Error('oracle leakage') }, get stages() { throw new Error('answer leakage') } }
  const p = prepare(row, graph, 256)
  assert.deepEqual(extend(p, 'distance', 3, 1).added, [3])
  assert.deepEqual(extend(p, 'unseen_count', 3, 1).added, [4, 5, 6])
  assert.deepEqual(extend(p, 'unseen_rarity', 3, 1).added, [4, 5, 6])
  for (const budget of [2, 3, 4]) {
    for (const policy of policies) {
      const result = extend(p, policy, budget, 2)
      assert.equal(result.added.length, budget)
      assert.equal(new Set(result.added).size, budget)
      assert.ok(result.expanded.length <= 2)
    }
  }
  assert.deepEqual(extend(p, 'distance', 0, 1).added, [])
  assert.deepEqual(extend(p, 'distance', 4, 0).added, [])
})

test('overlap is charged once and rarity discounts common neighbors', () => {
  const graph = topology([[2, 3], [3, 4], [], [], [], [2], [2], [3], [3]])
  const p = prepare({ events: [offer(0, 0), offer(1, 1)], evaluated: [0, 1] }, graph, 2)
  assert.deepEqual(extend(p, 'unseen_rarity', 3, 1).expanded, [1])
  for (const policy of policies) assert.deepEqual(extend(p, policy, 4, 2).added.slice().sort(), [2, 3, 4])
  assert.deepEqual(extend(p, 'alternating_distance_rarity', 3, 2).expanded, [0, 1])
})

test('caps, deterministic ties, and exhausted or empty shadows', () => {
  const graph = topology([[2], [3], [], []])
  const row = { events: [offer(1, 0), offer(0, 0)], evaluated: [0, 1] }
  for (const n of [1, 2, 3]) assert.equal(prepare(row, graph, n).candidates.length, Math.min(n, 2))
  const p = prepare(row, graph, 2)
  for (const policy of policies) assert.deepEqual(extend(p, policy, 100, 100).expanded, [0, 1])
  assert.deepEqual(extend(prepare({ events: [], evaluated: [] }, graph, 1), 'distance', 1, 1).added, [])
  for (const n of [-1, 0.5, 65537]) assert.throws(() => extend(p, 'distance', n, 1))
  for (const n of [-1, 0.5, 4097]) assert.throws(() => extend(p, 'distance', 1, n))
  for (const n of [0, 0.5, 4097]) assert.throws(() => prepare(row, graph, n))
  assert.throws(() => extend(p, 'unknown', 1, 1))
})

test('trace input bounds queued data and handles byte boundaries and unterminated lines', async () => {
  const source = Buffer.from('{"text":"東京"}\n{"id":2}')
  const parsed = []
  for await (const row of readRows([...source].map(byte => Buffer.from([byte])))) parsed.push(row)
  assert.deepEqual(parsed, [{ text: '東京' }, { id: 2 }])
  for (const bytes of [7, 8, 9]) {
    const consume = async () => { for await (const row of readRows([Buffer.from('{"id":2}\n')], bytes)) assert.equal(row.id, 2) }
    if (bytes < 8) await assert.rejects(consume, /allocation limit/)
    else await consume()
  }
  let read = 0
  async function* upstream() {
    for (let i = 0; i < 10; i++) { read++; yield Buffer.from('{"id":2}\n') }
  }
  for await (const row of readRows(upstream())) { assert.equal(row.id, 2); break }
  assert.equal(read, 1, 'consumer suspension must stop input iteration')
})
