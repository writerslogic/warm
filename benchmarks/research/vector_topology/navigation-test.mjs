// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import test from 'node:test'
import { inspect, topology } from './navigation-analysis.mjs'

test('directed reachability includes the entry and distinguishes disconnected nodes', () => {
  const graph = topology([[1], [2], [], []])
  assert.deepEqual([...graph.reach(0).parents], [0, 0, 1, -1])
  assert.equal(graph.reach(0).count, 3)
  assert.deepEqual([...graph.reach(1).parents], [-1, 1, 1, -1])
  assert.equal(graph.reach(3).count, 1)
  for (const entry of [-1, 4, 5]) assert.throws(() => graph.reach(entry))
  assert.throws(() => topology([[1], [2]]))
})

test('every miss has a rejection, eviction, or disconnected topology witness', () => {
  const adjacency = Array.from({ length: 15 }, () => [])
  adjacency[0] = Array.from({ length: 11 }, (_, i) => i + 1)
  adjacency[10] = [12]
  adjacency[9] = [13]
  const offer = (id, distance, parent, worst, admitted, evicted) => ({ kind: 'offer', id, stored_id: id, distance, parent, worst, admitted, evicted })
  const beam = [11, 1, 2, 3, 4, 0, 5, 6, 7, 8]
  const found = [1, 2, 3, 4, 5, 6, 7]
  const row = { ef: 10, query: 0, entry: 0, beam, evaluated: Array.from({ length: 12 }, (_, i) => i),
    oracle: [...found, 12, 13, 14], stages: { selected: beam, oracleCoverage: Array.from({ length: 5 }, () => found) }, events: [
      offer(0, 50, null, null, true, null), { kind: 'expand', id: 0 },
      ...Array.from({ length: 9 }, (_, i) => offer(i + 1, (i + 1) * 10, 0, null, true, null)),
      offer(10, 1000, 0, [90, 9], false, null), offer(11, 0, 0, [90, 9], true, 9),
      ...beam.filter(id => id !== 0).map(id => ({ kind: 'expand', id })),
    ] }
  const report = inspect(row, topology(adjacency), 10)
  assert.equal(report.hits, 7)
  assert.deepEqual(report.misses.map(miss => miss.classification), ['bounded_traversal', 'bounded_traversal', 'topology_unreachable'])
  assert.equal(report.misses[0].barrier.state, 'rejected')
  assert.equal(report.misses[1].barrier.state, 'evicted_before_expansion')
  assert.equal(report.misses[1].barrier.evictor.id, 11)
  assert.deepEqual(report.misses[2].path, [])
  row.events[12].evicted = 8
  assert.throws(() => inspect(row, topology(adjacency), 10))
})

test('rejects a falsely reported beam admission and an unexpanded predecessor', () => {
  const graph = topology([[1], []])
  const offer = (id, distance, parent, worst, admitted, evicted) => ({ kind: 'offer', id, stored_id: id, distance, parent, worst, admitted, evicted })
  const row = { ef: 1, entry: 0, beam: [0], evaluated: [0, 1], events: [
    offer(0, 0, null, null, true, null), { kind: 'expand', id: 0 },
    offer(1, 1, 0, [0, 0], true, null),
  ] }
  assert.throws(() => inspect(row, graph, 1), /incorrect beam admission/)
  row.events[1].id = 1
  assert.throws(() => inspect(row, graph, 1), /incorrect best-first expansion/)
})
