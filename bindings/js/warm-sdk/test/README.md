# Portable client contract tests

Run with Node 22.15 or newer. The packed consumer uses the [synchronous module hooks](https://nodejs.org/download/release/v22.15.0/docs/api/module.html#moduleregisterhooksoptions) to reject Node runtime imports and native addons in both ESM and CommonJS consumers. The SDK itself uses browser standard APIs; these tests do not exercise browser execution.

From the repository root, after building the SDK and the same-commit server:

```sh
cargo build --locked --features server --bin warm-server
npm --prefix bindings/js/warm-sdk run build
WARM_SERVER_BIN="$PWD/target/debug/warm-server" npm --prefix bindings/js/warm-sdk test
```

`npm test` runs transport, real-server, packed-consumer and installed TypeScript consumer tests. `WARM_SERVER_BIN` is mandatory; there is no skip for a missing binary. The fixture starts that binary on an ephemeral loopback port with dimension 24, temporary data and a fixture API key. It does not inherit service credentials or require model weights. Core queries supply vectors; memory retrieval exercises the server's lexical path.

The type gate uses the SDK's locally installed TypeScript compiler on external `.mts` and `.cts` consumers under NodeNext with ES2022 and DOM libraries, `types: []`, strict checking and `skipLibCheck: false`. It resolves the packed package exports and checks valid response types, nullable lookups, rejected native exports, snake_case options and invalid forget scopes. It is separate from the registry-only `test:package` command.

The packed test uses `npm pack --ignore-scripts`, then installs the tarball into a temporary project outside the source tree with install scripts disabled. Its ESM and CommonJS consumers each exercise the real HTTP server. To verify an already-published candidate instead:

```sh
WARM_SERVER_BIN="$PWD/target/debug/warm-server" WARM_SDK_PACKAGE=warm-sdk@0.2.0 npm --prefix bindings/js/warm-sdk run test:package
```

Registry mode requires the exact local manifest version, installs with `--ignore-scripts --save-exact --no-audit --no-fund`, then runs `npm audit signatures` and compares installed lock integrity against the registry's `dist.integrity`. It needs no local SDK build or dev dependencies. Both modes reject runtime, optional and peer dependencies and inspect the installed dependency tree. Temporary npm configuration avoids reading the user's registry credentials.

Transport fixtures use real loopback HTTP servers for redirects, error bodies, cancellation, byte boundaries and connection resets. Their 16 MiB request acceptance proves the client's transport limit; the real server also applies its separate 8 MiB aggregate batch-source limit. Semantic assertions come from the real server, not fixture responses. Contracts follow `docs/server/openapi.json`, `src/bin/warm-server`, `src/core/memory_api/json.rs` and the Python SDK live tests.
