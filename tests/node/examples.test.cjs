// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
const assert = require('node:assert/strict')
const { test } = require('node:test')
const { spawnSync } = require('node:child_process')
const { join } = require('node:path')

const root = join(__dirname, '../..')

const examples = [
  {
    file: 'examples/semantic-search.mjs',
    checks: [
      out => assert.ok(out.includes('Memorized 4 documents.')),
      out => {
        const match = out.match(/Query:.*\r?\n\s+(\S+)\s+\(score=([0-9.]+),\s+lexicalScore=([0-9.]+)\)/)
        assert.ok(match, `Expected ranked search hit in output:\n${out}`)
        assert.equal(match[1], 'backups')
        assert.ok(Number(match[2]) > 0, `Expected non-zero score, got ${match[2]}`)
        assert.ok(Number(match[3]) > 0, `Expected non-zero lexicalScore, got ${match[3]}`)
      },
    ],
  },
  {
    file: 'examples/triplet-knowledge.mjs',
    checks: [
      out => assert.ok(out.includes('Stored 5 triples (meaningEnabled=true).')),
      out => {
        const block = out.split('Structural query (paris, capital_of, ?):')[1].split('Multi-hop')[0]
        assert.deepEqual([...block.matchAll(/^\s+(\S+) \(confidence=/gm)].map(m => m[1]), ['france'])
      },
      out => assert.match(out, /Multi-hop query[^\n]*\n\s+europe \(/),
    ],
  },
  {
    file: 'examples/sequence-encoding.mjs',
    checks: [
      out => assert.ok(out.includes('Encoded 3 ordered sequences.')),
      out => assert.ok(out.includes('build-then-test (similarity=1.0000)')),
      out => assert.ok(out.includes('Sequence query [build]:')),
    ],
  },
  {
    file: 'examples/analogy-reasoning.mjs',
    checks: [
      out => assert.ok(out.includes('Memorized 5 lexical items.')),
      out => {
        const hits = [...out.split('Analogy query (walking:walked :: talking:?):')[1].matchAll(/^\s+(\S+) \(similarity=/gm)].map(m => m[1])
        assert.equal(hits[0], 'talked')
        for (const term of ['walking', 'walked', 'talking']) assert.ok(!hits.includes(term), `${term} must not be a hit`)
      },
    ],
  },
  {
    file: 'examples/external-embeddings.mjs',
    checks: [
      out => assert.ok(out.includes('Ingested 4 Float32Array vectors.')),
      out => assert.ok(out.includes('vec-cache')),
      out => assert.match(out, /Top vector matches for external query embedding:\r?\n\s+vec-auth \(similarity=0\.99/),
    ],
  },
]

for (const { file, checks } of examples) {
  test(`standalone Node example runs cleanly: ${file}`, () => {
    const result = spawnSync(process.execPath, [join(root, file)], {
      cwd: root, encoding: 'utf8', timeout: 120000,
    })
    assert.equal(result.status, 0, result.stderr || result.stdout)
    for (const check of checks) check(result.stdout)
  })
}
