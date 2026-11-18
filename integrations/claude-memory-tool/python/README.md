# warm-claude-tool (Python)

Backs Anthropic's client-side memory tool (`memory_20250818`) with WARM: Claude's `/memories` directory is stored in a `warm-mcp` server and every file is searchable by meaning.

## Install

```sh
pip install warm-claude-tool   # plus a warm-mcp binary: npx -y writerslogic-warm-mcp, or uvx warm-mcp
```

## Quickstart

<!-- quickstart-check: skip: requires API key (ANTHROPIC_API_KEY) -->
```python
import anthropic
from warm_memory_tool import WarmMemoryToolSync

client = anthropic.Anthropic()
with WarmMemoryToolSync.connect(command="warm-mcp") as memory:   # or url="http://127.0.0.1:8765/mcp"
    runner = client.beta.messages.tool_runner(
        model="claude-sonnet-5-5",
        max_tokens=1024,
        messages=[{"role": "user", "content": "Remember that Acme Corp prefers email follow-ups."}],
        tools=[memory, memory.recall_tool()],
    )
    print(runner.until_done().content)
```

Needs `ANTHROPIC_API_KEY`, so it is not run by the docs check; the tool itself is tested offline against a real `warm-mcp` in `tests/`.

## Options

`WarmMemoryTool` (async, `BetaAsyncAbstractMemoryTool`) and `WarmMemoryToolSync` (`BetaAbstractMemoryTool`) take the same arguments to `connect(...)`.

| Name | Type | Default | Meaning |
|---|---|---|---|
| `command`, `args`, `env` | str, list, dict | none | Start `warm-mcp` over stdio |
| `url`, `token` | str, str | none, `WARM_MCP_TOKEN` | Connect to a shared `warm-mcp --transport http` |
| `namespace` | str | none | One store per user (server needs `--namespace-arg`) |
| `max_view_chars` | int | 16000 | Longest `view` result before paging with `view_range` |
| `edit_retries` | int | 25 | Compare-and-set retries when another writer edits the same file |
| `request_timeout` | float | 60.0 | Seconds per MCP request |

## Compatibility

Python 3.10+, `anthropic` >= 1.16, `mcp` 2.3 to <3, a `warm-mcp` server. Tested on Python 3.12 (31 tests against a real `warm-mcp`).

## Reference

Commands, safety rules and semantic recall: [Anthropic memory tool on WARM](https://github.com/writerslogic/warm/blob/main/docs/integrations/mcp/memory-tool.md). TypeScript sibling: [`../typescript`](../typescript/README.md).
