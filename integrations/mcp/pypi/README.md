<a href="https://writerslogic.com/warm/"><img src="https://raw.githubusercontent.com/writerslogic/warm/main/assets/logo/warm-logo.png" alt="WARM logo" width="96" align="left"></a>

# WARM MCP

<!-- mcp-name: io.github.writerslogic/warm-mcp -->

Persistent local memory for AI agents through the Model Context Protocol. The server stores memories on your machine and supports stdio and streamable HTTP transports.

[Website](https://writerslogic.com/warm/) · [Documentation](https://writerslogic.com/warm/docs/latest/integrations/mcp/) · [Source](https://github.com/writerslogic/warm) · [License](https://github.com/writerslogic/warm/blob/main/LICENSE)

<br clear="left">

## Install

None: the launcher runs on demand. Python 3.9+ is required.

```sh
uvx warm-mcp                      # stdio, for clients that start a local command
uvx warm-mcp --transport http     # shared server at http://127.0.0.1:8765/mcp
```

## Quickstart

Add the server to a client's `mcpServers` (Claude Desktop, Cursor, Windsurf and others):

```json
{
  "mcpServers": {
    "writerslogic-warm": {
      "command": "uvx",
      "args": ["warm-mcp"]
    }
  }
}
```

## Options

| Name | Type | Default | Meaning |
|---|---|---|---|
| `--transport` | `stdio` or `http` | `stdio` | Transport; http serves `http://127.0.0.1:8765/mcp` |
| `WARM_MCP_BINARY` | path | unset | Run your own `warm-mcp` build, no download |
| `WARM_MCP_CACHE_DIR` | path | user cache dir | Where the verified binary is cached |
| `WARM_MCP_RELEASE_BASE` | URL | GitHub release | Download base; plain HTTP is refused except on loopback |
| `WARM_MCP_CHECKSUMS` | path | shipped table | SHA-256 table the download is checked against |

Everything else (`--data-dir`, `--store`, namespaces, encryption, authentication, the tools and resources the server exposes) is `warm-mcp` itself: see the reference.

## Compatibility

Python 3.9+; downloads the `warm-mcp` binary release tagged with the launcher version (`mcp-v<version>`). First run needs network access for the binary download; the server itself runs locally. The same launcher is [available on npm](https://www.npmjs.com/package/writerslogic-warm-mcp).

## Reference

[MCP guide](https://writerslogic.com/warm/docs/latest/integrations/mcp/) (also `docs/integrations/mcp/README.md` in the repository).
