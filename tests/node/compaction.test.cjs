const assert = require('node:assert/strict')
const { test } = require('node:test')
const { mkdtempSync, readdirSync, rmSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { WarmMemorySystem } = require('../../index.js')

const generations = directory =>
  readdirSync(join(directory, 'vectors_data.bin')).filter(name => name.startsWith('gen-'))

async function churned(directory) {
  const warm = new WarmMemorySystem(1024, directory)
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 32; i++) await warm.memorizeText(`v${i}`, `round ${round} item ${i}`)
  }
  for (let i = 0; i < 32; i++) await warm.delete(`v${i}`)
  await warm.memorizeText('kept', 'kept value')
  return warm
}

test('background compaction reports progress, reclaims space and keeps foreground writes', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'warm-node-compaction-'))
  try {
    const warm = await churned(directory)
    const job = warm.startCompaction({ maxCpuFraction: 0.5 })
    let writes = 0
    while (!job.finished || writes < 5) {
      await warm.memorizeText(`during-${writes}`, `during ${writes}`)
      assert.ok((await warm.query('kept value', 1)).length > 0)
      writes++
    }
    const report = await job.wait()
    assert.equal(report.status, 'completed', report.message)
    assert.equal(report.published, true)
    assert.equal(report.progress.stage, 'finished')
    assert.equal(report.progress.framesWritten, report.progress.framesTotal)
    assert.equal(report.reclaimedBytes, Math.max(0, report.diskBytesBefore - report.diskBytesAfter))
    assert.equal(
      report.writeAmplification,
      report.reclaimedBytes > 0 ? report.progress.bytesWritten / report.reclaimedBytes : null
    )
    assert.deepEqual(await job.wait(), report)
    assert.equal(job.progress().stage, 'finished')
    assert.equal(generations(directory).length, 1)
    assert.equal(warm.vectorCount, 1 + writes)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('budgets, no-progress and option validation finish without publishing', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'warm-node-compaction-'))
  try {
    const warm = await churned(directory)
    const disk = await warm.startCompaction({ maxTempDiskBytes: 1 }).wait()
    assert.equal(disk.status, 'budget_exceeded')
    assert.equal(disk.resource, 'temp_disk')
    assert.equal(disk.published, false)
    const memory = await warm.startCompaction({ maxMemoryBytes: 1 }).wait()
    assert.equal(memory.status, 'budget_exceeded')
    assert.equal(memory.resource, 'memory')
    assert.equal(generations(directory).length, 0)
    assert.equal((await warm.startCompaction().wait()).status, 'completed')
    const again = await warm.startCompaction().wait()
    assert.equal(again.status, 'no_progress')
    assert.equal(generations(directory).length, 1)
    assert.throws(() => warm.startCompaction({ maxCpuFraction: 0 }))
    assert.throws(() => warm.startCompaction({ maxMemoryBytes: -1 }))
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('cancel is idempotent and a cancelled or finished job leaves only the live generation', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'warm-node-compaction-'))
  try {
    const warm = await churned(directory)
    const job = warm.startCompaction({ maxCpuFraction: 0.01 })
    job.cancel()
    job.cancel()
    assert.equal(job.progress().cancelRequested, true)
    const report = await job.wait()
    // The worker may already be past the point of no return; it never reports a published
    // generation as cancelled.
    if (report.status === 'cancelled') assert.equal(report.published, false)
    else assert.equal(report.status, 'completed')
    assert.ok(generations(directory).length <= 1)
    assert.ok((await warm.query('kept value', 1)).length > 0)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
