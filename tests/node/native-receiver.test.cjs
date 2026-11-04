// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

const assert = require('node:assert/strict')
const { test } = require('node:test')
const { WarmMemorySystem } = require('../../index.js')

test('native methods reject unwrapped and prototype-spoofed receivers', async () => {
  const receivers = [{}, Object.create(WarmMemorySystem.prototype), new Date(), null]
  for (const receiver of receivers) {
    assert.throws(() => WarmMemorySystem.prototype.configureMemory.call(receiver), TypeError)
    await assert.rejects(async () => WarmMemorySystem.prototype.listDocuments.call(receiver), TypeError)
    const getter = Object.getOwnPropertyDescriptor(WarmMemorySystem.prototype, 'dimensions').get
    assert.throws(() => getter.call(receiver), /Illegal invocation|Failed to unwrap exclusive reference/)
  }
})

test('compaction job methods reject unwrapped and prototype-spoofed receivers', async () => {
  const { CompactionJob } = require('../../index.js')
  const receivers = [{}, Object.create(CompactionJob.prototype), new Date(), null]
  const finished = Object.getOwnPropertyDescriptor(CompactionJob.prototype, 'finished').get
  for (const receiver of receivers) {
    assert.throws(() => CompactionJob.prototype.progress.call(receiver), TypeError)
    assert.throws(() => CompactionJob.prototype.cancel.call(receiver), TypeError)
    assert.throws(() => finished.call(receiver), /Illegal invocation|Failed to unwrap exclusive reference/)
    await assert.rejects(async () => CompactionJob.prototype.wait.call(receiver), TypeError)
  }
})
