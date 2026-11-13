// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, realpath, rm } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export const API_KEY = 'portable-sdk-fixture-key-not-a-real-secret'
export const DIM = 24
export const BYTE_LIMIT = 16 * 1024 * 1024

export function deferred() {
  let resolve
  let reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

export async function deadline(promise, milliseconds, label) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out`)), milliseconds)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

export function json(res, body, status = 200, headers = {}) {
  res.writeHead(status, { 'content-type': 'application/json', ...headers })
  res.end(JSON.stringify(body))
}

export async function httpFixture(t, handler) {
  const requests = []
  const errors = []
  const server = createServer(async (req, res) => {
    const seen = { method: req.method, url: req.url, headers: req.headers, body: '' }
    requests.push(seen)
    try {
      const chunks = []
      let bytes = 0
      for await (const chunk of req) {
        bytes += chunk.length
        if (bytes > BYTE_LIMIT + 65536) throw new Error('fixture request exceeded its bound')
        chunks.push(chunk)
      }
      seen.body = Buffer.concat(chunks).toString('utf8')
      await handler(req, res, seen)
    } catch (error) {
      if (req.aborted) return
      errors.push(error)
      res.destroy()
    }
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject)
      resolve()
    })
  })
  t.after(async () => {
    const closed = new Promise(resolve => server.close(resolve))
    server.closeAllConnections()
    await closed
    assert.deepEqual(errors, [], 'HTTP fixture failed')
  })
  return { url: `http://127.0.0.1:${server.address().port}`, requests }
}

export function cleanEnvironment() {
  return Object.fromEntries(
    ['PATH', 'SystemRoot', 'WINDIR', 'TEMP', 'TMP', 'TMPDIR']
      .filter(name => process.env[name] !== undefined)
      .map(name => [name, process.env[name]]),
  )
}

export async function warmServer(t) {
  assert.ok(process.env.WARM_SERVER_BIN, 'WARM_SERVER_BIN is required; build the same-commit warm-server with --features server')
  const binary = await realpath(process.env.WARM_SERVER_BIN)
  const directory = await mkdtemp(join(tmpdir(), 'warm-js-server-'))
  const child = spawn(binary, [
    '--bind', '127.0.0.1:0', '--input-dim', String(DIM), '--dim', '4096', '--data-dir', directory,
  ], {
    env: { ...cleanEnvironment(), WARM_API_KEY: API_KEY, RUST_LOG: 'warn' },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const ready = deferred()
  const exited = new Promise(resolve => child.once('exit', (code, signal) => {
    ready.reject(new Error(`warm-server exited before readiness: ${code ?? signal}`))
    resolve()
  }))
  let output = ''
  let diagnostics = ''
  child.once('error', ready.reject)
  child.stderr.on('data', bytes => { diagnostics = (diagnostics + bytes.toString()).slice(-4096) })
  child.stdout.on('data', bytes => {
    output = (output + bytes.toString()).slice(-65536)
    const match = output.match(/(?:^|\n)warm-server listening on (127\.0\.0\.1:\d+)\r?\n/)
    if (match) ready.resolve(`http://${match[1]}`)
  })
  t.after(async () => {
    try {
      if (child.pid && child.exitCode === null && child.signalCode === null) {
        child.kill('SIGTERM')
        try {
          await deadline(exited, 2000, 'warm-server shutdown')
        } catch {
          child.kill('SIGKILL')
          await deadline(exited, 2000, 'warm-server forced shutdown')
        }
      }
    } finally {
      await rm(directory, { recursive: true, force: true })
    }
  })
  try {
    return { url: await deadline(ready.promise, 15000, 'warm-server startup'), apiKey: API_KEY }
  } catch (error) {
    throw new Error(`${error.message}${diagnostics ? `\n${diagnostics}` : ''}`)
  }
}

export function vector(index) {
  return Array.from({ length: DIM }, (_, position) => position === index % DIM ? 1 : 0)
}

export function ids(results) {
  return results.map(result => result.id).sort()
}
