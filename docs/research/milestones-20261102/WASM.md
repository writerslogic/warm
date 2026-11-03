# Installed WebAssembly package qualification

The macOS arm64 run passes for the installed `@writerslogic/warm-wasm@0.1.0`
archive, including the bounded filtered-query change. It completed at
2026-11-03T14:24:19Z with no failed or skipped cases in the declared final gate.

| Surface | Pinned runtime | Passing checks |
|---|---|---|
| Installed server runtimes | Node 22.23.4, Deno 2.9.6, Bun 1.4.1 | 18 public lifecycle tests each; packed imports and TypeScript checks |
| Browsers | Chrome 156.0.8078.80, Firefox 159.0 | Seven cases each, including OPFS/IndexedDB persistence and offline demos |
| Bundler | webpack, pinned npm lock | Two entry points exercised in Chrome |
| Rust browser unit tests | Rust 1.96.0, wasm-bindgen 0.2.129 | Four cases in each browser |
| Process-death recovery | All three server runtimes | 18 killed writers, 54 successful cross-runtime readers |

The shared tests cover ingest/search/replace/delete, atomic batches, damaged
exports, snapshot import, failed and concurrent persistence, encryption changes,
bounded KDF parameters, and dense scalar/SIMD agreement. The added filtered
regression compares complete results against an untrained reference through
training, missing embeddings, eligibility changes, deletion/insertion, retraining
and snapshot reopening, for both fusion modes and scalar/automatic SIMD.

For each runtime, encryption mode (plaintext/PBKDF2/Argon2id), and signal
(SIGKILL/SIGTERM), the writer acknowledges saves after 32 inserts, 16 replacements
and eight deletions. The parent fsyncs its acknowledgement ledger, kills the
still-open writer, then gives each runtime its own copy of the crash image.
All readers verify 24 surviving records, their content/revision, eight absences,
and another write/save/reopen. This verifies completed-save recovery, not arbitrary
mid-save interruption or power loss. The public synthetic passphrase and low
PBKDF2 test iteration count exercise compatibility, not password-hardening strength.

The first attempt exposed a Node 22 test-script defect: a directory argument to
`node --test` was treated as a module path. The repository script now names the
test file explicitly. Attempts two and three exposed a harness configuration
conflict between the TypeScript bundler consumer and Deno. The final run uses a
separate installed Deno consumer with pinned Node types and typechecking enabled;
the proposed Deno package configuration was removed after the existing source
suite passed. An initial new-test assertion also confused current-generation
index status with retention of a base tree plus delta; that assertion was fixed
after inspecting the API contract. All failure logs remain in the raw artifacts.

All eight scalar/SIMD WASM targets were built in v1. V4 reuses their byte-verified
outputs because production source was unchanged; its changes are the Node test
command, shared regression and consumer harness. The complete v4 freeze records
494 source/lock files and reused artifacts. Its inherited reuse-description field
mentions only the earlier script correction; the separate `testChanges` field
and source hashes also record the later test/harness changes.

Raw root: `/Volumes/A/.hms-bench-cache/warm-wasm-qualification-20261103-v4`.
Freeze SHA256:
`23c68fb90c1cce24b4ce4cf3505da6986cdb4789bab3742a5a6a85dca44b3e99`.
Archive: `packed/writerslogic-warm-wasm-0.1.0.tgz`, SHA256
`84da9d6eafb7917f0a140c83300bed689070a4c200846b3b4c51aefba92ce1b4`.
All 64 installed files, source files, archive members, logs and recovery ledgers
were hash-verified. Receipts: `wasm-v4-{freeze,finished,recovery}.json`; runner:
`mac-wasm-qualification.py`; plan: `benchmarks/public/wasm_qualification/PLAN.json`.

This does not qualify Linux/Windows runtime hosts, Safari, other advertised
bundlers, the minimum Node version, legacy-format migration or optional in-WASM
AES. Those remain part of the open platform/migration matrix in
[PLATFORMS.md](PLATFORMS.md). Snapshot round trips alone do not prove migration.
