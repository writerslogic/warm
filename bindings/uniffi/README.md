# warm-mobile (UniFFI)

`warm-mobile` is the UniFFI crate under the Swift and Kotlin bindings. It is a standalone
workspace and is not published; nothing here is meant to be used directly. To get to a first
query, start from the binding for your platform:

- Swift (iOS, macOS, Mac Catalyst, visionOS): [swift binding](../swift/README.md), package in `bindings/swift`.
- Kotlin (JVM and Android): [kotlin binding](../kotlin/README.md), Gradle build in `bindings/kotlin`.

Both build this crate for their targets. To work on the crate itself:

```sh
cargo test --manifest-path bindings/uniffi/Cargo.toml                 # tests/store.rs
bindings/uniffi/scripts/generate-bindings.sh swift  /tmp/warm-swift   # or: kotlin /tmp/warm-kotlin
```

`generate-bindings.sh` builds a debug host library unless `LIBRARY=<path>` names one, and reads
the UniFFI metadata from it; the metadata is the same for every target built from this source.
