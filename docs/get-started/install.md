---
description: Install WARM for Rust, Python or Node, with the version requirements and a one-line check that it works.
next_steps:
  - title: Quickstart
    text: Ingest, search, compare with SQLite and encrypt, in five minutes.
    url: quickstart/index.md
  - title: Concepts
    text: What "holographic" means in this code, and what is lossy.
    url: concepts/index.md
  - title: Server
    text: Run warm-server for the JavaScript or Python client.
    url: SERVER.md
---

# Install

WARM is a Rust engine; the other runtimes are surfaces over it. [Choose an integration](choose.md) first. Package installation needs network access; the engine can run offline after installation.

## Requirements

| You need | For |
|---|---|
| Rust 1.89 or newer (the MSRV) | the Rust crate, the HTTP server and embedded Node source builds |
| Rust 1.94 or newer | the `local-models` feature and `--all-features` |
| Node 18 or newer, or standard Fetch and Web Streams APIs | the portable JavaScript client |
| Node 18 or newer | the embedded Node bindings (verified on 18.19); Node 24 for the native quickstart, which uses `node:sqlite` |
| Node 20.19 or newer | building the portable JavaScript client from a checkout (`tsup`) |
| Python 3.11 or newer | the Python HTTP SDK (`warm-python` needs 3.9) |

For Python, create and activate a virtual environment first: `uv venv`, then
`source .venv/bin/activate` (PowerShell: `.venv\Scripts\Activate.ps1`).

## Install for your runtime

=== "Rust"

    ```sh
    cargo add warm-rust
    ```

    `default = []`: nothing is on unless you ask. Turn on what you need, for example
    `cargo add warm-rust --features security` for AES-256-GCM stores and signed audit logs.
    Every feature, what it turns on and whether it is experimental: [Cargo features](../reference/features.md).

=== "Python"

    `warm-sdk` is an HTTP client for `warm-server`, which holds the engine and stores the vectors you supply:

    ```sh
    uv venv && uv pip install warm-sdk   # run Python as .venv/bin/python
    ```

    Build and install the server (needs Rust 1.89 or newer), then start it:

    ```sh
    cargo install --locked --features server,security --bin warm-server warm-rust
    WARM_API_KEY=change-me warm-server --input-dim 384
    ```

    From a checkout, `cargo build --release --features server,security --bin warm-server` builds the same
    binary. Prebuilt, checksum-verified binaries (`scripts/install-warm-server.sh`, Linux glibc and
    macOS, no Alpine) start with the first `server-v<version>` GitHub release; none is published yet.

    The `warm-python` wheel runs native code in-process, imported as `writerslogic_warm`. From
    0.7.0 it includes the in-process `Warm` class (document search, `remember`, `recall`,
    `forget`, `explain`); the earlier 0.6.8 wheel provides the vector indexes and phase vectors only.

    ```sh
    uv pip install warm-python
    ```

=== "JavaScript client"

    `warm-sdk` 0.2.2 has no runtime dependencies or native binaries. For a published release:

    ```sh
    npm install warm-sdk@0.2.2
    ```

    Check the [available npm versions](https://www.npmjs.com/package/warm-sdk?activeTab=versions).
    To build from the checkout, then install the built directory in your application:

    ```sh
    npm ci --prefix bindings/js/warm-sdk
    npm run build --prefix bindings/js/warm-sdk
    # From your application directory:
    npm install /absolute/path/to/warm/bindings/js/warm-sdk
    ```

    Run `warm-server` separately using the Python tab's server commands; the same service
    supports both clients. Installing the client does not start the server or download models.
    Browser deployments must permit the application's origin at the HTTP service boundary.
    See [package availability](../RELEASING.md#javascript-package-availability).

=== "Embedded Node"

    `writerslogic-warm` embeds the engine in Node and selects an optional native binary for
    your OS, CPU and libc. For a published engine release, keep optional dependencies enabled:

    ```sh
    npm install writerslogic-warm@0.7.1
    ```

    Check the [available npm versions](https://www.npmjs.com/package/writerslogic-warm?activeTab=versions).
    To build the embedded bindings from a checkout with Rust installed:

    ```sh
    npm ci --omit=optional
    npm run build
    ```


## Check that it works

=== "Rust"

    ```sh
    cargo run --release --manifest-path examples/quickstart/rust/Cargo.toml
    ```

=== "Python"

    ```sh
    uv run --with warm-sdk python -c "from writerslogic_warm_sdk import WarmClient; print(WarmClient.__name__)"
    ```

=== "JavaScript client"

    ```sh
    node -e "const { WarmClient } = require('warm-sdk'); console.log(typeof WarmClient)"
    ```

=== "Embedded Node"

    ```sh
    node -e "const { WarmMemorySystem } = require('writerslogic-warm'); console.log(typeof WarmMemorySystem)"
    ```

These checks verify imports without contacting the server. The [quickstart](../quickstart/index.md)
runs a full ingest and search, with server round trips for the client runtimes.

## Connect an editor

Run the separate MCP launcher from npm:

```sh
npx -y writerslogic-warm-mcp
```

To build `warm-mcp` from this checkout instead and configure your editor to launch its absolute path:

```sh
cargo build --release --features mcp,security --bin warm-mcp
```

Host setup: [editor guide](../integrations/editors/README.md). Tools, storage and shared HTTP access: [MCP reference](../integrations/mcp/README.md).

Setup failures (imports, native binaries, authentication, vector dimensions, locked stores): [troubleshooting](troubleshooting.md).
