// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { WarmMemorySystem } from 'writerslogic-warm'

const dir = mkdtempSync(join(tmpdir(), 'warm-triplet-knowledge-'))

try {
  const warm = new WarmMemorySystem(4096, dir, { meaningEnabled: true })
  const triples = [
    ['t1', 'paris', 'capital_of', 'france'],
    ['t2', 'berlin', 'capital_of', 'germany'],
    ['t3', 'tokyo', 'capital_of', 'japan'],
    ['t4', 'france', 'continent', 'europe'],
    ['t5', 'germany', 'continent', 'europe'],
  ]

  for (const [id, subject, relation, object] of triples) {
    await warm.memorizeTriplet(id, subject, relation, object)
  }
  console.log(`Stored ${triples.length} triples (meaningEnabled=${warm.meaningEnabled}).`)

  const countryHits = await warm.structuralQuery(['paris'], ['capital_of'], 'object')
  console.log('Structural query (paris, capital_of, ?):')
  for (const hit of countryHits) {
    console.log(`  ${hit.entityId} (confidence=${hit.confidence.toFixed(4)}, path=${hit.path})`)
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
