// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
'use strict'

// Copies the native bindings built with `napi build --platform` into the package root as
// `<binaryName>.<suffix>.node`, so one npm package carries every platform and index.js picks
// the file for the running os/cpu/libc. The root package.json owns the platform list
// (napi.targets); a release must ship exactly those bindings, never a partial set.
//
// usage: node scripts/ci/npm-bundle-bindings.cjs <artifacts-dir> <package-dir> [--allow-partial]

const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..', '..')
const rootPackage = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))

const suffixes = {
  'aarch64-apple-darwin': 'darwin-arm64',
  'x86_64-apple-darwin': 'darwin-x64',
  'aarch64-unknown-linux-gnu': 'linux-arm64-gnu',
  'aarch64-unknown-linux-musl': 'linux-arm64-musl',
  'x86_64-unknown-linux-gnu': 'linux-x64-gnu',
  'x86_64-unknown-linux-musl': 'linux-x64-musl',
  'aarch64-pc-windows-msvc': 'win32-arm64-msvc',
  'x86_64-pc-windows-msvc': 'win32-x64-msvc',
}

function expectedSuffixes() {
  return rootPackage.napi.targets.map(triple => {
    assert.ok(suffixes[triple], `napi.targets lists ${triple}, which this script does not know`)
    return suffixes[triple]
  })
}

function findBindings(dir) {
  const prefix = `${rootPackage.napi.binaryName}.`
  const found = new Map()
  const walk = current => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.startsWith(prefix) && entry.name.endsWith('.node')) {
        const suffix = entry.name.slice(prefix.length, -'.node'.length)
        assert.ok(!found.has(suffix), `two bindings for ${suffix}: ${found.get(suffix)} and ${full}`)
        found.set(suffix, full)
      }
    }
  }
  walk(dir)
  return found
}

function main(argv) {
  const [artifacts, packageDir] = argv.filter(arg => !arg.startsWith('--'))
  const partial = argv.includes('--allow-partial')
  assert.ok(artifacts && packageDir, 'usage: npm-bundle-bindings.cjs <artifacts-dir> <package-dir> [--allow-partial]')
  assert.ok(!rootPackage.optionalDependencies, 'the package bundles its bindings; it must not declare platform packages')
  const expected = expectedSuffixes()
  const bindings = findBindings(artifacts)
  for (const suffix of bindings.keys()) assert.ok(expected.includes(suffix), `unexpected binding ${suffix}`)
  const missing = expected.filter(suffix => !bindings.has(suffix))
  assert.ok(partial || missing.length === 0, `missing bindings: ${missing.join(', ')}`)
  const unsuffixed = path.join(packageDir, `${rootPackage.napi.binaryName}.node`)
  assert.ok(!fs.existsSync(unsuffixed), `${unsuffixed} is a local development build; remove it before packing`)
  const copied = []
  for (const suffix of expected.filter(s => bindings.has(s))) {
    const file = `${rootPackage.napi.binaryName}.${suffix}.node`
    fs.copyFileSync(bindings.get(suffix), path.join(packageDir, file))
    copied.push(file)
  }
  console.log(JSON.stringify({ name: rootPackage.name, version: rootPackage.version, bindings: copied, missing }))
}

main(process.argv.slice(2))
