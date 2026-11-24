// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { WarmMemorySystem } from 'writerslogic-warm'

const dir = mkdtempSync(join(tmpdir(), 'warm-analogy-reasoning-'))

try {
  const warm = new WarmMemorySystem(4096, dir)
  const words = ['walking', 'walked', 'talking', 'talked', 'playing', 'played']

  for (const word of words) {
    await warm.memorizeText(word, word)
  }
  console.log(`Memorized ${words.length} lexical items.`)
  console.log('Note: findAnalogy operates over memorized multiscale-words-v1 lexical hypervectors.')

  const hits = await warm.findAnalogy('walking', 'walked', 'talking', 3)
  console.log('Analogy query (walking:walked :: talking:?):')
  for (const hit of hits) {
    console.log(`  ${hit.id} (similarity=${hit.similarity.toFixed(4)})`)
  }

  await warm.flush()
} finally {
  rmSync(dir, { recursive: true, force: true })
}
