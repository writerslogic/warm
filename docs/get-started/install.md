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

WARM is a Rust engine. The primary JavaScript package, `warm-sdk`, is a portable HTTP client,
as is the Python SDK. Embed the engine with the separate `writerslogic-warm` Node package or
Rust crate, or connect an editor through MCP. [Choose an integration](choose.md) before installing.
Package installation needs network access; the engine can run offline after installation.

## Requirements

| You need | For |
|---|---|
| Rust 1.89 or newer (the MSRV) | the Rust crate, the HTTP server and embedded Node source builds |
| Node 18 or newer, or standard Fetch and Web Streams APIs | the portable JavaScript client |
| Node 22 or newer | the embedded Node bindings; Node 24 for the native quickstart, which uses `node:sqlite` |
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

    The Python SDK is an HTTP client for `warm-server`, which holds the engine and stores the
    vectors you supply:

    ```sh
    uv pip install warm-sdk
    ```

    Build and start the server from a checkout, then point the client at it:

    ```sh
    git clone https://github.com/writerslogic/warm
    cd warm
    cargo build --release --features server --bin warm-server
    WARM_API_KEY=change-me target/release/warm-server --input-dim 384
    ```

    The `warm-python` wheel provides native vector search and phase vectors,
    imported as `writerslogic_warm`. It does not include document search.

    ```sh
    uv pip install warm-python
    ```

=== "JavaScript client"

    `warm-sdk` 0.2.1 has no runtime dependencies or native binaries. For a published release:

    ```sh
    npm install warm-sdk@0.2.1
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
    supports both clients. Installing the client does not start the server or download models. See
    [package availability](../RELEASING.md#javascript-package-availability).

=== "Embedded Node"

    `writerslogic-warm` embeds the engine in Node and selects an optional native binary for
    your OS, CPU and libc. For a published engine release, keep optional dependencies enabled:

    ```sh
    npm install writerslogic-warm@0.6.8
    ```

    Check the [available npm versions](https://www.npmjs.com/package/writerslogic-warm?activeTab=versions).
    To build the embedded bindings from a checkout with Rust installed:

    ```sh
    npm ci --omit=optional
    npm run build
    ```

    The portable client has no embedded subpath. `writerslogic-warm-mcp` is a separate MCP launcher.

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

The [quickstart](../quickstart/index.md) runs a full ingest and search against the same checkout.

The client checks verify imports without contacting the server. The
[JavaScript memory example](../memory-api/README.md#quick-start) and Python tab in the
[quickstart](../quickstart/index.md) perform server round trips.

## Connect an editor

Run the separate MCP launcher from npm:

```sh
npx -y writerslogic-warm-mcp
```

To build `warm-mcp` from this checkout instead and configure your editor to launch its absolute path:

```sh
cargo build --release --features mcp,security --bin warm-mcp
```

Follow the [editor setup guide](../integrations/editors/README.md) for your host. The
[MCP reference](../integrations/mcp/README.md) covers tools, storage and shared HTTP access.

## If setup fails

See [troubleshooting](troubleshooting.md) for import errors, missing native binaries,
server authentication, vector dimensions and locked stores.
