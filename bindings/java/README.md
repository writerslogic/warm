# Java binding

Maven artifact `com.writerslogic:writerslogic-warm`, version 0.1.0: Foreign Function & Memory API over the C ABI, no JNI.

```sh
cargo build --release --manifest-path bindings/c/Cargo.toml
cd bindings/java && mvn -Dwarm.native.dir="$CARGO_TARGET_DIR/release" test
```

The runnable usage is [`StoreTest.java`](src/test/java/com/writerslogic/warm/StoreTest.java).

| Item | Value |
|------|-------|
| Needs | JDK 22+ (FFM is final in 22), Maven, Rust 1.89+ |
| Run flag | `--enable-native-access=ALL-UNNAMED` |
| Release | source only; not on Maven Central |

Usage and library resolution: [docs/runtimes/java](../../docs/runtimes/java/README.md).
