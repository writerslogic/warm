// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
const assert = require('node:assert/strict')
const { mkdtempSync, rmSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { spawnSync } = require('node:child_process')

// Runs each phase of a scenario script in its own process against one shared store directory,
// so later phases observe what earlier phases left on disk after the process exited.
function runScenario(script, prefix, phases) {
  const directory = mkdtempSync(join(tmpdir(), prefix))
  try {
    for (const phase of phases) {
      const result = spawnSync(process.execPath, [join(__dirname, script), phase, directory], {
        cwd: join(__dirname, '../../..'), encoding: 'utf8', timeout: 120000,
      })
      assert.equal(result.status, 0, `${phase}: ${result.stderr || result.stdout}`)
    }
  } finally { rmSync(directory, { recursive: true, force: true }) }
}

module.exports = { runScenario }
