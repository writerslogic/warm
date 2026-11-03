# Supported-platform qualification inventory

The initial Android follow-up was frozen at
`/Volumes/A/.hms-bench-cache/warm-android-qualification-20261103-v1`, PID 48199,
after the owner-membership performance screen completed. Plan:
`benchmarks/public/mobile_qualification/ANDROID-PLAN.json`; runner:
`mac-android.py`; 508-file freeze SHA256:
`81d6a82c35ef2ad8ded082c3e318d3e3e00c0ab3022c762d7762f521b1147bdc`.
It builds all four Android ABIs, tests fresh JAR/AAR consumers (seven JVM and
nine Android lifecycle cases), and declares two Android process-death cases
after completed snapshots. Execution is scoped to API 35 arm64; other packaged
ABIs remain build-only. The final V6 result is recorded below. The run also verifies the
Kotlin scripts' fix for the already-reproduced Mach-O stripping failure: no
stripping on the macOS JVM library, and a host-only build override for Android.
Emulator commands and force-stop behavior follow the official
[emulator](https://developer.android.com/studio/run/emulator-commandline),
[instrumentation](https://developer.android.com/studio/test/command-line) and
[ADB](https://developer.android.com/tools/adb) documentation.

V1 passed the seven JVM tests and built all four Android ABIs, then rejected the
AAR because it assumed packaging preserved pre-strip bytes. Independent
`llvm-strip --strip-unneeded` exactly reproduces every packaged library and
Gradle's stripped output; V2 models that transformation and retains both hashes.
V2 then executed all nine Android cases: eight failed opening a store with
`try_lock() not supported`; only chunking passed. This is a product blocker,
not a skipped Android test. The owned emulator shut down successfully.

The pinned Rust 1.96 standard-library source excludes Android from file locking,
matching upstream [issue #148325](https://github.com/rust-lang/rust/issues/148325).
The pinned NDK's `sys/file.h` supplies `flock`. The shared file helper now uses
nonblocking exclusive `libc::flock` and explicit unlock on Android; store and
vector-index ownership, plus the experimental durable graph, use that helper.
Other platforms retain the standard-library operations. Ten focused host checks
and strict library clippy with `security,experimental` pass, including native
lock interoperability, cloned-descriptor lifetime and protected-path rejection.
V3 freezes the fix, rebuilds both packages and all four ABIs, and adds those ten
native checks inside Android before retrying the unchanged nine lifecycle and
two completed-snapshot process-death cases. The V3–V6 outcomes follow below.
Receipts: `android-v{1,2}-finished.json`, `android-v1-strip-diagnostic.json`,
`android-v2-lifecycle-failure.log`, `android-lock-host-{tests,clippy}.log` and
`android-v3-{freeze,launched}.json`.

V6 completed successfully at 2026-11-03T17:24:01Z: seven JVM tests, all ten native
Android checks, all nine Android app lifecycle tests, plaintext/encrypted writer
termination and recovery, and Compose sample installation/startup/store creation.
All 509 source files, assets, installed artifacts and logs reverified; the owned
emulator is stopped. The package/runtime builds are reused unchanged from V3;
V4 added the omitted test-only navigation module and compiled the native test
binary. V4's runner could not discover it because cargo-ndk consumes Cargo
artifact messages; V5 pins that fresh AArch64 binary directly. V5 passed the two
new lock tests, but shell UID 2000 could not create FIFO/hardlink attack fixtures.
V6 runs all ten unchanged native checks under the owned emulator's debug root,
then verifies UID 2000 and SELinux Enforcing before app tests. SELinux stayed
enforcing; host/other-device security settings were unchanged. No test was dropped.

Each recovery case fsynced the three validated acknowledgements into the host
ledger before `am force-stop`, verified the writer PID was gone, and reopened
24 surviving records with eight deletions, correct replacements and a successful
new write/snapshot/reopen. This is the completed-snapshot boundary only.
Freeze SHA256: `538d4e7dfd658472b311072fc8d39da364bdfd03171460da04eb17dd7d7125a7`.
AAR SHA256: `d7786be3130e1e830c701c26ab1adbf73a0be35846908cba29d619de1636427d`.
JAR SHA256: `903d234ad41e957e1c90533ac6536cd193a624561f31e78ac0a75118018e550e`.
Full receipts: `android-v6-finished.json`, `android-v6-recovery-{plain,encrypted}.json`
and `android-v6-native-fixture-privileges.json`. Physical devices, non-arm64
execution, other Android releases and migration remain open.

This records scoped results and remaining required evidence. The frozen
research source starts at `ab978afd`; current uncommitted milestone changes need
their own build/source receipts. Existing workflow definitions describe coverage
intent and cannot substitute for passing runs against that source.

The current native library passed all 893 functional tests with
`security,conversation-memory,experimental` on macOS arm64/Rust 1.96.0 at
18:25Z. All twelve ignored cases were reviewed against source: eleven timing or
memory experiments and one external-index memory census; none is recorded as
passing. The v1 bundle omitted the vector-format golden directory and retained
892 passes/one missing-file failure. V2 adds all thirteen tracked fixture files,
verified byte-for-byte against HEAD, without regenerating them or changing code.
Raw root: `/Volumes/A/.hms-bench-cache/warm-core-qualification-20261103-v2`;
522-file freeze SHA256:
`cbf253a0d0d01b6875373246e24ddcc51deabf5ba0d96f8f6a16973ed3856bf8`.
Receipts: `core-v2-finished.json` and `core-v2-ignored-review.json`.
Other feature combinations and platform gates remain separate requirements.

Current `local-models` source and all three existing Qwen weight sets are now
hash-verified and frozen (`local-models-v1-freeze.json`). The release server/reader
build and all 32 model-unit tests pass; the two conversation-model cases are
still running, explicitly including the weight-dependent test. Raw root:
`/Volumes/A/.hms-bench-cache/warm-local-models-qualification-20261103-v1`.
This establishes no HTTP model lifecycle, generator/reranker workflow or QA advantage.

Six optional HTTP SDK adapters passed frozen installed-wheel qualification:
LangChain, LlamaIndex,
Haystack, CrewAI, Pydantic AI and smolagents. The unchanged repository `uv.lock`
supplies hash-pinned dependencies for Python 3.12.12. Tests use local embeddings,
Pydantic AI's `FunctionModel`, and a real frozen server. They execute as the normal
guest user inside a private network namespace containing only loopback; a direct
check verified working loopback and an unreachable external route. This requires
no model API or cloud calls. Every frozen test function must execute with zero
skips, failures or errors. All 94 cases from 33 test functions passed, with zero
skips/failures/errors. Both DSPy adapters now also pass the scoped checks below;
the remaining separate integration modules remain open.

A DSPy follow-up passed at
`/Volumes/A/.hms-bench-cache/warm-dspy-qualification-20261103-v2` at 18:21Z.
It packages both current Python adapters as fresh wheels and declares nine HTTP
SDK cases plus the separate integrations package's eleven existing DSPy cases.
The new SDK cases exercise native predictions, batched queries, requested-limit
boundaries, replacement/deletion, tenant separation and error propagation.
The existing integration cases include async calls, optimizer-style copying,
ReAct with DummyLM and a fresh-process DiskCache rejection guard.

The repository integration lock predates the current Filelock >=4.2.6 metadata:
it contains 3.32.7/4.2.4 and LiteLLM 1.106.4. The qualification environment instead
has a separate uv lock for the already documented DSPy 3.4.1 / LiteLLM 1.103.10 /
Filelock 4.2.6 profile. Its test processes had only loopback networking and
local embeddings/DummyLM; no model API is authorized or required. Runner:
`linux-dspy-qualification-v2.py`. The 133-file freeze SHA256 is
`fa1581edd06e9485b473796e4a83ff3826a68b3a04cca29ac7040c981c7f8408`;
archive SHA256 is `935a0347fb561bff871506d515d7599a7e2738716fe39d1793d7270896bfa4ac`.
All nine SDK cases and eleven integration cases passed, without skips, errors or
failures, against freshly installed verified wheels on Linux arm64/Python 3.12.12.
The v1 run retained seven passes and two failures: its test incorrectly expected
HTTP top_k values 999 and 1000 to succeed, although the server's default maximum
is 100. V2 changes only that test's boundary values to 99/100/101 and its runner's
output directory; product wheels and dependency pins are unchanged. Both raw
result directories and `dspy-v1-finished.json`/`dspy-v2-finished.json` are retained.
These checks completed before serving measurements.

The fifteen-profile integration qualification completed after serving. V1
exposed a missing mem0 `_UNSET` sentinel and two stale limit assertions in the
Haystack/LlamaIndex tests. V2 fixes the sentinel, checks omission versus explicit
expiry clearing through six sync/async regression cases, and accepts exactly the
advertised enumeration limit while rejecting limit+1. It reuses v1's exact
per-profile dependency locks and hashed requirements with freshly installed wheels.

All 391 collected cases completed: 389 passed, zero failed/errors, and two
Haystack upstream optional `recreate_index`/`recreate_collection` cases skipped
because WARM exposes neither parameter. Ordinary delete-all conformance passes;
these two exclusions are explained, not counted as executed passes. The raw
strict no-skip gate remains false. Ordinary-user consumers used loopback-only
networking and local/scripted models; no paid services were used. V1 failures
remain preserved. Receipts: `integrations-v1-finished.json`,
`integrations-v2-finished.json` and `integrations-v2-reviewed.json`. Raw v2 root:
`/Volumes/A/.hms-bench-cache/warm-integrations-qualification-20261103-v2`.

Source: `/Volumes/A/.hms-bench-cache/warm-sdk-frameworks-20261103-v1`; guest
`sdk-frameworks-v1`; launcher PID 28758. The 12-file archive SHA256 is
`4cf7d14b39426da594328d7922cbcf8a58948db5677b6de11babf22a0cf3e42b`.
Receipts are `sdk-frameworks-source.json`, `sdk-frameworks-launcher.pid` and
`sdk-frameworks-network-check.log`. Guest results will be in
`evidence/sdk-frameworks-v1/finished.json`; a local copy is
`sdk-frameworks-v1-finished.json`.

The base installed HTTP SDK passes all 12 live tests and both process-recovery
cases on each of Python 3.11.14 and 3.14.7, with all 60 installed files verified.
V1 failed one newly added test because it expected HTTP 400 for an unsupported
contract version; existing server code maps `MemoryError::Invalid` to 422.
V2 corrects that expectation without changing SDK or server bytes. Receipts:
`python-sdk-live-v{1,2}-finished.json` and `python-sdk-live-v2-{311,314}-recovery.json`.

The macOS arm64 native Python source distribution and abi3 wheel built, and each
fresh consumer on Python 3.9.6/3.12.12/3.14.7 passed 119 binding tests with zero
skips plus the 16-query/resonator smoke check. Receipt: `python-native-v1-finished.json`.
NumPy 2.0.2 emits matmul floating-point warnings on the first two runtimes;
[NumPy's upstream fix](https://github.com/numpy/numpy/pull/29223) identifies an
M4 SME/status-register interaction. A separate frozen audit compares every
affected oracle score against non-BLAS einsum and sampled `math.fsum`, requiring
finite results and identical top-10 IDs before relying on those oracle results.
That audit passed under `warm-python-native-oracle-audit-20261103-v1`: 3,204,096
score comparisons per runtime are finite, differ by at most 8.53e-14 from the
non-BLAS reference, and produce identical top-10 IDs. Three identity-matrix cases
also remain exactly correct despite warnings in NumPy 2.0.2. NumPy 2.5.4 emits no
warnings. The original warnings/tests are retained; none was suppressed to pass
qualification. Receipts: `python-native-oracle-audit-finished.json` and
`python-native-oracle-audit-{39,312,314}.json`.

A macOS arm64 C-family check is also frozen and queued after both native Python
and the guest framework checks finish. It builds a separate Rust target, packages
the C static/shared libraries, executes the public C consumer against extracted
headers/libraries, runs Go's race-enabled tests from a fresh source consumer, and
tests fresh Java jar and .NET NuGet consumers using bundled native libraries.
The existing binding workflow selects Go 1.24, Java 25 and .NET 8; this run pins
Go 1.24.13, Temurin 25.0.5+8, Maven 3.9.16, installed .NET 8.0.130 and Rust 1.96.0.
Official archive checksums were verified, and tools are isolated under the run
directory. No global toolchain was replaced. Existing tests must run without
skips; explicit process-death/migration coverage and other target OSes remain open.

Run: `/Volumes/A/.hms-bench-cache/warm-c-family-qualification-20261103-v1`;
launcher PID 20356. Its 492-file source freeze SHA256 is
`e00b3af33801f62edfe51e2107173b28ec4b0e9b628d0d98f61570b554e10951`.
Runner: `mac-c-family.py`; receipts: `c-family-freeze.json` and
`c-family-launcher.pid`; terminal result: run-directory `finished.json`.
V1 stopped at Rust version verification because the isolated environment omitted
configured Rust home variables. V2 fixed that but `cargo --locked` exposed a stale
C-binding lockfile: the root package lacked its already-declared `windows-sys`
dependency edge. The repair adds that one edge without changing any resolved
package version/checksum. V3 waits for the filtered-tree diagnostic before running.
Current run suffix is `v3`, PID 29644, freeze
`3bb413fd31fe3f680bac6f227591f46a63e37db8d3e437b826a38bf2ab03eb1f`;
receipts `c-family-v3-{freeze.json,launcher.pid}`. V3 passed at 12:59:12Z: five
Rust ABI/header/leak cases, both extracted C consumers, 15 Go cases with race
checks, 30 installed Java cases and 19 installed .NET cases; zero skips/failures.
All frozen source and package digests were reverified. Terminal receipt:
`c-family-v3-finished.json`.

Native process-death checks are separately frozen under
`/Volumes/A/.hms-bench-cache/warm-native-binding-recovery-20261103-v1`, launcher
PID 27181. Each C/Go/Java/.NET writer flushes a 32-document insert, 16 replacements
and eight deletions, then is killed with SIGKILL or SIGTERM after the parent's
acknowledgement ledger is fsynced. All four bindings must read each recovered
store, validate surviving content/metadata and deleted IDs, then flush a new write
and verify it after close/reopen: eight writer cases and 32 cross-language readers.
This covers the declared flushed boundary, not arbitrary mid-mutation or power
failure. It waits for the C-family and filtered-tree diagnostic terminal receipts,
and refuses a failed or changed C-family artifact. C syntax and two protocol tests
pass; the runtime consumers have not compiled or executed yet. Source/protocol:
`benchmarks/public/native_recovery/`. Seven-file freeze SHA256:
`6816a5755e46905b91762fcb494ca1f05c86aa66c3dfe12c7f8b6dacd45ba4c1`.
V1/v2 refused the failed C-family receipts. V3 uses the same recovery protocol
and follows the corrected C-family v3 artifact; PID 29646, freeze
`3b13f9536f0bae29738188b12f10ce02bd5fbfe164dca9b4ae53d77612258ffc`.
Receipts: `native-recovery-v3-{freeze.json,launcher.pid}` and
`native-recovery-protocol-tests.log`; run suffix `v3`, terminal `finished.json`.
V3 completed all eight writer deaths and 32 cross-language readers successfully
at 13:01:20Z, including the post-recovery write/flush/reopen check in every reader.
Receipt: `native-recovery-v3-finished.json`.

The Linux arm64 glibc C-family follow-up is prepared with the same 492-file source
and matching Go 1.24.13, Temurin 25.0.5+8, Maven 3.9.16 and .NET SDK 8.0.130 tools.
Official archive checksums were verified locally. Host coordinator PID 27323 waits
for successful Mac package and cross-language recovery results before transferring
or compiling anything in the guest. It also requires the guest framework terminal
receipt. Guest target will be
`/home/davidcondrey.guest/warm-milestones-20261103/c-family-v1`; no guest run exists yet.
Local prepared run:
`/Volumes/A/.hms-bench-cache/warm-linux-c-family-qualification-20261103-v1`.
Source freeze SHA256:
`190abc53131ac83ab0854e94ab1f42ee43a263e657d14f9fc13983a97e601700`.
The 441,743,360-byte transfer archive SHA256 is
`552d1c9a9e0e7cc1ee6de28572562edabcdb2a8f4a2b4c089d6c3d2c91c92a09`.
Runners: `linux-c-family.py`, `linux-c-family-launch.py`; receipts:
`linux-c-family-{freeze,transfer}.json`, `linux-c-family-coordinator.pid`.
The host's `launch-finished.json` records staging only; the guest's `finished.json`
will record the actual API/package outcome. V1's coordinator stopped after the
Mac prerequisite failure; its archive remains prepared but no guest run started.
It must be replaced with a new freeze carrying the corrected C lockfile and
successful current prerequisite receipts. V2 supplied the same one-edge lockfile
repair and explicitly added the existing pinned Rust tool directory to guest PATH.
It passed at 13:04:50Z with the same five Rust, two C, 15 Go, 30 Java and 19 .NET
checks, zero skips/failures. Source freeze:
`c2e8fa47d9ff6b415910b9a0a63dd1f5e2fe7fa308ce2e3a149d976f004e7746`;
transfer SHA256 `7e12edaf54d4396573ea96c7c7d535be184200e610a3ab8258df2d6c69498404`.
Guest `c-family-v2`; receipts `linux-c-family-v2-{freeze,transfer,finished}.json`.
The same eight-writer/32-reader recovery protocol is now running separately in
guest `native-recovery-v1`, PID 47250, with unchanged C/Go/Java/.NET consumer code.
Freeze: `175963e81e645d3f3012597242af257713b98a6192992e00a498d512eaa7fdf6`.
Runner `linux-native-recovery.py`; receipt `linux-native-recovery-v1-freeze.json`.
That recovery run passed all eight writer cases and 32 cross-language readers at
13:17:07Z; receipt `linux-native-recovery-v1-finished.json`.

The Linux arm64 native Python qualification passed from the identical 442-file
macOS source freeze: fresh Python 3.9.25/3.12.12/3.14.7 consumers each passed 119
tests and the search/resonator smoke checks, with no skips or numerical warnings.
The source distribution is byte-identical to the macOS build. The produced wheel
is `manylinux_2_34_aarch64`; this does not qualify the advertised manylinux2014
(glibc 2.17) or musl targets. Runner: `linux-python-native.py`; receipts:
`linux-python-native-v1-{freeze,finished}.json`. Freeze SHA256:
`f18bfddd7ca8d17c72af8fe749eda4fbcd9ce2162f88f25bc60eacb8b0ca9c98`.

Windows read-only inventory at 2026-11-03T12:29Z confirms running SSH/Tailscale
services and .NET SDK 8.0.427. The SSH PATH lacks Rust, Node, Go, Java and C/C++
build tools; known user/system install paths also lack Rust/Node/Go, and `vswhere`
reports no installed Visual Studio instance. The WindowsApps Python alias is not
evidence of an installed interpreter. This is the same physical machine as the
Intel WSL soak, so no Windows build or installation was started. Receipts:
`windows-tool-inventory-20261103.json`, `windows-tool-paths-20261103.json`.

| Surface | Advertised or configured targets | Evidence required / available |
|---|---|---|
| Node native package | `package.json` declares macOS arm64/x64; Linux arm64/x64 glibc and musl; Windows arm64/x64 MSVC | macOS arm64 and Linux arm64 glibc / Node 22.23.4: each passes fresh local tarball install, 36 public API tests, type and packed-package checks, migration and process-death recovery. Three local-embed tests per artifact are explicitly outside this standard feature build. Other targets and optional platform-package selection remain open. |
| Python native package | `python-wheels.yml` declares manylinux2014 and musllinux_1_2 arm64/x64, macOS arm64/x64, Windows x64; abi3 minimum Python 3.9 | macOS arm64 and Linux arm64 glibc 2.34 wheels each pass fresh installs, 119 tests and smoke checks on three runtimes. Advertised older glibc, musl, x64 and Windows targets remain open. |
| C, Java, .NET | `bindings.yml` runs Linux x64/arm64, macOS and Windows; C executable tests run only outside Windows | Resolve advertised coverage from each binding's docs, then execute fresh-install lifecycle checks. A configured skip is not evidence that the corresponding advertised workflow works. |
| Go | Same binding workflow, currently excludes Windows | Establish documented support before treating the Windows exclusion as acceptable. Run race-enabled lifecycle tests on declared targets. |
| WASM | Existing tests for Node, Deno, Bun, bundlers and browser | macOS arm64 installed package passes 18 lifecycle tests on each of Node/Deno/Bun, seven cases plus four Rust tests per Chrome/Firefox, two webpack entries, and 18 killed-writer/54 reader recovery checks. Other hosts/browsers/bundlers and legacy migration remain open; see [WASM.md](WASM.md). |
| UniFFI / Kotlin / Swift | Source-built JVM/Android libraries and Swift iOS/macOS/Catalyst/visionOS package | macOS arm64 passes 18 UniFFI tests and seven tests each through fresh Kotlin JAR and Swift XCFramework consumers; iOS 27 arm64 simulator passes seven fresh-consumer tests. Android API 35 arm64 passes ten native checks, nine app lifecycle cases and plaintext/encrypted process-death recovery after completed snapshots. Other Android ABIs are build-only; physical devices, other hosts, legacy migration and remaining recovery surfaces stay open. |
| Rust library and CLI | CI default/all-feature tests, feature-specific clippy, Windows suite, CLI migration/verification contracts | Focused qgraph tests completed; full fresh-install/public lifecycle and recovery matrix remains open. |
| HTTP server and Python HTTP SDK | Local HTTP API plus framework adapters/importers | Linux server: 24 API tests, 13 operations tests, 10 durable-receipt tests passed after restoring omitted archive fixtures. Frozen soak running. macOS arm64 SDK wheel passes base client/transport/memory/evidence/importer-validation contracts on Python 3.11.14 and 3.14.7 (54 tests plus 46 subtests each, zero skips). Live SDK lifecycle and optional integrations remain open. |

For every applicable surface, record installation artifact/source hashes,
toolchain/runtime versions, ingest, search, replace, forget, close/reopen,
migration and documented recovery behavior. Preserve failures and explicitly
explain inapplicable operations rather than silently skipping them.

Available physical configurations are Apple M4/macOS and Intel Core Ultra 9
185H/Windows+WSL. Windows and WSL are one computer. Do not count a cross-compiled
artifact as a successful native lifecycle run. Do not start paid CI or cloud jobs
for preparation or qualification without the user's specific exception to the
paid-development restriction. Avoid adding heavy work to the remote host during
its frozen 24-hour soak.

## macOS arm64 Node package evidence

The artifact uses the same frozen native binary as the shipping accuracy run:
`bd38731775e91b6728661ac23b94fefe55a7509d029b73fe6b69d6c6b43fec7b`, built with
`node-api,security`. Node 22 follows the repository CI matrix. Version 22.23.4
was downloaded from nodejs.org and checked against its published SHA256 file
(`037650d0839f457747396f052073fb48333d31bf5abd3c98b74da20d89ebb1ad` for the
darwin-arm64 archive).

An isolated source package was packed, installed into an empty consumer, and
every installed package file compared byte-for-byte with the staging input.
Existing tests were copied next to the installed artifact; its application files
were unchanged. The first test invocation passed 34 cases and failed two because
the harness omitted the existing legacy-audit golden fixture and temporal example.
Restoring those test-only assets and rerunning just those cases passed both.
The three explicit skips require `local-embed`, which is absent from the standard
build; that optional feature remains a separate qualification item.

Type checking initially lacked TypeScript 7's native executable because the
development install omitted optional dependencies. The repository's ordinary
`npm ci --ignore-scripts` restored it; the same type check then passed. The
existing `npm run test:package` gate also passed. All original frozen input
hashes and installed package bytes still match at completion. These harness
failures and their logs are preserved, not erased or reported as product fixes.

The new [process-death probe](../../../benchmarks/public/node_recovery.cjs) writes
a 32-record batch, supersedes a temporal fact and forgets another. After public
`flush()` completes, its parent fsyncs the acknowledgement and SIGKILLs only the
writer child. A fresh process recovers all 32 batch records, current and historical
facts, the supersession chain and the deletion; retries have no duplicate effects.
This exercises one supported durable boundary, not power loss or every possible
crash location. Separate existing tests cover cancelled/atomic transactions and
Vectra import/reopen behavior.

Raw artifact, package source, installed consumer, exact commands and results:
`/Volumes/A/.hms-bench-cache/warm-node-qualification-20261103-v1`.
Repository copies are `node-freeze.json`, `node-finished.json`, and the
`node-*.log` files here. The receipt distinguishes the local packed binding from
the untested per-platform optional dependency selection mechanism. Milestone 7
remains open for the rest of the advertised matrix.

## Isolated Linux arm64 environment

A new Lima 2.2.0 guest `warm-arm64` runs Ubuntu 24.04.5 on the Mac, with four
virtual CPUs, 8 GiB RAM, no swap and a 48 GiB disk. It is a platform environment
on the existing Apple M4, not another physical hardware configuration. No other
Lima guest was changed. Its private state and disk are under
`/Volumes/A/.hms-lima`; invoke it with `LIMA_HOME=/Volumes/A/.hms-lima`.
No host directories are mounted, and agent/public-key forwarding is disabled.
The [configuration](linux-arm64-lima.yaml) pins the Ubuntu image SHA256.

Lima ignores XDG_CACHE_HOME on macOS and initially downloaded/converts the image
under `~/Library/Caches/lima`. The specific newly used image cache was preserved
at `/Volumes/A/.hms-bench-cache/lima-cache/noble-20261103` with a symlink at its
original per-URL cache path, restoring the original host free space. The cache
path is `download/by-url-sha256/7c8e4507222e57734b580a24bf30b7828afaae43c2f040f6e0977f45669ce961`.
Other cache entries are unchanged. [Lima documents this hard-coded macOS cache](https://lima-vm.io/docs/dev/internals/).

The source archive is baseline `ab978afd` plus the durable-receipt fix and Node
recovery probe. It excludes unrelated CI edits. Source, overlay description,
initial bootstrap, and the Cargo target supplement are frozen under
`/Volumes/A/.hms-bench-cache/warm-linux-arm64-source-20261103-v1`.
Source tar SHA256 is `ddfc3d5fa66afb4317b66742e6724691c489ed9c2f0071ca9e2194d398cc350a`;
the supplemental Cargo example/bench archive is
`8194367e5c1775bc7710a870cda04027f36b33bf3150485fd7457a94aef8fdb6`.
The first build stopped at manifest parsing because those registered target
sources were absent; the supplement supplies all registered external target
paths and their example/bench directories before resuming.

The [bootstrap](linux-arm64-bootstrap.sh) pins Rust 1.96.0, Node 22.23.4 and uv
0.12.10; downloaded rustup and Node archives are checksum-checked. Native builds
and fresh-package checks run under guest `/home/davidcondrey.guest/warm-milestones-20261103`.
The completed build's host log is `linux-arm64-build.log`; guest logs and the
terminal `bootstrap-finished.txt` are under `evidence/`. All 433 frozen source
files were checked against the two archives after building. Every file installed
from the tarball still matched its staged source. Native tests passed 36/36
applicable cases with the same three explained local-embed skips; type, packed API
and process-death recovery checks passed. The server built successfully, but that
alone does not qualify its HTTP behavior or performance on this platform.

Terminal receipt and complete logs are copied to `linux-arm64/` here. Native Node
SHA256: `0ded4eca7588873c109c34e96abc8475ee6b0ecca5e1e528064d43281ca8ab53`;
server SHA256: `e10107257731bb3e0c18c8ef4bf43e6fb606e0eb1ef4cc801c3a6e55356d5361`;
packed package SHA256: `c535dcc9d402ba0c3962ec40c97d174ebf0ad7e173c76c01ff81d0eb9fbfb92b`.

## Python HTTP SDK installed-wheel contracts

`uv build` produced the SDK source distribution and built its wheel from that
distribution. Two clean consumer projects installed the wheel with repository-
locked httpx 0.28.1, sniffio 1.3.1, orjson 3.13.0 and pytest 9.2.0, using existing
Python 3.11.14 and 3.14.7 runtimes. All 60 installed package files matched the wheel
bytes, and all 92 frozen source/test/manifest files were unchanged after testing.
Tests ran with isolated Python imports against the installed package. The evidence
test's direct file lookup was pointed to that same installed package by a symlink.
Both interpreters passed 54 tests and 46 subtests with zero failures or skips.

Artifact, consumer uv locks, installed-file receipts and frozen source are under
`/Volumes/A/.hms-bench-cache/warm-python-sdk-qualification-20261103-v1`.
Repository receipts are `python-sdk-{freeze,finished}.json` and `python-sdk-*.log`.
This covers the base HTTP SDK contract with local mocks/files. It does not qualify
native `warm-python`, live-server lifecycle, optional framework adapters or every
importer. Those remain separate platform-matrix obligations.

Native Python source is prepared separately at
`/Volumes/A/.hms-bench-cache/warm-python-native-qualification-20261103-v1`.
Its 442-file freeze uses the verified Linux source archives plus the repository's
Python metadata/tests and required Cargo include files; the tiny embedding fixture
matches its published repository digest. Maturin is pinned to 1.16.0 and Rust to
1.96.0. The queued host launcher (`mac-python-native.py`, frozen copy
`run.py` in the artifact root) waits for Linux calibration v2 to finish before
building a macOS arm64 abi3 wheel from an sdist. It will then install and test
Python 3.9, 3.12 and 3.14 consumers, including offline embedding fixtures. This is
queued work, not a passing wheel result; inspect `finished.json` and the retained
stage logs. No models or paid services are invoked. The first launcher, PID 16691,
was later absent without a terminal receipt or log output; its cause is unknown.
It was replaced with a detached direct-Python process, PID 16776, verified alive
while waiting. `launcher-first-attempt.json`, `launcher.pid` and `launcher.log`
in the artifact root preserve this distinction; the source and driver are unchanged.

## Queued installed-SDK live qualification

The existing pure-Python wheel is frozen into
`/Volumes/A/.hms-bench-cache/warm-python-sdk-live-20261103-v1`, with separate locked
Python 3.11.14 and 3.14.7 consumers. Thirteen source/artifact files are hashed;
the portable archive SHA256 is
`cd5a215af34fac89562a634bb3f372dcb85031f1e3b5d5e9a3e41f4d0e8e624f`.
The six existing live client cases and two retained-chunk cases are unchanged.
Four additional `test_memory_live.py` cases cover both temporal axes, explicit
contradictory evidence/provenance, atomic conflict rejection and retries, validity
boundaries, tenant/auth separation, and synchronous/asynchronous chained forgetting.
Collection found exactly twelve tests. Syntax and collection are preparation,
not passing live behavior.

[python_sdk_recovery.py](../../../benchmarks/public/python_sdk_recovery.py)
adds SIGKILL and graceful-restart cases after successful HTTP acknowledgements
and a client-fsynced receipt. Each checks 32 atomic facts, 32 document replacements,
deleted results, duplicate memory retries, temporal/source links and tenant
separation, followed by chain purge and another reopen. These are process-death
boundaries, not power-loss coverage. No remote soak data is used.

The [driver](linux-python-sdk-live.py) installs the frozen wheel into fresh Linux
arm64 consumers, compares every installed package file against the wheel, requires
all twelve live tests with zero skips, and runs both recovery cases. It uses the
existing frozen server binary, with no build or serving-source changes. Guest
launcher PID 27346 was verified waiting for `calibration-audit-v1-finished.json`;
the queue therefore adds no load to the active calibration. Its source is guest
`warm-milestones-20261103/sdk-live-v1`, with logs and terminal `finished.json` under
`evidence/python-sdk-live-v1`. The launcher log and PID are alongside that directory.
These tests are **queued, not passed**. The archive/source receipt is
`python-sdk-live-source.json`; no paid services or models are involved.


## iOS simulator qualification

The isolated iOS 27.0 arm64 consumer run uses the same frozen core as mobile host
v2. Its first clean build failed before simulator creation: `strip=debuginfo` also
produces a misaligned host procedural-macro dylib on macOS 27, matching
[Rust issue 157750](https://github.com/rust-lang/rust/issues/157750). The earlier
successful cached host build did not establish that this setting was reliable.
The Swift script now defaults to `strip=none`. Both the failed freeze/log and its
replacement are retained. The replacement builds the ios-sim XCFramework and
runs all seven existing XCTest cases from a separately extracted package on a
dedicated simulator, then shuts down that simulator.

Run: `/Volumes/A/.hms-bench-cache/warm-ios-simulator-qualification-20261103-v2`;
PID 44602; freeze `954ba9bc459319d7098aff8699577ea9f28b2397aa18edabbff7d2043daadf2e`.
Receipts: `ios-simulator-v{1,2}-{freeze,launched}.json` and v1 failure evidence.
V2 builds both native simulator slices successfully, then stops before tests
because preparation copied files without executable modes. V3 preserves and
verifies those modes; its 506 source contents are unchanged from v2. It builds the
XCFramework, extracts the consumer and starts the dedicated simulator tests.
V3 run: `warm-ios-simulator-qualification-20261103-v3`, PID 45866, freeze
`2dbebb0517bd1a5e8b41fde9d849caf75b250a497ad943359a8d52a80755aaa5`.
V3 reaches the simulator but XCTest blocks while opening the external-volume
test bundle. A sampled stack and TCC logs establish the pending access request;
the owned run is interrupted and shut down. V4 uses the identical compiled
package in a fresh internal-disk consumer, with build products also internal and
large intermediates retained externally. No privacy settings are changed.

V4 passes at 15:58:02Z: all seven tests execute on iOS 27.0 / arm64 with zero
failures, skips or expected failures. All 506 source hashes, seven installed
files, archive, logs and xcresult summary are verified, and the owned simulator
is shut down. This covers simulator lifecycle, filtering, deletion/reopen,
encryption, snapshot restore/tamper detection, errors, async concurrency and
chunking. It does not qualify a physical device, migration or process death.

Final run: `/Volumes/A/.hms-bench-cache/warm-ios-simulator-qualification-20261103-v4`;
freeze `4e01e83d9c01930d43ec0157174340f43580be7c7dc343eb6da2bcc008e1dea0`;
40,185,768-byte package SHA256
`cb2a922e7dab902394eca94833f43f06c5039c4d5b615bd94be6333551835ef6`.
Receipts: `ios-simulator-v4-{freeze,finished,test-summary,installed-files}.json`.

## UniFFI, Kotlin and Swift host qualification

The next frozen run uses the bounded-query product source plus the existing
UniFFI/Kotlin/Swift packages. Its initial full locked metadata check exposed a
stale UniFFI dependency edge. Offline resolution adds only `windows-sys` to the
root engine's dependency list; no package/version/checksum changes. The tracked
lock is corrected, with the original and failure log retained. Bindgen's lock
is unchanged.

Run root: `/Volumes/A/.hms-bench-cache/warm-mobile-host-qualification-20261103-v1`.
505-file freeze SHA256:
`216ef9174a01fbb84ce7ac8b7e59741662dfd568b8a8f76a92742b19848f2c29`.
Launcher PID 39259; runner `mac-mobile-host.py`; plan
`benchmarks/public/mobile_qualification/PLAN.json`.
It starts after the serving response audit terminates. The first gate passes
18 UniFFI tests (one unit and 17 integration), zero ignored/failed. Native library packaging and both Kotlin wrapper regeneration checks also passed;
the fresh JAR consumer passes all seven Kotlin tests with zero skipped/failed.
Swift then failed while loading a host `serde_derive` procedural macro: full
release stripping produced a misaligned LINKEDIT on macOS 27. The Swift build
script now defaults to `strip=debuginfo`, matching the existing JVM workaround.
The failed v1 receipt/log are retained.

V2 completed at 15:03:36Z. It reuses the hash-verified 18 UniFFI/seven Kotlin
results because only the Swift build script changed, then builds a new macOS
XCFramework and passes all seven tests in a fresh Swift package consumer.
Swift and both Kotlin generated interfaces match the committed sources after
trailing-whitespace normalization. All source, reused evidence, archive members,
installed Swift files and logs were verified. V2 freeze SHA256:
`2eb5ecf373ba8563333e6e1b9f9d33151a3325336e8e08a45d4cc6ffc9f25aee`.
Raw root: `/Volumes/A/.hms-bench-cache/warm-mobile-host-qualification-20261103-v2`;
runner `mac-mobile-host-v2.py`; terminal `mobile-host-v2-finished.json`.
JVM archive SHA256:
`35264da331b3bf6502af46e7649268207cd512fb1b3735e085abe394b23ba24c`.
Swift archive SHA256:
`cd5acab4f06040c3ab4f7e48156553d0e0fdbab23a2771d6b1b3214cf2bec3f0`.
The Swift archive includes arm64/x64 macOS slices; execution covers arm64 only.
The available JDK is Temurin 17.0.19+10, the project pins Gradle 8.14.6/Kotlin
2.2.21, and the host has Swift 6.4.2 and the existing Android SDK. No Android,
iOS, x64, Catalyst, visionOS, migration or process-death pass is inferred from
this host lifecycle run.
