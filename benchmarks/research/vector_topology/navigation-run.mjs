// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import { createReadStream, createWriteStream } from 'node:fs'
import readline from 'node:readline'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { pipeline } from 'node:stream/promises'
import { createGzip } from 'node:zlib'
import { inspect, topology } from './navigation-analysis.mjs'
import { exec, readJson, runMonitored, sha256, watchPhase, writeOnce } from '../../public/vectra_comparison/vector-receipts.mjs'

const here = path.dirname(fileURLToPath(import.meta.url)), repo = path.resolve(here, '../../..')
const [output, binary, size = '100000'] = process.argv.slice(2)
assert.ok(['10000', '100000'].includes(size))
const corpusRows = Number(size)
assert.ok(path.isAbsolute(output ?? '') && path.isAbsolute(binary ?? ''))
assert.equal(process.env.NODE_OPTIONS, undefined)
assert.ok(!Object.keys(process.env).some(name => name.startsWith('WARM_QGRAPH_')))
const input = await readJson(path.join(repo, 'docs/research/technical-audit-20261029/vectra-comparison-20261102/inputs.json'))
const stop = await readJson(path.join(repo, 'docs/research/technical-audit-20261029/vectra-comparison-20261102/user-stop-receipt.json'))
const oldRoot = '/Volumes/A/.hms-bench-cache/vectra-comparison-20261102-frozen/results'
const corpus = path.join(input.glove.directory, 'train.f32'), queries = path.join(input.glove.directory, 'test.f32')
assert.equal(await sha256(corpus), input.glove.files['train.f32'].sha256)
assert.equal(await sha256(queries), input.glove.files['test.f32'].sha256)
const names = corpusRows === 10000 ? {
  oracle: 'preparation-0020-glove-10000-oracle-oracle.ndjson',
  exact: 'preparation-0026-glove-10000-warm-quality.ndjson',
  128: 'preparation-0027-glove-10000-warm-quality.ndjson',
  256: 'preparation-0028-glove-10000-warm-quality.ndjson',
} : {
  oracle: 'preparation-0030-glove-100000-oracle-oracle.ndjson',
  exact: 'preparation-0036-glove-100000-warm-quality.ndjson',
  128: 'preparation-0037-glove-100000-warm-quality.ndjson',
  256: 'preparation-0038-glove-100000-warm-quality.ndjson',
}
const references = {}, referenceFiles = {}
for (const [key, name] of Object.entries(names)) {
  const record = stop.trees.results.files.find(file => file.path === name)
  assert.ok(record)
  const file = path.join(oldRoot, name)
  assert.equal(await sha256(file), record.sha256)
  const rows = (await fs.readFile(file, 'utf8')).trim().split('\n').map(JSON.parse)
    .filter(row => row.type === (key === 'oracle' ? 'oracle' : 'query') && row.state === 'baseline' && row.filter === 'all')
  assert.equal(rows.length, 1000)
  references[key] = rows.map((row, i) => {
    assert.equal(row.query, i)
    assert.equal(row.queryId, `query-${i}`)
    const ids = row.hits.map(hit => hit.ordinal)
    assert.equal(ids.length, 10)
    assert.equal(new Set(ids).size, 10)
    return ids
  })
  referenceFiles[key] = { path: file, sha256: record.sha256 }
}
assert.deepEqual(references.exact, references.oracle, 'original exact membership/ranking error')
await fs.mkdir(output)
const sources = [
  'Cargo.toml', 'Cargo.lock', 'src/core/qgraph/diagnostics.rs', 'src/core/qgraph/vertex.rs',
  'src/core/qgraph/vertex/navigation.rs',
  ...['Cargo.toml', 'Cargo.lock', 'NAVIGATION-PLAN.md', 'navigation-analysis.mjs', 'navigation-test.mjs', 'navigation-run.mjs',
    'trace-worker.mjs', 'src/bin/trace.rs', 'src/bin/trace/navigation.rs'].map(name => `benchmarks/research/vector_topology/${name}`),
  'benchmarks/public/vectra_comparison/vector-receipts.mjs',
]
const sourceHashes = {}
for (const name of sources) {
  const destination = path.join(output, 'sources', name)
  await fs.mkdir(path.dirname(destination), { recursive: true })
  await fs.copyFile(path.join(repo, name), destination, fs.constants.COPYFILE_EXCL)
  sourceHashes[name] = await sha256(destination)
}
const sourceCommit = (await exec('git', ['rev-parse', 'HEAD'], { cwd: repo })).stdout.trim()
const patch = (await exec('git', ['diff', '--binary', '--', 'Cargo.toml', 'Cargo.lock', 'src/core', 'benchmarks/research/vector_topology'], { cwd: repo, maxBuffer: 16 * 1024 ** 2 })).stdout
await fs.writeFile(path.join(output, 'source.patch'), patch, { flag: 'wx' })
const binaryCopy = path.join(output, 'diagnostic-binary')
await fs.copyFile(binary, binaryCopy, fs.constants.COPYFILE_EXCL)
await fs.chmod(binaryCopy, 0o700)
const frozen = { sourceCommit, patchSha256: await sha256(path.join(output, 'source.patch')), sources: sourceHashes,
  corpus, corpusSha256: await sha256(corpus), queries, queriesSha256: await sha256(queries), referenceFiles,
  binarySha256: await sha256(binaryCopy), node: process.version, rust: (await exec('rustc', ['--version'])).stdout.trim(),
  started: new Date().toISOString(), corpusRows, efs: [128, 256], queriesPerEf: 1000,
  expectedGraphHash: corpusRows === 100000 ? 'b032161111580eb9' : null,
  scope: 'Exposed-data failure diagnosis only. No latency, held-out quality, or competitive claim.',
  deadlineMs: 7_200_000, rssLimitBytes: 8 * 1024 ** 3 }
await writeOnce(path.join(output, 'started.json'), frozen)
const deadline = Date.now() + frozen.deadlineMs, hostFile = path.join(output, 'host.ndjson')
const watcher = watchPhase(hostFile, deadline, false)
let failure, receipt, result
try {
  const raw = path.join(output, 'navigation.ndjson')
  receipt = await runMonitored({ worker: path.join(here, 'trace-worker.mjs'),
    job: { development: true, binary: binaryCopy, corpus, queries, oracle: referenceFiles.oracle.path, output: raw, mode: corpusRows === 10000 ? 'navigation10k' : 'navigation' },
    jobFile: path.join(output, 'job.json'), receipt: path.join(output, 'receipt.json'), hostFile, deadline, signal: watcher.signal,
    environment: { RAYON_NUM_THREADS: '1', OMP_NUM_THREADS: '1', VECLIB_MAXIMUM_THREADS: '1' } })
  assert.ok(receipt.complete)
  const reports = await fs.open(path.join(output, 'misses.ndjson'), 'wx')
  const cells = Object.fromEntries(frozen.efs.map(ef => [ef, { ef, queries: 0, hits: 0, worstHits: 10,
    missing: 0, classes: {}, barriers: {}, firstMissing: Array(5).fill(0), reachableMin: corpusRows, reachableMax: 0 }]))
  let graph, graphHash, count = 0
  try {
    for await (const line of readline.createInterface({ input: createReadStream(raw), crlfDelay: Infinity })) {
      const row = JSON.parse(line)
      if (count++ === 0) {
        assert.equal(row.type, 'start'); assert.equal(row.rows, corpusRows)
        graphHash = row.graphHash
        if (frozen.expectedGraphHash !== null) assert.equal(row.graphHash, frozen.expectedGraphHash)
      } else if (count === 2) {
        assert.equal(row.type, 'topology'); assert.equal(row.adjacency.length, corpusRows)
        graph = topology(row.adjacency)
      } else {
        assert.equal(row.type, 'navigation')
        const cell = cells[row.ef]
        assert.ok(cell)
        assert.equal(row.query, cell.queries)
        assert.ok(row.query < 1000)
        assert.deepEqual(row.oracle, references.oracle[row.query])
        assert.deepEqual(row.stages.selected, references[row.ef][row.query], `original ranking changed: ef=${row.ef} query=${row.query}`)
        const report = inspect(row, graph)
        cell.queries++; cell.hits += report.hits; cell.worstHits = Math.min(cell.worstHits, report.hits)
        cell.reachableMin = Math.min(cell.reachableMin, report.reachableCount)
        cell.reachableMax = Math.max(cell.reachableMax, report.reachableCount)
        for (const miss of report.misses) {
          cell.missing++
          cell.classes[miss.classification] = (cell.classes[miss.classification] ?? 0) + 1
          cell.firstMissing[miss.firstMissing]++
          if (miss.barrier) cell.barriers[miss.barrier.state] = (cell.barriers[miss.barrier.state] ?? 0) + 1
        }
        await reports.write(`${JSON.stringify(report)}\n`)
      }
    }
    assert.equal(count, 2002)
    for (const cell of Object.values(cells)) {
      assert.equal(cell.queries, 1000)
      cell.meanRecall = cell.hits / 10000; cell.worstRecall = cell.worstHits / 10
    }
  } finally { await reports.close() }
  for (const [name, hash] of Object.entries(sourceHashes)) assert.equal(await sha256(path.join(repo, name)), hash, `source changed: ${name}`)
  await pipeline(createReadStream(raw), createGzip({ level: 6 }), createWriteStream(`${raw}.gz`, { flags: 'wx' }))
  result = { graphHash, cells: Object.values(cells), rawSha256: await sha256(raw), gzipSha256: await sha256(`${raw}.gz`),
    missesSha256: await sha256(path.join(output, 'misses.ndjson')), originalRankingsMatched: 2000,
    originalExactRankingsMatched: 1000, peakOwnedTreeRssBytes: receipt.peakOwnedTreeRssBytes }
} catch (error) { failure = { name: error.name, message: error.message, stack: error.stack } }
finally { await watcher.stop() }
await writeOnce(path.join(output, 'finished.json'), { ...frozen, complete: !failure && !watcher.failure,
  failure: failure ?? watcher.failure ?? null, result, finished: new Date().toISOString() })
if (failure || watcher.failure) throw new Error(`navigation diagnostic failed: ${JSON.stringify(failure ?? watcher.failure)}`)
console.log(JSON.stringify(result, null, 2))
