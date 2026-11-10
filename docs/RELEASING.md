<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Releasing WARM

The engine and binding tags produce artefacts, provenance attestations and SBOMs. MCP launchers
have their own version and release tag. Pull requests do not publish. Manual runs default to
builds and checks; explicit publish inputs enable protected crate-only recovery, npm publication
on `main`, or publication from the Python release workflows. MCP manual runs do not publish.

| Tag | Workflow | Produces |
| --- | --- | --- |
| `vX.Y.Z` | `.github/workflows/release.yml` | the crate, 8 npm native packages and the root npm package, Python wheels and sdist (as release assets), SBOMs, SLSA provenance, a draft GitHub release; then publishes the crate and the npm packages |
| `py-models-vX.Y.Z` | `.github/workflows/warm-models-release.yml` | the pinned default offline `warm-models` pack, published to PyPI through Trusted Publishing |
| `py-vX.Y.Z` | `.github/workflows/warm-py-release.yml` | `warm-python` wheels and sdist, published to PyPI through Trusted Publishing |
| `sdk-vX.Y.Z` | `.github/workflows/warm-sdk-release.yml` | `warm-sdk` wheel and sdist, published to PyPI through Trusted Publishing |
| `mcp-vX.Y.Z` | `.github/workflows/warm-mcp-release.yml` | five `warm-mcp` binaries, checksum tables embedded in npm and PyPI launchers, a GitHub release, then the launcher packages and official MCP Registry entry |

Publish a new model pack before a native wheel that requires that new version through `[embed]`.
The existing `warm-models==0.1.0` pin does not require another model release for engine 0.6.7.
The Python distributions have separate release tags and Trusted Publisher workflows. For a full
engine release, push both `vX.Y.Z` and `py-vX.Y.Z` at the same commit; the native wheels build twice.

The npm native binding `warm-sdk` follows the engine version; the PyPI HTTP SDK also named
`warm-sdk` has an independent version in `bindings/python/warm-sdk/pyproject.toml`. The engine
version script does not bump that SDK or the MCP launchers. Framework adapters in `integrations/js`
and `integrations/python`, and the two Claude memory-tool packages, have no publication workflow
in this release pipeline. Their first publication is separate from the 0.6.7 maintenance release.

## Python HTTP SDK release

When the SDK changes, set a new version in `bindings/python/warm-sdk/pyproject.toml` and document
compatibility changes, including removed adapters. Run **Python SDK release** on the candidate
with `publish=false`. Its gate builds with `uv build`, tests against a real `warm-server`, and
checks a clean wheel install, import and `py.typed` marker. Require the framework contract jobs
in `ci.yml` as well. Publish a signed `sdk-vX.Y.Z` tag matching the SDK manifest; do not reuse
an existing version or substitute the engine version.

## MCP launcher release

`writerslogic-warm-mcp` on npm and `warm-mcp` on PyPI is versioned separately from the engine. Its launchers
download `warm-mcp` from a `mcp-vX.Y.Z` GitHub release and verify its hash against a table inside
the package. The release workflow builds macOS arm64 and x64, Linux arm64 and x64 (glibc 2.17
minimum), and Windows x64 binaries. It rejects missing or extra assets, tests both launchers with a
local asset server, packs and installs both distributions, creates the GitHub release, then publishes
to npm, PyPI and the official MCP Registry. The Registry entry uses both packages, and the package
metadata proves ownership of `io.github.writerslogic/warm-mcp`.

Before the first release, add a **pending Trusted Publisher** for the new project at
`https://pypi.org/manage/account/publishing/`: project `warm-mcp`, owner
`writerslogic`, repository `warm`, workflow `warm-mcp-release.yml`, environment `pypi`.
PyPI creates the project on the first successful upload. The repository's `NPM_TOKEN` secret must
have publish access to `writerslogic-warm-mcp`.

To check the pipeline without publishing, run **MCP launcher release** with `workflow_dispatch` or
open a pull request touching its files. To publish, tag the commit containing the tested workflow
with `mcp-vX.Y.Z` where X.Y.Z matches `integrations/mcp/npm/package.json`,
`integrations/mcp/pypi/pyproject.toml`, the Python launcher's `__version__`, and
`integrations/mcp/server.json` (including both package entries), and push the signed tag.
`python3 scripts/ci/mcp-release.py mcp-vX.Y.Z` checks those versions and registry metadata.
A changed `warm-mcp` binary needs a new launcher release even when launcher source is unchanged. The
workflow never replaces an existing release or registry version. If one publish job fails after the
GitHub release exists, correct its credentials or publisher configuration and rerun that job only.

After the official Registry entry is active, submit the repository through Glama's **Add MCP
Server** form. `glama.json` names the maintainer, as in `scrivener-mcp`. Verify third-party
directories separately; an official Registry entry does not confirm their listings. At the 0.1.2
release, [mcp.so](https://mcp.so/Submit?type=server) has a free submission path and an optional
$39 paid publication option. Use the free path; do not pay. PulseMCP is not accepting submissions.
The [modelcontextprotocol/servers repository](https://github.com/modelcontextprotocol/servers)
now contains reference servers and directs community server discovery to the official Registry.

After Glama approves the listing, a signed-in maintainer must [configure its Docker release](https://glama.ai/mcp/servers/writerslogic/warm/admin/dockerfile).
That form generates a Dockerfile from its fields; the repository-root `Dockerfile` is built and
tested separately by `mcp-container.yml`. Clear the **Pinned Commit SHA** (`5250df9` points to a
commit before the repository Dockerfile and current tool definitions); leave it empty so Glama can
sync newer commits. Pinning a SHA requires updating it for every subsequent release.

Set **Base Image** to `debian:trixie-slim` and **Node.js version** to `24`. Glama installs Node and
`mcp-proxy` in its generated image. Enter this **Build Step** after publishing the MCP launcher to
npm:

```sh
npm install -g --no-audit --no-fund writerslogic-warm-mcp@latest && WARM_MCP_CACHE_DIR=/tmp/warm-mcp-cache writerslogic-warm-mcp --version && install -m 755 /tmp/warm-mcp-cache/*/*/warm-mcp /usr/local/bin/warm-mcp && install -d -m 1777 /data
```

The npm launcher verifies its downloaded binary against the checksum embedded in that release;
the build copies the verified binary into the image so it needs no download at runtime. Set **CMD
Arguments** to `["mcp-proxy","--","/usr/local/bin/warm-mcp","--data-dir","/data"]`, **Environment
Variables JSON Schema** to `{"type":"object","properties":{}}`, and **Placeholder Parameters**
to `{}`. Persistent memory on a hosted deployment requires Glama's optional `/data` volume.

Click Deploy; after its build test succeeds, choose Make Release and publish the MCP launcher
version. The build step picks up npm's latest published launcher on each build. Glama requires a
new build test and release publication for each version. A release enables hosted deployment and
security scans; a GitHub tag alone does not.

Then check the [server score](https://glama.ai/mcp/servers/writerslogic/warm)
and confirm it can deploy and introspect all eight tools. Review every tool's six Tool Definition
Quality Score dimensions, the server coherence score, and the listed security and maintenance
findings. Use Glama's specific findings to improve any dimension below 5/5, any overall score below
the highest tier, or any reported risk; then confirm the next scan reflects the fix. The
`mcp-container.yml` CI check covers the Docker build and stdio introspection before submission.

Afterward, confirm `npm view writerslogic-warm-mcp version` and the PyPI project page show X.Y.Z.
Run `uvx warm-mcp --version` and `npx -y writerslogic-warm-mcp --version` on a
supported platform with an empty launcher cache; these commands print the underlying engine's
version, which is independent of the launcher version.

## What is built

- **Python** (`python-wheels.yml`): abi3 (`cp39-abi3`) wheels for manylinux2014 x86_64 and aarch64,
  musllinux_1_2 x86_64 and aarch64, macOS arm64 and x86_64, Windows x64, and the sdist. Each wheel is
  installed into an empty virtualenv with no source tree and no network, then a real associative
  search and a resonator factorization run through it (`scripts/ci/smoke_python.py`). Linux wheels
  are additionally installed in a Debian 11 (glibc 2.31, Python 3.9) or Alpine container.
- **npm** (`npm-packages.yml`): `warm-sdk-{darwin-arm64, darwin-x64, linux-arm64-gnu,
  linux-arm64-musl, linux-x64-gnu, linux-x64-musl, win32-arm64-msvc, win32-x64-msvc}` plus the
  JavaScript-only root `warm-sdk`, whose `optionalDependencies` select the binding by
  os, cpu and libc. glibc bindings are linked against glibc 2.17, musl bindings with zig. The
  tarballs are installed into an empty project on a machine of every target (Alpine containers for
  musl, Debian 11 for glibc) with the root's optional dependencies pointed at the local tarballs,
  so npm itself must pick exactly one package, and a document search must succeed
  (`scripts/ci/smoke-node.cjs`). The root `package.json` (`napi.targets`) is the single list of
  platforms and `scripts/ci/npm-platform-packages.cjs` refuses to pack if it disagrees with
  `optionalDependencies` or any binding is missing.
- **Crate**: `cargo package --locked`, which also builds the packaged tarball.
- **SBOM**: a CycloneDX 1.5 JSON per artefact (`cargo-cyclonedx`, `npm sbom`), named
  `<artefact>.cdx.json` and paired by `scripts/ci/pair-sboms.py`, which refuses a release where any
  artefact lacks one.
- **Provenance**: `actions/attest-build-provenance` signs a SLSA provenance statement for every
  artefact and SBOM, and `actions/attest-sbom` binds each SBOM to its artefact. The bundle is also a
  release asset (`provenance.intoto.jsonl`).

## One-time setup (repository settings, by the maintainer)

These cannot be committed; the publish jobs wait on them.

1. Environments (Settings, Environments), each with required reviewers: `release`, `crates-io`,
   `npm`, `pypi`.
2. crates.io: add this repository's `release.yml` and environment `crates-io` as a Trusted Publisher
   for `warm-rust`. The job uses `rust-lang/crates-io-auth-action` and holds no token.
3. npm: provide an automation token for the `warm-sdk` and
   eight `warm-sdk-*` platform names as `NPM_TOKEN` (a repository or `npm` environment secret).
   The first publish of the eight platform package names needs a token. Publishing uses
   `--provenance`. Legacy package retirement runs separately through `retire-legacy-npm.yml`; registry policy and interactive two-factor authentication can restrict permanent deletion.
4. PyPI: add pending Trusted Publishers under owner `writerslogic`, repository
   `warm`, environment `pypi`: `warm-models` uses
   `warm-models-release.yml`, `warm-python` uses `warm-py-release.yml`, and
   `warm-sdk` uses `warm-sdk-release.yml`. The MCP launcher uses
   `warm-mcp-release.yml` with the same owner, repository and environment. PyPI permits three pending
   projects at a time; publish one before registering the fourth.

## Cut a release

1. **Version.** `uv run python scripts/ci/bump-version.py X.Y.Z` sets Cargo.toml, Cargo.lock,
   package.json with its `optionalDependencies`, package-lock.json and `docs/server/openapi.json`;
   `scripts/ci/check-versions.py`
   runs in every artefact job and again against the tag, so a mismatch fails the build.
2. **Changelog.** `git cliff -c cliff.toml -o CHANGELOG.md`, committed as `chore: update changelog`.
   Tag the commit before any `[skip ci]` commit (those run no workflows).
3. **Dry run.** Merge to `main`, then run Actions, Release and Python native release on `main`.
   Keep all publish inputs false. Run Python SDK release and MCP launcher release when those
   distributions change; run Python models release only when a new pack is needed. The dry runs
   build and test without publishing. The `Reproducible build` workflow must also be green on that commit.
4. **Tag and push** (signed): `git tag -s vX.Y.Z -m vX.Y.Z && git push origin vX.Y.Z`, and the same
   for `py-vX.Y.Z`. Publish `py-models-vX.Y.Z` first if the native `[embed]` extra needs a new model
   pack. Publish matching independent `sdk-vX.Y.Z` and `mcp-vX.Y.Z` tags when those distributions
   change. Approve the `release`, `crates-io`, `npm` and
   `pypi` environments when asked.
5. **GitHub release.** `release.yml` leaves a *draft* release holding every artefact, SBOM,
   `SHA256SUMS` and the provenance bundle. Prepare the notes, but keep it a draft until the registry
   and artefact checks below pass, then publish it. The data releases
   (`evidence-v*`) use other tags and never trigger these workflows.
6. **npm listing.** Verify the new root package and all eight platform packages are available:

   ```sh
   version=X.Y.Z
   for package in warm-sdk \
     warm-sdk-darwin-arm64 warm-sdk-darwin-x64 \
     warm-sdk-linux-arm64-gnu warm-sdk-linux-arm64-musl \
     warm-sdk-linux-x64-gnu warm-sdk-linux-x64-musl \
     warm-sdk-win32-arm64-msvc warm-sdk-win32-x64-msvc; do
     test "$(npm view "${package}@${version}" version)" = "$version" || exit 1
   done
   npm view "warm-sdk@${version}" deprecated # must print nothing
   npm dist-tag ls warm-sdk                  # latest must be X.Y.Z
   ```

   Confirm the latest registry page shows no deprecation banner, the README logo renders, and
   `npm install warm-sdk` resolves to X.Y.Z. npm can acknowledge a new package before
   its version is available; a [`0.0.0-stage` placeholder](https://docs.npmjs.com/staged-publishing/)
   alone is not a published binding.
   Require the `verify-published` job to pass for this release: it waits for the root and every
   declared optional platform package with a fresh npm cache, installs the published root package
   in a clean consumer, verifies npm signatures, and runs a document search. Registry metadata or
   a successful tarball build alone does not establish that consumers can use the release.
7. **Verify** what was shipped (see below).

### First npm release under `warm-sdk`

The 0.6.6 release partially published the new npm names: seven platform packages succeeded,
but npm rejected `warm-sdk-win32-x64-msvc` for package-name spam detection and the job stopped
before publishing root `warm-sdk` ([failed job](https://github.com/writerslogic/warm/actions/runs/42557810040/job/128969748569)).
The source rename did not publish the missing packages or transfer ownership from
`writerslogic-warm`. Before release,
confirm publishing authority for all nine names and configure `NPM_TOKEN` for them.
Publish the platform tarballs first, then the root, through the existing protected
release workflow. Require the cross-platform build/install gates, provenance checks,
and registry consumer smoke test before directing consumers to the new name.

Consumers replace `writerslogic-warm` with `warm-sdk` in their package manifests and
imports, including `/semantic` and `/memory`. Keep existing store directories and
configuration. A future first publication of the framework adapters in `integrations/js` must
coordinate their `warm-sdk >=0.6.5` peer dependency with a complete native binding release; those
adapters are not published by the engine tag and are not a prerequisite for 0.6.7. Until the new
native packages are available, released consumers continue using the existing published package.

Retiring or deprecating `writerslogic-warm` requires a separate maintainer action after
the replacement is verified. This rename leaves the MCP launcher, embedding-model
packages, WebAssembly package, Rust crate, and Python distributions unchanged; npm and
PyPI can independently use the name `warm-sdk`.

## Verify an artefact

```sh
gh attestation verify warm_python-X.Y.Z-cp39-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl \
  --repo writerslogic/warm
gh attestation verify warm-sdk-linux-x64-gnu-X.Y.Z.tgz --repo writerslogic/warm
npm audit signatures            # in a project that depends on warm-sdk
sha256sum -c SHA256SUMS         # next to the downloaded release assets
```

## Reproducible builds

`.github/workflows/reproducible.yml` builds `warm-eval`, `warm-server`, the napi `cdylib`, the Python
wheel and the crate on two independent runners from different checkout paths and fails unless the
SHA-256 of every artefact is identical (`scripts/ci/repro-build.sh`, `scripts/ci/compare-hashes.py`).
Inputs that are pinned: the committed `Cargo.lock` (`--locked`), `SOURCE_DATE_EPOCH` (the commit
time), `maturin`, no incremental compilation, and `--remap-path-prefix` for the checkout, cargo home
and rustup home. The Rust toolchain is the runner's current `stable`, so both legs must run in the
same workflow run.

Result of the first runs (two runners, different checkout directories): `warm-eval`, `warm-server`,
the napi `cdylib` and the crate are bit-for-bit identical. The Python wheel is identical member for
member except `*.dist-info/sboms/*` (and `RECORD`, which lists its hash), the SBOM maturin embeds, whose `bom-ref`s contain the absolute
checkout directory; the workflow therefore allows the wheel hash to differ and fails if any other
member differs.

| Artefact | Reproducible | Reason |
| --- | --- | --- |
| `warm-eval`, `warm-server`, napi `cdylib`, `.crate` | yes | |
| Python wheel (manylinux x86_64, built natively) | all members except the embedded SBOM | absolute checkout path in the SBOM bom-refs |

Not covered: the per-platform npm bindings and the cross-compiled wheels, which depend on the zig
and napi cross toolchains downloaded at build time (their SHA-256 are in `SHA256SUMS` and the
attestations, not compared).

## If something goes wrong

- A publish job failed partway: inspect its logs and the workflow at that tag before retrying.
  Current npm release steps skip existing versions, `cargo publish` refuses an existing version,
  and PyPI uses `skip-existing`. Historical tag workflows may lack current recovery guards; a
  re-run uses the workflow file at the tag. Fixes on `main` apply from the next tag, not to the old run.
- npm answered `E403 Package name triggered spam detection` for a new package name: ask npm
  support to clear that exact name and verify the resolution before another publish. For 0.6.6,
  the rejected name was `warm-sdk-win32-x64-msvc`; seven platform packages had succeeded and the
  root was not published. This error does not establish a credential or ownership failure, and
  changing the version alone does not clear it. Preserve the signed 0.6.6 tags when releasing 0.6.7.
- For npm-only recovery from the reviewed candidate on `main`, run
  `gh workflow run npm-packages.yml --ref main -f publish_npm=true` after resolving the failure.
  This protected path builds and verifies the packages, compares the integrity of any existing
  versions before skipping them, and requires the published-package consumer gate. Confirm the
  candidate's version and commit first; the normal full release uses the signed engine tag.
- A tag's crate upload failed because the archive is too large: fix the `Cargo.toml` package
  contents on `main` and verify `cargo package --locked --no-default-features` reports less than
  9.5 MB compressed. The `crates-io` environment normally permits only `v*` tags; temporarily
  add `main` as an allowed deployment branch while retaining the `v*` policy and required reviewer.
  Run `Release` on `main` with `publish_crate_only=true` and approve the `crates-io` environment.
  Remove the temporary `main` policy after the job finishes. This protected run packages,
  attests, and publishes only the crate.
  Verify that the crate index checksum matches the `crate-recovery` artifact and the registry
  download. If the GitHub release is still a draft, replace its crate, crate SBOM, and matching
  `SHA256SUMS` entries before publishing. If it is already immutable, leave those assets intact;
  record the recovery commit, registry checksum, and attestation in the editable release notes so
  readers can distinguish the published crate from the original oversized release asset. Never
  move the signed release tag.
- Published a bad version: crates.io `cargo yank --version X.Y.Z`; npm `npm deprecate
  warm-sdk@X.Y.Z "reason"` (unpublish only within 72 hours); PyPI yank in the web UI (no
  API). Then release X.Y.(Z+1); never re-tag.
- A tag pushed by mistake: delete the draft release first, then the tag, before approving any
  environment.
