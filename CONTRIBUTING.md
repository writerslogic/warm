# Contributing to WARM

WARM is Apache-2.0. There is no CLA: you keep your copyright and certify your right to
contribute with a [sign-off on each commit](#sign-off-your-commits-dco). Participation is covered
by the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately per
[SECURITY.md](SECURITY.md), never in a public issue.

## Ways to help

- Pick an issue labelled [good first issue](https://github.com/writerslogic/warm/labels/good%20first%20issue).
  Each one states the files involved and how to check the result.
- Ask or answer questions in [Discussions](https://github.com/writerslogic/warm/discussions).
- Report a bug with the smallest input that reproduces it. Include the OS, Rust and Node
  versions, and the enabled Cargo features.
- Open an issue before a large change so the approach can be agreed first.

## Set up

The default Rust library supports Rust 1.89 or newer. `local-models` and
`--all-features` need Rust 1.94 or newer; CI uses the current stable toolchain.
The Node bindings require Node.js 18 or newer (24 for the quickstart, which uses
`node:sqlite`). The HTTP Python SDK requires Python 3.11 or newer; native Python
binding requirements are documented separately.

```sh
git clone https://github.com/writerslogic/warm
cd warm
cargo test                          # default features, no native dependencies
cargo test --all-features           # everything CI builds
npm ci --omit=optional && npm run build && npm test   # Node bindings
git diff --exit-code -- index.d.ts   # generated declarations must match Rust
bash scripts/check-generated.sh     # regenerate schemas/types in a temporary directory and compare
```

If your cargo `target-dir` is shared with other builds and you see
`Blocking waiting for file lock`, set a private one: `export CARGO_TARGET_DIR=$HOME/.cache/warm-target`.

`default = []`: nothing is on unless asked for. A change that compiles under only one feature
combination is not done. Check the combinations the [CI matrix](.github/workflows/ci.yml) builds.

## Before you open a pull request

Run what CI runs:

```sh
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --all-features
cargo +1.89 check --no-default-features        # MSRV
```

CI uses the newest stable clippy, which is stricter than older releases. Update with
`rustup update stable` if a lint appears only in CI.

Also:

- **Tests.** New logic, bug fixes and security-relevant changes come with an automated test that
  fails without the change. Test behaviour, not getters or your own implementation. If a test is
  not feasible, say why in the PR.
- **Defects.** Reproduce first, fix the cause, and verify through the same surface the user hit:
  a UI bug needs a UI check, an export bug needs a real round trip.
- **Numbers.** Every number in the README or the docs traces to a committed file under
  [`benchmarks/results/`](benchmarks/results). Never write a figure from memory. If your change
  moves a number, regenerate the results file and say so. Report where WARM loses as plainly as
  where it wins. Capacity and scaling claims state the tested range; they do not extrapolate.
  The comparison and `benchmarks/competitive/README-block.md` are generated; do not edit
  them by hand. Keep detailed measurements in the benchmark documentation so the main
  README stays focused on installation, integration and supported use cases.
- **Style.** Follow `cargo fmt` and fix every clippy warning. Add comments only for a constraint
  or a reason that is not obvious from the code. JavaScript uses two-space indentation.
- **Security features.** With the `security` feature in scope: zeroize key material after cipher
  initialisation, compare secrets in constant time, never branch on secret values, and validate
  public keys and algorithm parameters before use.
- **Docs.** Put new pages in the `docs/` folder for their area; they join the navigation without
  editing `mkdocs.yml`. Preview with
  `uv run --no-project --python 3.12 --with-requirements docs/requirements.txt mkdocs serve`
  (generated API pages also need `npm ci --prefix scripts/docs-api`). CI builds the versioned site
  and checks internal links and fragments; a weekly job checks external links. Each concept has
  one home page: link to it instead of restating it. More: [docs pipeline](docs/community/docs-pipeline.md).
- **New files** start with the licence header: `// Copyright 2024-2026 WritersLogic Contributors`
  and `// SPDX-License-Identifier: Apache-2.0` (`#` comments for Python, YAML, TOML and shell).

## Sign off your commits (DCO)

WARM uses the [Developer Certificate of Origin](https://developercertificate.org/) instead of a
CLA. By signing off you certify that you wrote the change or have the right to submit it under
the project licence. Add a trailer to every commit:

```
Signed-off-by: Your Name <you@example.com>
```

`git commit -s` adds it from your `user.name` and `user.email`. To add it to commits you have
already made: `git rebase --signoff origin/main`, then `git push --force-with-lease`.

A workflow checks every non-merge commit from a contributor without write access (maintainers and
bots are exempt from the check, not the certificate). Maintainers squash-merge, so commits need no
GPG or SSH signature.

## Commits and pull requests

- Commit subjects read `<type>: <imperative description>`, one line. Types: `feat`, `fix`,
  `refactor`, `test`, `docs`, `perf`, `security`, `chore`.
- Keep a pull request to one logical change, and fill in the template. Link the issue it closes.
- Rebase on `main` before requesting review. Resolve every review thread, and every bot finding
  (CodeQL, code quality), by fixing it or by replying with why it is a false positive.
- Do not commit generated binaries, `*.lock` scratch files, or evidence directories over a few
  megabytes; large benchmark evidence stays out of git and the results file references it.
  Keep dependency lockfiles used by the build. `index.d.ts` is generated by the canonical
  `npm run build` command and committed with its Rust source changes; CI rejects drift.
  Regenerate `schemas/*.json` and `bindings/types/*.ts` with
  `cargo test --locked --lib --features node-api,server export`; the CI lint job
  runs `scripts/check-generated.sh` to compare a fresh temporary export with the
  committed artifacts, including added or removed files.
  Historical benchmark inputs are listed in
  [the evidence archive guide](benchmarks/evidence/README.md), with their original revisions
  and checksums. Restore them locally instead of adding copied build trees or raw caches.
  Keep exploratory experiment logs, raw model outputs, local build products and research
  planning records outside the public repository. Promote reusable harnesses with their tests
  when they support maintained functionality. A published performance claim must include
  its complete protocol, pinned inputs, aggregate results, limits and a durable path to the
  evidence needed for independent reproduction; retain negative results in that evidence.

## Release process

Maintainers cut releases from signed tags; contributors do not tag, publish packages, or edit
version numbers. To check a published artefact, see
[Verify a release](docs/security.md#verify-a-release).

## Governance

WARM is maintained by WritersLogic, Inc. with one maintainer (`@dcondrey`, in
`.github/CODEOWNERS`), who merges to `main`, cuts releases, approves the publishing environments
and triages security reports. Anyone may open issues and pull requests under the DCO. A
contributor with several merged, non-trivial pull requests can ask in an issue for write access.

Changes land only through pull requests. Branch protection requires signed commits, resolved
conversations, a code-owner review and an up-to-date branch. Design questions are settled in the
issue before code is written, and the reason is recorded there. A change to a public API, a stored
format or a security property needs a test. Conduct is governed by the
[Code of Conduct](CODE_OF_CONDUCT.md); vulnerabilities by [SECURITY.md](SECURITY.md).

No second maintainer is designated yet. WritersLogic, Inc. owns the repository, and its
organisation owners can grant access if the maintainer becomes unavailable.
