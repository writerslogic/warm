# Swift binding

Swift Package `WarmMemory` over the UniFFI crate: iOS, macOS, Mac Catalyst, visionOS. Runs on the device.

```sh
bindings/swift/scripts/build-xcframework.sh     # SLICES="macos" builds only the host slice
cd bindings/swift && swift test
```

| Item | Value |
|------|-------|
| Sample app | [`Examples/WarmSample`](Examples/WarmSample) (SwiftUI note search; `swift run` on macOS) |
| Needs | macOS, Xcode 15+ (full Xcode for iOS slices), Rust 1.89+ |
| Release | source only; build the XCFramework locally |

Usage, errors and encryption: [docs/runtimes/swift](../../docs/runtimes/swift/README.md).
