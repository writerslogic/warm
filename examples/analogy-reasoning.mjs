// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { WarmMemorySystem } from 'warm-memory'

const dir = mkdtempSync(join(tmpdir(), 'warm-analogy-reasoning-'))

try {
  const warm = new WarmMemorySystem(4096, dir)
  // findAnalogy scores every memorized item against walking * walked * talking and does not
  // exclude the query terms, so `walked` and `talking` themselves rank high. Memorize only
  // the candidate answers and drop any query term from the hits.
  const candidates = ['talked', 'played', 'jumped', 'running', 'playing']
  const query = ['walking', 'walked', 'talking']

  for (const word of candidates) {
    await warm.memorizeText(word, word)
  }
  console.log(`Memorized ${candidates.length} lexical items.`)
  console.log('Note: findAnalogy operates over memorized multiscale-words-v1 lexical hypervectors.')

  const hits = (await warm.findAnalogy(...query, candidates.length)).filter(hit => !query.includes(hit.id))
  console.log('Analogy query (walking:walked :: talking:?):')
  for (const hit of hits) {
    console.log(`  ${hit.id} (similarity=${hit.similarity.toFixed(4)})`)
  }
  if (hits[0]?.id !== 'talked') {
    throw new Error(`expected talked first, got ${hits.map(h => h.id)}`)
  }

  await warm.flush()
} finally {
  rmSync(dir, { recursive: true, force: true })
}
