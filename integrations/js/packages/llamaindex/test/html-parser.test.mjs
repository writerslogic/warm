// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { HTMLNodeParser } from '@llamaindex/node-parser/html'
import { Document } from '@llamaindex/core/schema'

test('HTML conversion preserves stored text and custom selectors after the dependency upgrade', () => {
  const parser = new HTMLNodeParser({ htmlToTextOptions: {
    selectors: [{ selector: 'a', options: { ignoreHref: true } }, { selector: '.omit', format: 'skip' }],
  } })
  const document = new Document({ id_: 'html-source', text:
    '<p>Tom &amp; Ana &#x1F642;</p><p><a href="https://example.invalid/">link</a>' +
    '<span class="omit">hidden</span></p><script>never execute()</script>',
  })
  const nodes = parser.getNodesFromDocuments([document])
  assert.equal(nodes.length, 1)
  assert.equal(nodes[0].text, 'Tom & Ana 🙂\n\nlink')
  assert.equal(nodes[0].sourceNode.nodeId, 'html-source')
})

test('HTML conversion ignores prototype-supplied option values', () => {
  const htmlToTextOptions = JSON.parse('{"__proto__":{"uppercaseHeadings":false,"wordwrap":1},"selectors":[]}')
  const parser = new HTMLNodeParser({ htmlToTextOptions })
  const clean = new HTMLNodeParser({ htmlToTextOptions: { selectors: [] } })
  const document = new Document({ text: '<h1>Heading stays uppercase</h1><p>Text stays intact.</p>' })
  assert.equal(parser.getNodesFromDocuments([document])[0].text, clean.getNodesFromDocuments([document])[0].text)
  assert.equal(Object.prototype.uppercaseHeadings, undefined)
  assert.equal(Object.prototype.wordwrap, undefined)
})
