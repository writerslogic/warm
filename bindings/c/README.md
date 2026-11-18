# writerslogic-warm-c

C ABI over the WARM document store (BM25 + optional embeddings, encrypted at rest); Go, Java and .NET wrap it.

```sh
cargo build --release --manifest-path bindings/c/Cargo.toml     # libwriterslogic_warm_c.{a,dylib,so}
```

Run the quickstart ([`examples/quickstart.c`](examples/quickstart.c), prints `harbor 1.000`):

```sh
bindings/c/tests/c/run.sh        # builds, compiles quickstart.c and the test program, runs both
```

| Item | Value |
|------|-------|
| Header | `include/writerslogic_warm.h` (cbindgen, `scripts/gen-header.sh`) |
| Symbols | `hms1_*`, unversioned `warm_*` aliases |
| Build needs | Rust 1.89+, a C11 compiler |

Contract, surface and packaging: [docs/runtimes/c](../../docs/runtimes/c/README.md).
