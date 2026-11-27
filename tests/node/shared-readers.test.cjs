const assert = require('node:assert/strict')
const { test } = require('node:test')
const { spawnSync } = require('node:child_process')
const { mkdtempSync, rmSync, readdirSync, existsSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { WarmMemorySystem } = require('../../index.js')

const store = () => mkdtempSync(join(tmpdir(), 'warm-node-readers-'))

test('a reader sees a snapshot, refresh picks up commits, and it cannot write', async () => {
  const directory = store()
  try {
    const writer = new WarmMemorySystem(1024, directory)
    await writer.memorizeDocument({ id: 'one', text: 'first committed document' })

    const reader = WarmMemorySystem.openReadOnly(1024, directory)
    assert.equal(reader.readOnly, true)
    assert.equal(writer.readOnly, false)
    assert.equal(reader.refresh(), false)

    await writer.memorizeDocument({ id: 'two', text: 'second committed document' })
    await assert.rejects(() => reader.memorizeDocument({ id: 'x', text: 'denied' }), /\[read_only\]/)
    assert.equal(reader.refresh(), true)
    assert.equal(reader.refresh(), false)
    assert.ok(readdirSync(join(directory, '.hms.readers')).length === 1)

    reader.close()
    assert.equal(readdirSync(join(directory, '.hms.readers')).length, 0)
    writer.close()
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('close releases the lock, is idempotent, and later calls fail clearly', async () => {
  const directory = store()
  try {
    const first = new WarmMemorySystem(1024, directory)
    await first.memorizeDocument({ id: 'one', text: 'committed before close' })
    assert.throws(() => new WarmMemorySystem(1024, directory), /already open/)
    assert.equal(first.closed, false)
    first.close()
    first.close()
    assert.equal(first.closed, true)
    await assert.rejects(() => first.memorizeDocument({ id: 'two', text: 'late' }), /\[store_closed\]/)

    const second = new WarmMemorySystem(1024, directory)
    second.close()
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('a second writer process is refused with the holder named', async () => {
  const directory = store()
  try {
    const holder = new WarmMemorySystem(1024, directory)
    const probe = spawnSync(
      process.execPath,
      [
        '-e',
        `try { new (require(${JSON.stringify(join(__dirname, '../../index.js'))}).WarmMemorySystem)(1024, ${JSON.stringify(directory)}) } catch (e) { console.log(e.message) }`
      ],
      { encoding: 'utf8' }
    )
    assert.match(probe.stdout, /\[store_locked\].*already open.*(pid \d+|holder unknown)/)
    // Windows denies reads of a locked file, so only Unix can name the holder.
    if (process.platform !== 'win32') assert.ok(probe.stdout.includes(`pid ${process.pid}`))
    assert.ok(existsSync(join(directory, '.hms.lock')))
    holder.close()
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
