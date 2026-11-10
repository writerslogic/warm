// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { realpath, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import test from 'node:test'
import { promisify } from 'node:util'
import { installConsumer, packageRoot } from './package-support.mjs'
import { cleanEnvironment } from './support.mjs'

const execute = promisify(execFile)
const imports = `import type {
  JsonValue, QueryResponse, DocumentChunkRecord, FactRecord, Recalled,
  Remembered, Explanation, ForgetReport, ConsolidationReport, Stats,
} from 'warm-sdk'
`
const usage = `
const metadata: JsonValue = { nested: [null, true, 42, 'source'] }
const signal = new AbortController().signal
const client = new WarmClient({ url: 'https://example.invalid', fetch: globalThis.fetch })
async function consume(): Promise<JsonValue> {
  await client.addDocuments([{ id: 'doc', vector: [1, 0] as const, metadata: { source: metadata } }], { signal })
  const query: QueryResponse = await client.query({ queryVector: [1, 0] as const, documentIds: ['doc'] as const, topK: 3, index: 'exact' }, { signal })
  const chunk: DocumentChunkRecord = await client.readDocumentChunk('doc', 0, { signal })
  const text: string | null | undefined = chunk.text
  const remembered: Remembered = await client.memory.remember('Evidence', { id: 'fact', metadata: { source: metadata }, validFrom: 1000, signal })
  const fact: FactRecord | null = await client.memory.get(remembered.id, { signal })
  const recalled: Recalled[] = await client.memory.recall('Evidence', { history: 'all', knownAt: 2000, signal })
  const explanation: Explanation = await client.memory.explain('Evidence', remembered.id, { asOf: 1500, signal })
  const forgotten: ForgetReport = await client.memory.forget({ kind: 'ids', ids: [remembered.id] as const }, { dryRun: true, signal })
  const consolidated: ConsolidationReport = await client.memory.consolidate({ dryRun: true, signal })
  const stats: Stats = await client.stats({ signal })
  return { metadata, matches: query.matches.map(match => match.metadata), text: text ?? null,
    source: fact?.metadata ?? null, scores: recalled.map(result => result.score),
    retrieved: explanation.retrieved, forgotten: forgotten.forgotten,
    created: consolidated.created.map(item => item.id), documents: stats.documents }
}
void consume
// @ts-expect-error The portable API exposes camelCase query options.
client.query({ query_vector: [1, 0] })
// @ts-expect-error Forget scopes are discriminated and require their matching fields.
client.memory.forget({ kind: 'subject', ids: ['fact'] })
// @ts-expect-error A fact lookup explicitly represents absence with null.
const present: Promise<FactRecord> = client.memory.get('missing')
// @ts-expect-error JSON metadata cannot contain bigint values.
const invalidMetadata: JsonValue = { value: 1n }
`

test('installed ESM and CommonJS exports expose strict portable TypeScript contracts', { timeout: 180000 }, async t => {
  const compiler = await realpath(join(packageRoot, 'node_modules', 'typescript', 'bin', 'tsc'))
  const { consumer } = await installConsumer(t)
  await writeFile(join(consumer, 'consumer.mts'), `import { WarmClient } from 'warm-sdk'
// @ts-expect-error Native embedded memory is a separate package.
import { WarmMemorySystem } from 'warm-sdk'
${imports}${usage}`)
  await writeFile(join(consumer, 'consumer.cts'), `import warm = require('warm-sdk')
const { WarmClient } = warm
// @ts-expect-error Native embedded memory is a separate package.
warm.WarmMemorySystem
${imports}${usage}`)
  await writeFile(join(consumer, 'tsconfig.json'), JSON.stringify({
    compilerOptions: { target: 'ES2022', module: 'NodeNext', moduleResolution: 'NodeNext',
      lib: ['ES2022', 'DOM', 'DOM.Iterable'], types: [], strict: true,
      skipLibCheck: false, noEmit: true, forceConsistentCasingInFileNames: true },
    files: ['consumer.mts', 'consumer.cts'],
  }))
  const compiled = await execute(process.execPath, [compiler, '--project', 'tsconfig.json', '--pretty', 'false'], {
    cwd: consumer, env: cleanEnvironment(), timeout: 60000, maxBuffer: 65536,
  }).catch(error => assert.fail(error.stdout || error.stderr || error.message))
  assert.equal(compiled.stdout, '')
  assert.equal(compiled.stderr, '')
})
