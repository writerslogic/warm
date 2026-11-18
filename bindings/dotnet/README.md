# WritersLogic.WarmMemory (.NET)

.NET 8 binding over the C ABI: `Store.Open`, `Ingest`, `Search` and `*Async` forms.

```sh
cargo build --release --manifest-path bindings/c/Cargo.toml
WARM_LIBRARY_PATH="$CARGO_TARGET_DIR/release" dotnet test bindings/dotnet/tests/WarmMemory.Tests
```

Add `CARGO_TARGET_DIR` (or use `bindings/c/target`) so `WARM_LIBRARY_PATH` points at the built library. The runnable usage is `SearchRoundTrip` in [`StoreTests.cs`](tests/WarmMemory.Tests/StoreTests.cs).

| Item | Value |
|------|-------|
| Project | `src/WarmMemory/WarmMemory.csproj` (net8.0) |
| Native library lookup | `WARM_LIBRARY_PATH`, next to the assembly, `runtimes/<rid>/native/`, OS path |
| Release | source only; not on NuGet |

Usage and packaging: [docs/runtimes/dotnet](../../docs/runtimes/dotnet/README.md).
