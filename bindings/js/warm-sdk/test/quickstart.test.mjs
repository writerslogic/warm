// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
import { cleanEnvironment, warmServer } from './support.mjs'

test('the public JavaScript quickstart completes against the actual server', { timeout: 60000 }, async t => {
  const server = await warmServer(t)
  const example = fileURLToPath(new URL('../../../../examples/quickstart/js-client/quickstart.mjs', import.meta.url))
  const result = await promisify(execFile)(process.execPath, [example], {
    env: { ...cleanEnvironment(), WARM_URL: server.url, WARM_API_KEY: server.apiKey },
    timeout: 30000, maxBuffer: 65536,
  })
  assert.equal(result.stdout.trim(), 'Restore deleted documents from verified backups.\nJavaScript client quickstart passed: remember, recall, get, forget')
})
