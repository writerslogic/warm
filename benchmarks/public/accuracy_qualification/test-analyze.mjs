// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import test from 'node:test'
import { analyze } from './analyze.mjs'

const row = (query, oracleRecall = 1) => ({ type: 'query', query, state: 'baseline', phase: 'prepared',
  filter: 'all', correctness: true, oracleRecall, route: 'fullGraph' })

test('quality gates enforce mean and worst rather than aggregate success', () => {
  for (const misses of [9, 10, 11]) {
    const cells = analyze(Array.from({ length: 100 }, (_, i) => row(i, i < misses ? 0.9 : 1)), 100)
    assert.equal(cells[0].pass, misses <= 10)
  }
  assert.equal(analyze([row(0), row(1, 0.8)], 2)[0].pass, false)
  assert.equal(analyze([{ ...row(0), correctness: false }], 1)[0].pass, false)
})

test('exact substitutions require resolution and empty filters require no results', () => {
  assert.equal(analyze([{ ...row(0), audit: { missed: [2], substitutes: [3], pass: true } }], 1)[0].pass, false)
  const empty = { ...row(0), filter: 'empty', hits: [], oracleRecall: null, route: 'filteredExact' }
  assert.equal(analyze([empty], 1)[0].pass, true)
  assert.equal(analyze([{ ...empty, hits: [{ id: 'unexpected' }] }], 1)[0].pass, false)
})

test('missing, duplicate and out-of-range query evidence cannot qualify', () => {
  assert.throws(() => analyze([row(0)], 2), /incomplete/)
  assert.throws(() => analyze([row(0), row(0)], 2), /duplicate/)
  for (const query of [-1, 1, 2]) assert.throws(() => analyze([row(query)], 1))
  assert.throws(() => analyze([], 1), /no query evidence/)
})
