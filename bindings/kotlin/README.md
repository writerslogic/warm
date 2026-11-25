# Kotlin binding (JVM and Android)

Gradle build over the UniFFI crate: `:jvm` library, `:android` AAR, `:sample-compose` app.

```sh
cd bindings/kotlin
scripts/bootstrap-gradle-wrapper.sh && scripts/build-jvm-native.sh
./gradlew :jvm:test          # runs WarmStoreTest
```

| Item | Value |
|------|-------|
| Needs | JDK 17, Rust 1.89+; Android adds SDK 35, NDK r27, `cargo-ndk` |
| Release | source only |

Modules, Android build and usage: [language bindings](../../docs/integrations.md#language-bindings).
