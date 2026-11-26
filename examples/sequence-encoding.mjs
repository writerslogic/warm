// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { WarmMemorySystem } from 'warm-memory'

const dir = mkdtempSync(join(tmpdir(), 'warm-sequence-encoding-'))

try {
  const warm = new WarmMemorySystem(4096, dir)
  const sequences = [
    ['build-then-test', ['build', 'test']],
    ['test-then-build', ['test', 'build']],
    ['lint-then-deploy', ['lint', 'deploy']],
  ]

  for (const [id, steps] of sequences) {
    await warm.memorizeSequence(id, steps)
  }
  console.log(`Encoded ${sequences.length} ordered sequences.`)

  for (const probe of [['build', 'test'], ['build']]) {
    const hits = await warm.querySequence(probe, 3)
    console.log(`Sequence query [${probe.join(', ')}]:`)
    for (const hit of hits) {
      console.log(`  ${hit.id} (similarity=${hit.similarity.toFixed(4)})`)
    }
  }

  await warm.flush()
} finally {
  rmSync(dir, { recursive: true, force: true })
}
