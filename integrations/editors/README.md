# Editor and agent configs for WARM

Ready-made configs that start `warm-mcp` (`npx -y writerslogic-warm-mcp`) in each host. Per-host details, remote form and verification status: [docs/integrations/editors/README.md](../../docs/integrations/editors/README.md).

| Editor | Config in this repo | Copy to | Launches | Install |
|---|---|---|---|---|
| Claude Code | `claude-code/plugin/` (marketplace in `claude-code/.claude-plugin/`) | installed as a plugin | `warm-mcp` plus a SessionStart memory hook and skill | `claude plugin marketplace add ./integrations/editors/claude-code && claude plugin install writerslogic-warm@writerslogic-warm` |
| Claude Desktop | `claude-desktop/claude_desktop_config.json` | `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS), `%APPDATA%\Claude\` (Windows); restart the app | `warm-mcp` over stdio | merge the `mcpServers` entry |
| Cline | `cline/cline_mcp_settings.json` | `cline_mcp_settings.json` (MCP Servers panel, Configure) | `warm-mcp`, read-only tools auto-approved | merge the `mcpServers` entry |
| Codex CLI | `codex/config.toml` | `~/.codex/config.toml` or `.codex/config.toml` | `warm-mcp` over stdio | `codex mcp add writerslogic-warm -- npx -y writerslogic-warm-mcp` |
| Continue | `continue/writerslogic-warm.yaml` | `.continue/mcpServers/` | `warm-mcp` over stdio (agent mode) | `mkdir -p .continue/mcpServers && cp integrations/editors/continue/writerslogic-warm.yaml .continue/mcpServers/` |
| Cursor | `cursor/mcp.json` | `.cursor/mcp.json` (project) or `~/.cursor/mcp.json` | `warm-mcp`, project namespace | `mkdir -p .cursor && cp integrations/editors/cursor/mcp.json .cursor/mcp.json` |
| Gemini CLI | `gemini-cli/` (extension and `WARM.md`) | linked as an extension | `warm-mcp` plus a context file | `gemini extensions link integrations/editors/gemini-cli` |
| VS Code (Copilot) | `vscode/mcp.json` | `.vscode/mcp.json` or the user-profile `mcp.json` | `warm-mcp`, project namespace | `code --add-mcp '{"name":"writerslogic-warm","type":"stdio","command":"npx","args":["-y","writerslogic-warm-mcp"]}'` |
| Windsurf | `windsurf/mcp_config.json` | `~/.codeium/windsurf/mcp_config.json` | `warm-mcp` over stdio | merge the `mcpServers` entry |
| Zed | `zed/settings.json` | `settings.json` (`zed: open settings file`) | `warm-mcp` over stdio | merge the `context_servers` entry |

Check every file: `uv run --with pyyaml python integrations/editors/validate.py` (it also fails if this table drifts from the directory).
