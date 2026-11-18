# wasm browser demo

A static note-search page on `@writerslogic/warm-wasm`: add notes or text files, search with BM25 settings, persist to OPFS or IndexedDB (optionally encrypted with a passphrase), download or restore a snapshot. A service worker caches the app so it reloads offline.

```sh
cd bindings/wasm/npm && npm install && node scripts/build.mjs     # once
node examples/wasm/serve.mjs [port]                               # prints the localhost URL
```

| Item | Value |
|------|-------|
| Files | `index.html`, `app.js`, `sw.js`; `serve.mjs` serves them and the built package at `/pkg/` |
| Needs | `wasm32-unknown-unknown` target, a `wasm-bindgen` CLI matching the lockfile, Node 20+, a modern browser |
| Secure context | localhost qualifies; OPFS, IndexedDB and service workers need it |

Package API and storage adapters: [bindings/wasm/npm](../../bindings/wasm/npm/README.md). Runtime notes: [docs/runtimes/wasm](../../docs/runtimes/wasm/README.md).
