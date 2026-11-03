// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'

export const policies = ['distance', 'unseen_count', 'unseen_rarity', 'alternating_distance_rarity']

function compare(a, b) {
  if (a.distance !== b.distance) return a.distance - b.distance
  if (Object.is(a.distance, -0) !== Object.is(b.distance, -0)) return Object.is(a.distance, -0) ? -1 : 1
  return a.stored_id - b.stored_id
}

export function prepare({ events, evaluated }, graph, shadowRows) {
  assert.ok(Number.isSafeInteger(shadowRows) && shadowRows > 0 && shadowRows <= 4096)
  const expanded = new Set(events.filter(event => event.kind === 'expand').map(event => event.id))
  const visited = new Set(evaluated)
  const shadow = events.filter(event => event.kind === 'offer' && !expanded.has(event.id))
    .sort(compare).slice(0, shadowRows)
  let adjacencyEntriesRead = 0
  const candidates = shadow.map(event => {
    const neighbors = graph.adjacency[event.id]
    assert.ok(neighbors)
    adjacencyEntriesRead += neighbors.length
    return { id: event.id, stored_id: event.stored_id, distance: event.distance,
      degree: neighbors.length, neighbors: neighbors.filter(id => !visited.has(id)) }
  })
  return { candidates, adjacencyEntriesRead, visitedCount: visited.size,
    weight: id => 1 / (1 + graph.incoming[id].length) }
}

export function extend(prepared, policy, vectorBudget, expansionBudget) {
  assert.ok(policies.includes(policy))
  assert.ok(Number.isSafeInteger(vectorBudget) && vectorBudget >= 0 && vectorBudget <= 65536)
  assert.ok(Number.isSafeInteger(expansionBudget) && expansionBudget >= 0 && expansionBudget <= 4096)
  const added = new Set(), expanded = [], remaining = prepared.candidates.slice()
  let priorityNeighborVisits = 0, expansionEntriesRead = 0, candidatePriorityVisits = 0
  while (expanded.length < expansionBudget && added.size < vectorBudget && remaining.length) {
    const active = policy === 'alternating_distance_rarity'
      ? expanded.length % 2 === 0 ? 'distance' : 'unseen_rarity' : policy
    let best = -1, bestValue = -Infinity
    for (let i = 0; i < remaining.length; i++) {
      const candidate = remaining[i]
      candidatePriorityVisits++
      let value = 0
      for (const id of candidate.neighbors) {
        priorityNeighborVisits++
        if (!added.has(id)) {
          value += active === 'unseen_rarity' ? prepared.weight(id) : 1
          if (active === 'distance') break
        }
      }
      if (value > 0 && (best < 0 || value > bestValue)) {
        best = i; bestValue = value
      }
      if (best >= 0 && active === 'distance') break
    }
    if (best < 0) break
    const [candidate] = remaining.splice(best, 1)
    expanded.push(candidate.id)
    expansionEntriesRead += candidate.degree
    for (const id of candidate.neighbors) {
      if (added.size >= vectorBudget) break
      added.add(id)
    }
  }
  return { added: [...added], expanded, newVectorCount: added.size,
    shadowRows: prepared.candidates.length, adjacencyEntriesRead: prepared.adjacencyEntriesRead,
    priorityNeighborVisits, candidatePriorityVisits, expansionEntriesRead }
}
