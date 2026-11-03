# Kotlin (JVM and Android)

`bindings/kotlin` is a Gradle build with two library modules over the UniFFI crate in
`bindings/uniffi`, plus a Compose sample:

These modules are available as source in this repository; no packaged JVM or Android release is published yet. Build the native libraries locally before using them.

| Module | What | Native library |
| --- | --- | --- |
| `:jvm` | Kotlin/JVM library; JNA loads the library from classpath resources (`<os>-<arch>/`) | host, via `scripts/build-jvm-native.sh` |
| `:android` | Android AAR (minSdk 24), `jniLibs` for `arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86` | `scripts/build-android.sh` (cargo-ndk) |
| `:sample-compose` | Minimal Jetpack Compose note-search app | from `:android` |

The generated bindings (`jvm/src/main/kotlin`, `android/src/main/kotlin`) are committed; the two
flavors differ only in the Android-safe cleaner. Regenerate with `scripts/generate.sh`. The API
has no Android or JVM types, so the sources are a ready starting point for a Kotlin Multiplatform
`expect/actual` wrapper.

## Requirements

- JDK 17, Gradle (the wrapper pins 8.14.6; AGP 8.13.2, Kotlin 2.2.21).
- Rust stable 1.89 or later and `rustup`; the scripts add the Rust targets (user-level).
- Android: SDK platform 35, build-tools 35, **NDK r27.3** (`27.3.13750724`; r26 or later should
  work), `cargo install cargo-ndk --locked`. Put the SDK path in `bindings/kotlin/local.properties`
  (`sdk.dir=...`) or `ANDROID_HOME`; the NDK is found under `$ANDROID_HOME/ndk` or `ANDROID_NDK_HOME`.

## Build and test

```sh
cd bindings/kotlin
scripts/bootstrap-gradle-wrapper.sh # once: fetch the checksum-pinned gradle-wrapper.jar
scripts/build-jvm-native.sh        # host library into jvm/build/natives
./gradlew :jvm:test                # JUnit on the JVM
scripts/build-android.sh           # four ABIs into android/src/main/jniLibs
./gradlew :android:assembleRelease :sample-compose:assembleDebug
./gradlew :android:connectedAndroidTest   # needs a device or emulator
```

## Use

```kotlin
val store = WarmStore.open(
    File(filesDir, "warm").absolutePath,
    StoreOptions(dimensions = 4096u, encryptionKey = keyFromKeystore))

store.ingest(DocumentInput(id = "note-1", text = "Title\nBody ...", titleWeight = 3u))
val hits = store.search("title", SearchOptions(k = 10u, bm25K1 = 1.2, bm25B = 0.75))
store.snapshot(backupDir.absolutePath)
val restored = WarmStore.restore(backupDir.absolutePath, otherDir.absolutePath, sameOptions)
```

Disk-touching calls are `suspend` functions that run on a worker thread, with `…Blocking`
variants. A store is thread-safe. A directory opens once per process (`WarmException.StoreLocked`);
`shutdown()` releases it at once, `close()` (AutoCloseable) drops the handle. Errors are the
sealed `WarmException` (`InvalidInput`, `StoreLocked`, `Encryption`, `IncompatibleStore`,
`NotFound`, `AlreadyExists`, `Corrupt`, `Io`, `Closed`, `Internal`; each carries `details`).
Metadata and filters are JSON object strings (`metadataJson`, `filterJson`).

Proguard/R8: the AAR ships `consumer-rules.pro` keeping JNA and the binding package.

## Verified

- `./gradlew :jvm:test` on macOS arm64 (JDK 17): 7 JUnit cases pass (typed errors, encryption with
  a plaintext scan of the store, snapshot/restore with tamper detection, suspend forms under
  concurrency, BM25 options, `titleWeight`, filters).
- A fresh AAR consumer on an arm64-v8a emulator (Android 15, API 35, running on the Apple
  silicon host) passes nine lifecycle tests, including the two original Android cases.
- Plaintext and encrypted stores pass forced process termination after completed snapshots,
  including reopening and further writes; ten native locking/path checks also pass.
  Source, artifacts, failures and exact scope are recorded in the
  [qualification evidence](../../research/milestones-20261102/PLATFORMS.md).
- `:android:assembleRelease` produces the AAR; `:sample-compose:assembleDebug` builds the sample,
  which installed on that emulator, launched, and created its store.
- All four ABIs build with cargo-ndk (NDK r27.3, API 24).

Not verified: execution of `armeabi-v7a`, `x86` and `x86_64` libraries (built and packaged, not
run), a physical device, and Android versions other than 15. The macOS host library defaults to
`strip = none`: both symbol and debuginfo stripping can produce misaligned Mach-O libraries
([Rust issue #157750](https://github.com/rust-lang/rust/issues/157750)). Android builds on macOS
disable stripping for host build dependencies through Cargo's
[`build-override`](https://doc.rust-lang.org/cargo/reference/config.html#profilenamebuild-override);
Android libraries and Linux host libraries retain the release profile's stripping policy.

Sizes and timings: `benchmarks/results/mobile_v1.json`.
