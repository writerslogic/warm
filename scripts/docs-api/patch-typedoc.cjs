// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

const { createHash, randomUUID } = require('node:crypto')
const { readFileSync, writeFileSync, renameSync, unlinkSync } = require('node:fs')
const { join } = require('node:path')

const root = join(__dirname, 'node_modules', 'typedoc')
const path = join(root, 'dist', 'node-utils.js')
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version
if (version !== '0.28.20') throw new Error('Review the TypeDoc compatibility patch for this version')

const originalHash = '44982772d80190c9065e29081826b9586c2f65ab71a8d4dd0bbfdeecfecd9ebc'
const beforeConstructor = '  const mini = new Minimatch2(pattern);'
const afterConstructor = `${beforeConstructor}\n  const recurse = new Minimatch2(pattern, { flipNegate: true });`
const beforeMatch = `      return mini.set.some((row) => mini.matchOne(
        childPath,
        row,
        /* partial */
        true
      ));`
const afterMatch = '      return recurse.match(childPath.join("/"), true);'
const source = readFileSync(path, 'utf8')
const original = source.replace(afterConstructor, beforeConstructor).replace(afterMatch, beforeMatch)
if (createHash('sha256').update(original).digest('hex') !== originalHash) {
  throw new Error('TypeDoc source differs from the reviewed package')
}
const patched = original.replace(beforeConstructor, afterConstructor).replace(beforeMatch, afterMatch)
if (source !== original && source !== patched) throw new Error('Partial TypeDoc patch detected')
if (source !== patched) {
  const temporary = `${path}.${randomUUID()}.tmp`
  try {
    writeFileSync(temporary, patched, { flag: 'wx' })
    renameSync(temporary, path)
  } finally {
    try { unlinkSync(temporary) } catch (error) { if (error.code !== 'ENOENT') throw error }
  }
}
