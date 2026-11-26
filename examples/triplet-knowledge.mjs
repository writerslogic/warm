// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { WarmMemorySystem } from 'warm-memory'

const dir = mkdtempSync(join(tmpdir(), 'warm-triplet-knowledge-'))

try {
  const warm = new WarmMemorySystem(4096, dir, { meaningEnabled: true })
  const triples = [
    ['t1', 'paris', 'capital_of', 'france'],
    ['t2', 'berlin', 'seat_of_government_of', 'germany'],
    ['t3', 'tokyo', 'largest_city_in', 'japan'],
    ['t4', 'france', 'continent', 'europe'],
    ['t5', 'germany', 'member_of', 'nato'],
  ]
  // structuralQuery cleans up every stored triple whose overlap with the query is within 30%
  // of the best one and reports all confidences as 1.0, so triples that share a relation
  // (paris/berlin capital_of) all answer. Each triple here uses its own relation so only
  // the triple that holds the subject and relation answers.

  for (const [id, subject, relation, object] of triples) {
    await warm.memorizeTriplet(id, subject, relation, object)
  }
  console.log(`Stored ${triples.length} triples (meaningEnabled=${warm.meaningEnabled}).`)

  const countryHits = await warm.structuralQuery(['paris'], ['capital_of'], 'object')
  console.log('Structural query (paris, capital_of, ?):')
  for (const hit of countryHits) {
    console.log(`  ${hit.entityId} (confidence=${hit.confidence.toFixed(4)}, path=${hit.path})`)
  }
  if (countryHits.length !== 1 || countryHits[0].entityId !== 'france') {
    throw new Error(`expected paris -> france only, got ${countryHits.map(h => h.entityId)}`)
  }

  const continentHits = await warm.multiHopQuery('paris', ['capital_of', 'continent'])
  console.log('Multi-hop query (paris -> capital_of -> continent):')
  for (const hit of continentHits) {
    console.log(`  ${hit.entityId} (confidence=${hit.confidence.toFixed(4)}, method=${hit.method})`)
  }

  await warm.flush()
} finally {
  rmSync(dir, { recursive: true, force: true })
}
