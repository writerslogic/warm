// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { WarmMemorySystem } from 'writerslogic-warm'
import { DocumentMemory } from 'writerslogic-warm/semantic'

const dir = mkdtempSync(join(tmpdir(), 'warm-semantic-search-'))

try {
  const warm = new WarmMemorySystem(4096, dir)
  const docs = new DocumentMemory(warm)
  const items = [
    { id: 'backups', text: 'Restore deleted documents from verified backups.' },
    { id: 'retention', text: 'Retain deleted documents in a local archive.' },
    { id: 'encryption', text: 'AES-256-GCM seals stored frames and text segments at rest.' },
    { id: 'indexing', text: 'Train NSG and IVF indices after large vector ingestion batches.' },
  ]

  await docs.ingest(items)
  console.log(`Memorized ${items.length} documents.`)

  const queryText = 'restore deleted documents from backup'
  const hits = await docs.search(queryText, { k: 3 })
  console.log(`Query: "${queryText}"`)
  for (const hit of hits) {
    console.log(`  ${hit.documentId} (score=${hit.score.toFixed(4)}, lexicalScore=${hit.lexicalScore.toFixed(4)})`)
  }

  await docs.flush()
} finally {
  rmSync(dir, { recursive: true, force: true })
}
