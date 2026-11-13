// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { glob } from './node_modules/typedoc/dist/node-utils.js'
import { Minimatch } from 'minimatch'

test('TypeDoc discovers complete entry points through literal, brace and globstar paths', () => {
  const root = mkdtempSync(join(tmpdir(), 'warm-typedoc-glob-'))
  const files = ['index.d.ts', 'src/one.d.ts', 'src/nested/two.d.ts', 'src/.hidden.d.ts',
    'other/three.d.ts', 'node_modules/dep/index.d.ts', 'src/readme.md']
  try {
    for (const name of files) {
      const path = join(root, name)
      mkdirSync(dirname(path), { recursive: true })
      writeFileSync(path, 'export interface Item { value: string }\n')
    }
    const cases = [
      ['index.d.ts', ['index.d.ts']],
      ['src/**/*.d.ts', ['src/one.d.ts', 'src/nested/two.d.ts']],
      ['{src,other}/**/*.d.ts', ['src/one.d.ts', 'src/nested/two.d.ts', 'other/three.d.ts']],
      ['src/.hidden.d.ts', ['src/.hidden.d.ts']],
      ['**/*.d.ts', ['index.d.ts', 'src/one.d.ts', 'src/nested/two.d.ts', 'other/three.d.ts']],
      ['node_modules/**/*.d.ts', ['node_modules/dep/index.d.ts']],
      ['missing/**/*.d.ts', []],
    ]
    for (const [pattern, expected] of cases) {
      assert.deepEqual(glob(`${root}/${pattern}`, root).sort(), expected.map(name => `${root}/${name}`).sort(), pattern)
    }
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('partial traversal preserves positive alternatives for negated patterns', () => {
  for (const prefix of ['', '!']) {
    const matcher = new Minimatch(`${prefix}/repo/{src,lib}/**/*.ts`, { flipNegate: true })
    assert.equal(matcher.match('/repo/src', true), true)
    assert.equal(matcher.match('/repo/lib/nested', true), true)
    assert.equal(matcher.match('/repo/other', true), false)
  }
})

test('entry point matching retains literal and glob semantics at optimization level two', () => {
  const matcher = new Minimatch('/repo/{src,lib}/**/*.ts', { optimizationLevel: 2 })
  assert.equal(matcher.match('/repo/src/nested/file.ts'), true)
  assert.equal(matcher.match('/repo/src/nested/file.js'), false)
  assert.equal(matcher.match('/repo/other/file.ts'), false)
})

test('globstar parent segments respect the expansion bound', () => {
  const pattern = `${'**/../a/b/'.repeat(10)}file.ts`
  for (const braceExpandMax of [63, 64, 65]) {
    const matcher = new Minimatch(pattern, { optimizationLevel: 2, braceExpandMax })
    assert.ok(matcher.globParts.length > 0)
    assert.ok(matcher.globParts.length <= braceExpandMax)
  }
})
