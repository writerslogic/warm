// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import { isBuiltin, registerHooks } from 'node:module'

registerHooks({
  resolve(specifier, context, nextResolve) {
    // Node's fetch implementation may load its own built-ins; SDK/userland imports may not.
    if (isBuiltin(specifier) && !context.parentURL?.startsWith('node:')) {
      throw new Error(`Portable consumer imported a Node runtime module: ${specifier}`)
    }
    const resolved = nextResolve(specifier, context)
    if (/\.node(?:$|[?#])/.test(resolved.url)) {
      throw new Error('Portable consumer loaded a native addon')
    }
    return resolved
  },
})
