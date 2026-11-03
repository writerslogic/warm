// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'

export async function* readRows(stream, maxLineBytes = 64 * 1024 ** 2) {
  assert.ok(Number.isSafeInteger(maxLineBytes) && maxLineBytes > 0 && maxLineBytes <= 64 * 1024 ** 2)
  let parts = [], bytes = 0
  for await (const chunk of stream) {
    assert.ok(Buffer.isBuffer(chunk))
    for (let start = 0; start < chunk.length;) {
      const newline = chunk.indexOf(10, start)
      const end = newline < 0 ? chunk.length : newline
      parts.push(chunk.subarray(start, end)); bytes += end - start
      assert.ok(bytes <= maxLineBytes, 'trace line exceeds allocation limit')
      if (newline < 0) break
      const line = Buffer.concat(parts, bytes).toString('utf8')
      parts = []; bytes = 0
      yield JSON.parse(line)
      start = newline + 1
    }
  }
  if (bytes) yield JSON.parse(Buffer.concat(parts, bytes).toString('utf8'))
}
