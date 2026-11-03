// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import { createReadStream } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { topology } from './navigation-analysis.mjs'
import { prepare, extend, policies } from './information-navigation.mjs'
import { readRows } from './information-input.mjs'
import { readJson, sha256, writeOnce } from '../../public/vectra_comparison/vector-receipts.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const [output] = process.argv.slice(2)
assert.ok(path.isAbsolute(output ?? ''))
const plan = await readJson(path.join(here, 'INFORMATION-PLAN.json'))
assert.deepEqual(plan.policies, policies)
await fs.mkdir(output)
const sources = {}
for (const name of ['INFORMATION-PLAN.json', 'information-navigation.mjs', 'information-input.mjs', 'information-run.mjs', 'information-test.mjs', 'navigation-analysis.mjs']) {
  await fs.copyFile(path.join(here, name), path.join(output, name), fs.constants.COPYFILE_EXCL)
  sources[name] = await sha256(path.join(output, name))
}
const inputs = []
for (const { directory, rows } of plan.inputs) {
  assert.ok(Number.isSafeInteger(rows) && rows > 0)
  const receipt = await readJson(path.join(directory, 'finished.json'))
  assert.equal(receipt.complete, true)
  if (receipt.corpusRows !== undefined) assert.equal(receipt.corpusRows, rows)
  const raw = path.join(directory, 'navigation.ndjson')
  assert.equal(await sha256(raw), receipt.result.rawSha256)
  inputs.push({ directory, raw, sha256: receipt.result.rawSha256, rows })
}
await writeOnce(path.join(output, 'freeze.json'), { plan, sources, inputs, node: process.version, started: new Date().toISOString() })
const started = Date.now(), cells = new Map()
let peakRssBytes = 0, failure = null
const rowsOut = await fs.open(path.join(output, 'queries.ndjson'), 'wx')
try {
  for (const input of inputs) {
    let graph, count = 0
    const queries = Object.fromEntries(plan.efs.map(ef => [ef, 0]))
    for await (const row of readRows(createReadStream(input.raw))) {
      if (count++ === 0) {
        assert.equal(row.type, 'start')
        if (row.rows !== undefined) assert.equal(row.rows, input.rows)
        continue
      }
      if (count === 2) {
        assert.equal(row.type, 'topology'); assert.equal(row.adjacency.length, input.rows)
        graph = topology(row.adjacency); continue
      }
      assert.equal(row.type, 'navigation')
      assert.equal(row.query, queries[row.ef]++)
      const p = prepare({ events: row.events, evaluated: row.evaluated }, graph, plan.shadowRows)
      const selected = new Set(row.stages.selected)
      const baseHits = row.oracle.filter(id => selected.has(id)).length
      for (const budget of plan.newVectorBudgets) {
        let controlHits
        for (const policy of policies) {
          const result = extend(p, policy, budget, plan.maximumExpansions)
          const union = new Set([...selected, ...result.added])
          const hits = row.oracle.filter(id => union.has(id)).length
          if (policy === 'distance') controlHits = hits
          const key = `${input.rows}/${row.ef}/${budget}/${policy}`
          if (!cells.has(key)) cells.set(key, { rows: input.rows, ef: row.ef, budget, policy, queries: 0,
            baseHits: 0, hits: 0, gained: 0, worstHits: 10, below90: 0, wins: 0, ties: 0, losses: 0,
            newVectorCount: 0, expansions: 0, adjacencyEntriesRead: 0, priorityNeighborVisits: 0,
            candidatePriorityVisits: 0, expansionEntriesRead: 0 })
          const cell = cells.get(key)
          cell.queries++; cell.baseHits += baseHits; cell.hits += hits; cell.gained += hits - baseHits
          cell.worstHits = Math.min(cell.worstHits, hits); cell.below90 += Number(hits < 9)
          cell[hits > controlHits ? 'wins' : hits < controlHits ? 'losses' : 'ties']++
          cell.expansions += result.expanded.length
          for (const metric of ['newVectorCount', 'adjacencyEntriesRead', 'priorityNeighborVisits', 'candidatePriorityVisits', 'expansionEntriesRead']) cell[metric] += result[metric]
          await rowsOut.write(`${JSON.stringify({ rows: input.rows, ef: row.ef, query: row.query, budget, policy, baseHits, hits, ...result })}\n`)
        }
      }
      peakRssBytes = Math.max(peakRssBytes, process.memoryUsage().rss)
      assert.ok(peakRssBytes <= plan.limits.maxRssBytes, 'RSS limit exceeded')
      assert.ok(Date.now() - started <= plan.limits.seconds * 1000, 'deadline exceeded')
    }
    assert.equal(count, 2 + plan.efs.length * plan.queriesPerEf)
    for (const n of Object.values(queries)) assert.equal(n, plan.queriesPerEf)
  }
  for (const [name, hash] of Object.entries(sources)) assert.equal(await sha256(path.join(here, name)), hash)
} catch (error) { failure = { name: error.name, message: error.message, stack: error.stack } }
finally { await rowsOut.close() }
const promotion = policies.filter(policy => policy !== 'distance').map(policy => {
  const comparisons = [...cells.values()].filter(cell => cell.policy === policy).map(cell => {
    const control = cells.get(`${cell.rows}/${cell.ef}/${cell.budget}/distance`)
    return { rows: cell.rows, ef: cell.ef, budget: cell.budget, gain: cell.gained, controlGain: control.gained,
      improvement: control.gained ? cell.gained / control.gained - 1 : null,
      passed: cell.worstHits >= control.worstHits && (cell.rows !== 100000 || (control.gained > 0 && cell.gained >= 1.1 * control.gained)) }
  })
  return { policy, passed: !failure && comparisons.length === 8 && comparisons.every(cell => cell.passed), comparisons }
})
await writeOnce(path.join(output, 'finished.json'), { complete: !failure, failure, elapsedSeconds: (Date.now() - started) / 1000,
  peakRssBytes, freezeSha256: await sha256(path.join(output, 'freeze.json')), rawSha256: await sha256(path.join(output, 'queries.ndjson')),
  cells: [...cells.values()], promotion, finished: new Date().toISOString(), scope: plan.scope })
console.log(JSON.stringify({ failure, promotion, peakRssBytes }))
if (failure) process.exitCode = 1
