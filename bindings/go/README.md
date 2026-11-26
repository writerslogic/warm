# Go binding

Module `github.com/writerslogic/warm/bindings/go`, package `warm`: document ingest and BM25 search over the C ABI (cgo, static link).

```sh
bindings/go/scripts/build-native.sh      # builds and installs the static C library for this GOOS/GOARCH
cd bindings/go && go test ./...          # runs Example() in example_test.go, which is the quickstart
```

| Item | Value |
|------|-------|
| Quickstart | [`example_test.go`](example_test.go) (`Example`, output `rust`) |
| Needs | Go 1.24+, Rust 1.89+, a C compiler |
| Release | source only; no tagged module yet |

Use from your own module, API and context semantics: [language bindings](../../docs/integrations.md#language-bindings).
