// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { analyze } from './analyze.mjs'
import { exec, packageFiles, readJson, runMonitored, sha256, watchPhase, writeOnce } from '../vectra_comparison/vector-receipts.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const [output, inputRoot, sourceRoot, binary] = process.argv.slice(2)
for (const argument of [output, inputRoot, sourceRoot, binary]) assert.ok(path.isAbsolute(argument ?? ''))
assert.ok(!Object.keys(process.env).some(key => key.startsWith('WARM_') || key.startsWith('HMS_')))
assert.equal(process.env.NODE_OPTIONS, undefined)
const preparation = await readJson(path.join(inputRoot, 'finished.json'))
assert.equal(preparation.complete, true)
const inputFreeze = await readJson(path.join(inputRoot, 'freeze.json'))
const plan = await readJson(path.join(here, 'PLAN.json'))
assert.deepEqual(inputFreeze.plan, plan, 'search policy differs from pre-input seal')
assert.equal((await exec('git', ['rev-parse', 'HEAD'], { cwd: sourceRoot })).stdout.trim(), plan.policy.sourceRevision)
await fs.mkdir(output)
const inputs = []
for (const spec of plan.datasets) {
  const directory = path.join(inputRoot, spec.name), record = await readJson(path.join(directory, 'inputs.json'))
  assert.equal(await sha256(path.join(directory, 'inputs.json')), preparation.completed.find(item => item.name === spec.name)?.inputsSha256)
  assert.equal(await sha256(path.join(directory, 'train.f32')), record.training.sha256)
  assert.equal(await sha256(path.join(directory, 'test.f32')), record.queries.sha256)
  assert.equal(record.training.retainedOrdinals.length, plan.inputSelection.rows)
  assert.equal(record.queries.retainedOrdinals.length, plan.inputSelection.queries)
  inputs.push({ name: spec.name, rows: plan.inputSelection.rows, dimensions: spec.dimensions,
    queryCount: plan.inputSelection.queries, corpus: path.join(directory, 'train.f32'), queries: path.join(directory, 'test.f32'),
    corpusSha256: record.training.sha256, queriesSha256: record.queries.sha256 })
}
const packageDir = path.join(output, 'package'), source = path.join(output, 'source'), driver = path.join(output, 'driver')
await fs.mkdir(packageDir); await fs.mkdir(source); await fs.mkdir(driver)
for (const name of ['index.js', 'index.mjs', 'vectra.js', 'vectra-selector.js', 'package.json'])
  await fs.copyFile(path.join(sourceRoot, name), path.join(packageDir, name), fs.constants.COPYFILE_EXCL)
const addon = path.join(packageDir, 'warm-sdk.darwin-arm64.node')
assert.equal(process.platform, 'darwin'); assert.equal(process.arch, 'arm64')
await fs.copyFile(binary, addon, fs.constants.COPYFILE_EXCL)
for (const name of ['src', 'Cargo.toml', 'Cargo.lock', 'build.rs'])
  await fs.cp(path.join(sourceRoot, name), path.join(source, name), { recursive: true, errorOnExist: true, force: false })
for (const name of ['vector-worker.mjs', 'oracle-vector.mjs', 'vector-receipts.mjs'])
  await fs.copyFile(path.join(here, '../vectra_comparison', name), path.join(driver, name), fs.constants.COPYFILE_EXCL)
for (const name of ['PLAN.json', 'run.mjs', 'analyze.mjs', 'test-analyze.mjs'])
  await fs.copyFile(path.join(here, name), path.join(output, name), fs.constants.COPYFILE_EXCL)
const config = { warmPath: packageDir, nativeAddon: addon, highRecall: plan.policy.highRecall,
  inputIdentity: await sha256(path.join(inputRoot, 'freeze.json')) }
const configPath = path.join(output, 'config.json')
await writeOnce(configPath, config)
const frozen = { plan, inputs, package: await packageFiles(packageDir), source: await packageFiles(source),
  driver: await packageFiles(driver), controller: Object.fromEntries(await Promise.all(['PLAN.json', 'run.mjs', 'analyze.mjs', 'test-analyze.mjs'].map(async name => [name, await sha256(path.join(output, name))]))),
  node: process.version, rust: (await exec('rustc', ['--version'])).stdout.trim(),
  sourceCommit: plan.policy.sourceRevision, inputFreezeSha256: config.inputIdentity,
  environment: { RAYON_NUM_THREADS: '2', OMP_NUM_THREADS: '2', VECLIB_MAXIMUM_THREADS: '2' },
  started: new Date().toISOString(), timings: 'Disabled: shared-host accuracy qualification only; worker development flag suppresses timings and does not alter queries.' }
await writeOnce(path.join(output, 'freeze.json'), frozen)
const cells = [], receipts = []
let failure = null
try {
  for (const dataset of inputs) {
    const directory = path.join(output, dataset.name)
    await fs.mkdir(directory)
    const store = path.join(directory, 'store'), oracle = path.join(directory, 'oracle.ndjson')
    const run = async (name, action, mode, state) => {
      const destination = name === 'oracle' ? oracle : path.join(directory, `${name}.ndjson`)
      const deadline = Date.now() + plan.limits.phaseSeconds * 1000, hostFile = path.join(directory, `${name}-host.ndjson`)
      const watcher = watchPhase(hostFile, deadline, false)
      let receipt
      try {
        receipt = await runMonitored({ worker: path.join(driver, 'vector-worker.mjs'),
          job: { action, engine: 'warm', dataset, config: configPath, store, oracle, output: destination,
            arm: { mode, ef: plan.policy.ef, highRecall: plan.policy.highRecall }, highRecall: plan.policy.highRecall,
            state, development: true, accuracyQualification: true, writeStyle: 'batch' },
          jobFile: path.join(directory, `${name}-job.json`), receipt: path.join(directory, `${name}-receipt.json`),
          hostFile, deadline, environment: frozen.environment, signal: watcher.signal })
        assert.ok(receipt.complete, `${dataset.name}/${name} did not complete`)
      } finally { await watcher.stop() }
      assert.equal(watcher.failure, null)
      receipts.push({ dataset: dataset.name, phase: name, outputSha256: await sha256(destination),
        receiptSha256: await sha256(path.join(directory, `${name}-receipt.json`)), peakOwnedTreeRssBytes: receipt.peakOwnedTreeRssBytes })
      if (!['oracle', 'prepare'].includes(name)) {
        const rows = (await fs.readFile(destination, 'utf8')).trim().split('\n').map(JSON.parse)
        const result = analyze(rows, dataset.queryCount)
        assert.equal(result.length, action === 'churn' ? 8 : 4)
        cells.push(...result.map(cell => ({ dataset: dataset.name, run: name, mode, ...cell })))
      }
      await writeOnce(path.join(directory, `${name}-verified.json`), { complete: true, cells: cells.filter(cell => cell.dataset === dataset.name && cell.run === name) })
      console.log(JSON.stringify({ dataset: dataset.name, phase: name, complete: true }))
    }
    await run('oracle', 'oracle', 'exact', 'baseline')
    await run('prepare', 'prepare', 'exact', 'baseline')
    await run('baseline-exact', 'quality', 'exact', 'baseline')
    await run('baseline-graph', 'quality', 'graph', 'baseline')
    await run('churn-graph', 'churn', 'graph', 'churn')
    await run('churn-exact', 'quality', 'exact', 'churn')
    await run('reopen-graph', 'reopen', 'graph', 'churn')
  }
  assert.deepEqual(await packageFiles(packageDir), frozen.package)
  assert.deepEqual(await packageFiles(driver), frozen.driver)
  assert.deepEqual(await packageFiles(source), frozen.source)
} catch (error) { failure = { name: error.name, message: error.message, stack: error.stack } }
await writeOnce(path.join(output, 'finished.json'), { complete: !failure, qualified: !failure && cells.length === 72 && cells.every(cell => cell.pass),
  failure, cells, receipts, freezeSha256: await sha256(path.join(output, 'freeze.json')), finished: new Date().toISOString() })
if (failure) throw new Error(`accuracy qualification incomplete: ${failure.message}`)
