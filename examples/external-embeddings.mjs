// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { WarmMemorySystem } from 'warm-memory'

// Opt out of default encryption so memorizeVector/queryVector use the dense UpdatableVGraph cosine index.
process.env.WARM_ENCRYPTION ??= 'off'

const normalize = values => {
  const norm = Math.hypot(...values)
  return Float32Array.from(values, v => v / norm)
}

if (process.argv[2]) {
  const warm = new WarmMemorySystem(4096, process.argv[2])
  const embeddings = [
    ['vec-auth', [0.92, 0.35, 0.12, 0.08]],
    ['vec-cache', [0.85, 0.48, 0.18, 0.10]],
    ['vec-ocean', [0.05, 0.12, 0.94, 0.31]],
    ['vec-music', [0.10, 0.08, 0.25, 0.96]],
  ]
  for (const [id, raw] of embeddings) {
    await warm.memorizeVector(id, normalize(raw))
  }
  console.log(`Ingested ${embeddings.length} Float32Array vectors.`)

  const queryVec = normalize([0.90, 0.40, 0.14, 0.09])
  const hits = await warm.queryVector(queryVec, 3)
  console.log('Top vector matches for external query embedding:')
  for (const hit of hits) {
    console.log(`  ${hit.id} (similarity=${hit.similarity.toFixed(4)})`)
  }
  await warm.flush()
} else {
  const dir = mkdtempSync(join(tmpdir(), 'warm-external-embeddings-'))
  try {
    // Run in a short-lived child process so mmap handles drop before Windows cleanup.
    execFileSync(process.execPath, [fileURLToPath(import.meta.url), dir], { stdio: 'inherit' })
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}
