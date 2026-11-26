// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// Clean-room check run from an empty project that installed the packed adapter tarballs:
// usage: node smoke-adapter.mjs <package>. Loads the ESM and CJS entry points and, for core,
// round-trips a record through the real native engine.
import { createRequire } from 'node:module'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
const pkg = process.argv[2]
const esm = await import(pkg)
// The Claude memory tool is ESM-only.
const esmOnly = pkg === '@writerslogic/warm-claude-tool'
const cjs = esmOnly ? {} : createRequire(import.meta.url)(pkg)
const names = Object.keys(esm).filter((k) => k !== 'default')
if (names.length === 0 || (!esmOnly && Object.keys(cjs).length === 0)) throw new Error(`${pkg} exports nothing`)
console.log(pkg, 'esm', names.length, 'cjs', Object.keys(cjs).length)
if (pkg === '@writerslogic/warm-core') {
  const store = new esm.WarmStore({ path: mkdtempSync(join(tmpdir(), 'warm-smoke-')) })
  await store.remember([{ id: 'a', text: 'Ada wrote the first program' }])
  const hits = await store.recall('first program', { k: 1 })
  if (hits[0]?.id !== 'a') throw new Error('recall did not return the stored record')
  await store.close()
  console.log('round trip ok')
}
