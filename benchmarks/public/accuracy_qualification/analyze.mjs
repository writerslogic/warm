// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'

export function analyze(rows, queryCount = 1000) {
  const cells = new Map(), seen = new Set()
  for (const row of rows) {
    if (row.type !== 'query') continue
    assert.ok(Number.isSafeInteger(row.query) && row.query >= 0 && row.query < queryCount)
    assert.ok(['all', '10pct', '1pct', 'empty'].includes(row.filter))
    const key = `${row.state}/${row.phase}/${row.filter}`
    const identity = `${key}/${row.query}`
    assert.ok(!seen.has(identity), 'duplicate query result')
    seen.add(identity)
    if (!cells.has(key)) cells.set(key, { key, queries: 0, correct: 0, hits: 0, worstRecall: 1,
      below90: 0, unresolvedExactMembership: 0, routes: {} })
    const cell = cells.get(key)
    cell.queries++; cell.correct += Number(row.correctness === true)
    cell.routes[row.route ?? 'error'] = (cell.routes[row.route ?? 'error'] ?? 0) + 1
    if (row.filter === 'empty') {
      if (!Array.isArray(row.hits) || row.hits.length !== 0) cell.correct -= Number(row.correctness === true)
    } else {
      const recall = row.error ? 0 : row.oracleRecall
      assert.ok(Number.isFinite(recall) && recall >= 0 && recall <= 1)
      const hits = Math.round(10 * recall)
      assert.ok(Math.abs(recall - hits / 10) < 1e-12, 'recall is not a top-10 membership count')
      cell.hits += hits; cell.worstRecall = Math.min(cell.worstRecall, recall)
      cell.below90 += Number(recall < 0.9)
    }
    if (row.audit?.missed?.length) cell.unresolvedExactMembership++
  }
  assert.ok(cells.size > 0, 'no query evidence')
  return [...cells.values()].map(cell => {
    assert.equal(cell.queries, queryCount, `incomplete workload ${cell.key}`)
    const empty = cell.key.endsWith('/empty')
    const meanRecall = empty ? null : cell.hits / (10 * cell.queries)
    return { ...cell, meanRecall, pass: cell.correct === queryCount && cell.unresolvedExactMembership === 0
      && (empty || (meanRecall >= 0.99 && cell.worstRecall >= 0.9)) }
  })
}
