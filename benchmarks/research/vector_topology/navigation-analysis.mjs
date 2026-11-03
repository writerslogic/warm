// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'

export function topology(adjacency) {
  assert.ok(Array.isArray(adjacency) && adjacency.length > 0)
  const incoming = Array.from(adjacency, () => [])
  adjacency.forEach((neighbors, id) => {
    assert.equal(new Set(neighbors).size, neighbors.length)
    for (const neighbor of neighbors) {
      assert.ok(Number.isSafeInteger(neighbor) && neighbor >= 0 && neighbor < adjacency.length)
      incoming[neighbor].push(id)
    }
  })
  const cache = new Map()
  return { adjacency, incoming, reach(entry) {
    assert.ok(Number.isSafeInteger(entry) && entry >= 0 && entry < adjacency.length)
    if (cache.has(entry)) return cache.get(entry)
    const parents = new Int32Array(adjacency.length).fill(-1), queue = [entry]
    parents[entry] = entry
    for (let cursor = 0; cursor < queue.length; cursor++) {
      for (const id of adjacency[queue[cursor]]) if (parents[id] < 0) {
        parents[id] = queue[cursor]; queue.push(id)
      }
    }
    const value = { parents, count: queue.length }
    cache.set(entry, value)
    return value
  } }
}

function compare(a, b) {
  if (a.distance !== b.distance) return a.distance - b.distance
  if (Object.is(a.distance, -0) !== Object.is(b.distance, -0)) return Object.is(a.distance, -0) ? -1 : 1
  return a.stored_id - b.stored_id
}

export function inspect(row, graph, k = 18) {
  assert.ok(Number.isSafeInteger(row.ef) && row.ef >= k)
  const pool = [], nodes = new Map(), stored = new Set()
  let pending = [], active = null, expansions = 0
  row.events.forEach((event, sequence) => {
    assert.ok(Number.isSafeInteger(event.id) && event.id >= 0 && event.id < graph.adjacency.length)
    if (event.kind === 'expand') {
      assert.equal(pending.length, 0, 'unscored outgoing neighbors')
      const expected = pool.find(node => !node.expanded)
      assert.equal(event.id, expected?.id, 'incorrect best-first expansion')
      expected.expanded = true
      nodes.get(event.id).expansion = sequence
      expansions++
      active = event.id
      pending = graph.adjacency[event.id].filter(id => !nodes.has(id))
      return
    }
    assert.equal(event.kind, 'offer')
    assert.ok(Number.isFinite(event.distance))
    assert.ok(Number.isSafeInteger(event.stored_id) && event.stored_id >= 0 && event.stored_id < graph.adjacency.length)
    assert.ok(!stored.has(event.stored_id) && !nodes.has(event.id), 'vertex scored more than once')
    stored.add(event.stored_id)
    if (event.parent === null) {
      assert.equal(pending.length, 0)
      assert.ok(pool.every(node => node.expanded) && pool.length < k)
      if (nodes.size === 0) assert.equal(event.id, row.entry)
    } else {
      assert.equal(event.parent, active)
      assert.equal(pending.shift(), event.id, 'incorrect adjacency/visited order')
    }
    const worst = pool.length >= row.ef ? pool.at(-1) : null
    assert.deepEqual(event.worst, worst ? [worst.distance, worst.id] : null)
    const admitted = !worst || compare(event, worst) < 0
    assert.equal(event.admitted, admitted, 'incorrect beam admission')
    assert.equal(event.evicted, admitted && worst ? worst.id : null)
    nodes.set(event.id, { offer: sequence, event, expansion: null, eviction: null })
    if (admitted) {
      if (worst) {
        pool.pop()
        nodes.get(worst.id).eviction = sequence
      }
      const position = pool.findIndex(node => compare(event, node) < 0)
      pool.splice(position < 0 ? pool.length : position, 0, { ...event, expanded: false })
    }
  })
  assert.equal(pending.length, 0)
  assert.ok(pool.every(node => node.expanded), 'traversal stopped before draining the beam')
  assert.deepEqual(pool.map(node => node.id), row.beam)
  const evaluated = new Set(row.evaluated)
  for (const id of nodes.keys()) assert.ok(evaluated.has(id))
  const state = id => {
    const node = nodes.get(id)
    if (!node) return { id, state: evaluated.has(id) ? 'upper_layer_only' : 'unseen' }
    const status = !node.event.admitted ? 'rejected'
      : node.expansion === null ? 'evicted_before_expansion'
        : node.eviction === null ? 'expanded_retained' : 'expanded_then_evicted'
    return { id, state: status, offer: node.offer, expansion: node.expansion, eviction: node.eviction,
      admission: node.event, evictor: node.eviction === null ? null : row.events[node.eviction] }
  }
  const { parents, count } = graph.reach(row.entry)
  const selected = new Set(row.stages.selected)
  assert.equal(selected.size, 10)
  const misses = row.oracle.filter(id => !selected.has(id)).map(id => {
    const reachable = parents[id] >= 0, path = []
    if (reachable) {
      for (let current = id; ; current = parents[current]) {
        path.push(current)
        if (current === row.entry) break
      }
      path.reverse()
    }
    const incoming = graph.incoming[id].map(state)
    if (!nodes.has(id)) assert.ok(incoming.every(node => node.expansion == null), 'miss has an expanded predecessor')
    const firstMissing = row.stages.oracleCoverage.findIndex(stage => !stage.includes(id))
    assert.ok(firstMissing >= 0)
    const classification = !reachable && !nodes.has(id) ? 'topology_unreachable'
      : !nodes.has(id) ? 'bounded_traversal' : ['evaluation', 'retention', 'rerank_input', 'shortlist', 'final_rescore'][firstMissing]
    const pathStates = path.map(state)
    const barrier = pathStates.slice(0, -1).find(node => node.expansion == null) ?? null
    if (classification === 'bounded_traversal') {
      assert.ok(barrier && ['rejected', 'evicted_before_expansion'].includes(barrier.state))
    }
    return { id, classification, firstMissing, reachable, target: state(id), incoming, path: pathStates, barrier }
  })
  return { ef: row.ef, query: row.query, entry: row.entry, reachableCount: count,
    expansions, offers: nodes.size, hits: 10 - misses.length, misses }
}
