// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
'use strict'

// Installs the release tarball into an empty project the way a user's npm would and runs a
// real document search. The package bundles every platform binding; the loader must pick the
// one for this machine.
//
// usage: node scripts/ci/smoke-node.cjs <tarballs-dir> --expect <suffix>

const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

const argv = process.argv.slice(2)
const expectAt = argv.indexOf('--expect')
const expected = expectAt >= 0 ? argv[expectAt + 1] : undefined
const tarballs = path.resolve(argv.find(arg => !arg.startsWith('--') && arg !== expected))
assert.ok(tarballs && expected, 'usage: smoke-node.cjs <tarballs-dir> --expect <suffix>')

function npm(args, cwd) {
  const windows = process.platform === 'win32'
  // Windows runs npm through its .cmd shim, which needs a shell; the arguments are fixed paths.
  return execFileSync(windows ? 'npm.cmd' : 'npm', args, { cwd, encoding: 'utf8', shell: windows, stdio: ['ignore', 'pipe', 'inherit'] })
}
const forwardSlashes = p => p.split(path.sep).join('/')

const files = fs.readdirSync(tarballs).filter(f => f.endsWith('.tgz'))
const rootTarball = files.find(f => /^warm-memory-\d/.test(f))
assert.ok(rootTarball, `no warm-memory tarball in ${tarballs}: ${files.join(', ')}`)
assert.deepEqual(files, [rootTarball], 'the release ships one npm package')

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'warm-smoke-'))
try {
  const project = path.join(work, 'project')
  fs.mkdirSync(project)
  fs.writeFileSync(path.join(project, 'package.json'), JSON.stringify({ name: 'smoke', private: true }))
  npm(['install', '--no-audit', '--no-fund', '--ignore-scripts', forwardSlashes(path.join(tarballs, rootTarball))], project)

  const installed = path.join(project, 'node_modules', 'warm-memory')
  const bindings = fs.readdirSync(installed).filter(n => n.endsWith('.node')).sort()
  assert.equal(bindings.length, 8, `expected eight bundled bindings, found ${bindings.join(', ')}`)
  assert.ok(bindings.includes(`writerslogic-warm.${expected}.node`), `no binding for ${expected}`)
  assert.ok(!bindings.includes('writerslogic-warm.node'), 'a development build leaked into the package')

  fs.writeFileSync(path.join(project, 'search.cjs'), `
    const assert = require('node:assert/strict');
    (async () => {
      const cjs = require('warm-memory');
      const esm = await import('warm-memory');
      assert.equal(cjs.WarmMemorySystem, esm.WarmMemorySystem);
      const { DocumentMemory } = await import('warm-memory/semantic');
      const warm = new cjs.WarmMemorySystem(4096, './store');
      const memory = new DocumentMemory(warm);
      await memory.memorize({ id: 'manual', text: 'Backups protect stored documents.', sourceUri: 'manual.md' });
      await memory.memorize({ id: 'other', text: 'Rivers carry water to the sea.', sourceUri: 'other.md' });
      const hits = await memory.search('backups');
      assert.equal(hits[0].sourceUri, 'manual.md');
      await memory.flush();
      console.log('search ok on ' + process.platform + '-' + process.arch);
    })().catch(e => { console.error(e); process.exitCode = 1; });
  `)
  execFileSync(process.execPath, ['search.cjs'], { cwd: project, stdio: 'inherit', timeout: 120000 })
  console.log(`smoke passed: ${expected}`)
} finally {
  fs.rmSync(work, { recursive: true, force: true })
}
