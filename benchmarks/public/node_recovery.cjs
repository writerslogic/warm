// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
'use strict'

const assert = require('node:assert/strict')
const { spawn, spawnSync } = require('node:child_process')
const { once } = require('node:events')
const fs = require('node:fs')
const path = require('node:path')

const [packageDir, output, phase] = process.argv.slice(2)
assert.ok(path.isAbsolute(packageDir ?? '') && path.isAbsolute(output ?? ''))
const store = path.join(output, 'store')
const batch = Array.from({ length: 32 }, (_, i) => ({ id: `batch-${i}`, text: `Batch item ${i} is durable` }))
const original = { id: 'city-1', text: 'Alice lives in Paris', subject: 'alice/city', validFrom: 1000 }
const replacement = { id: 'city-2', text: 'Alice lives in Berlin', subject: 'alice/city', validFrom: 2000 }

async function worker() {
  const { WarmMemorySystem } = require(packageDir)
  const { Memory } = require(path.join(packageDir, 'memory.js'))
  const warm = new WarmMemorySystem(4096, store)
  const memory = new Memory(warm, { supersession: 'latest-by-subject' })
  if (phase === 'write') {
    assert.equal((await memory.rememberMany(batch)).length, batch.length)
    await memory.remember(original)
    await memory.remember(replacement)
    await memory.remember({ id: 'deleted', text: 'This record must remain forgotten' })
    assert.deepEqual((await memory.forget({ ids: ['deleted'] })).forgotten, ['deleted'])
    await warm.flush()
    process.stdout.write('durable-ack\n')
    setInterval(() => {}, 1000)
    return
  }
  assert.equal(phase, 'verify')
  for (const item of batch) assert.equal((await memory.get(item.id)).text, item.text)
  assert.equal((await memory.get(original.id)).status, 'superseded')
  assert.equal((await memory.get(replacement.id)).status, 'current')
  assert.equal(await memory.get('deleted'), null)
  assert.deepEqual((await memory.history(original.id)).map(item => item.id), [original.id, replacement.id])
  assert.deepEqual((await memory.recall('Alice lives', { subject: 'alice/city' })).map(item => item.id), [replacement.id])
  assert.deepEqual((await memory.recall('Alice lives', { subject: 'alice/city', asOf: 1500 })).map(item => item.id), [original.id])
  assert.ok((await memory.rememberMany(batch)).every(item => item.created === false))
  assert.equal((await memory.remember(original)).created, false)
  assert.equal((await memory.remember(replacement)).created, false)
  await warm.flush()
  process.stdout.write(JSON.stringify({ complete: true, batchSize: batch.length, duplicateRetryEffects: 0,
    currentAndHistoricalFactsPreserved: true, deletedRecordReturned: false }) + '\n')
}

async function main() {
  if (phase) return worker()
  fs.mkdirSync(output)
  fs.copyFileSync(__filename, path.join(output, 'node_recovery.cjs'), fs.constants.COPYFILE_EXCL)
  const stderr = fs.openSync(path.join(output, 'writer-stderr.log'), 'wx')
  const child = spawn(process.execPath, [__filename, packageDir, output, 'write'], { stdio: ['ignore', 'pipe', stderr] })
  fs.closeSync(stderr)
  const exited = once(child, 'exit')
  const timer = setTimeout(() => child.kill('SIGKILL'), 60000)
  try {
    let text = ''
    for await (const chunk of child.stdout) {
      text += chunk.toString('utf8')
      assert.ok(text.length <= 128)
      if (text.includes('\n')) break
    }
    assert.equal(text, 'durable-ack\n')
    const ack = fs.openSync(path.join(output, 'acknowledged.txt'), 'wx')
    try { fs.writeSync(ack, text); fs.fsyncSync(ack) } finally { fs.closeSync(ack) }
    assert.equal(child.kill('SIGKILL'), true)
    const [code, signal] = await exited
    assert.equal(code, null)
    assert.equal(signal, 'SIGKILL')
    const result = spawnSync(process.execPath, [__filename, packageDir, output, 'verify'], { encoding: 'utf8', timeout: 60000 })
    fs.writeFileSync(path.join(output, 'reopen-stdout.log'), result.stdout ?? '', { flag: 'wx' })
    fs.writeFileSync(path.join(output, 'reopen-stderr.log'), result.stderr ?? '', { flag: 'wx' })
    assert.equal(result.status, 0, result.stderr)
    fs.writeFileSync(path.join(output, 'finished.json'), JSON.stringify({ ...JSON.parse(result.stdout),
      node: process.version, platform: `${process.platform}-${process.arch}`, interruption: signal,
      boundary: 'After public flush completed and acknowledgement was fsynced by parent',
      limitations: 'One supported process-death boundary; not power loss or every possible interruption.' }, null, 2) + '\n', { flag: 'wx' })
  } finally {
    clearTimeout(timer)
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await exited }
  }
}

main().catch(error => { console.error(error); process.exitCode = 1 })
