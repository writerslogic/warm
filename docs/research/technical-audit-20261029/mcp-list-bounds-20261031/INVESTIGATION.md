<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# MCP listing and export follow-up

Source inspected at `6174370d3ba03c31700efa92bb5d6f6141b96713`. This investigation
is separate from PR #370. The export-cap failure is reproduced below; the
listing failure candidate and any performance gain remain unmeasured.

## Source observations

- `src/bin/warm-mcp/store.rs`, `WarmBackend::list`, reads 256 text-bearing
  documents before applying the requested tag, prefix end, or result limit.
  Consequently it reads text for excluded records and lookahead records. The
  newly fallible metadata projection can enumerate candidates without text.
  A proposed replacement must recheck the returned record's metadata after
  fetching its text, preserve cursor ordering, skip concurrent deletions, and
  retain explicit read errors for selected records. Listings already do not
  promise a transaction spanning all scanned pages.
- `src/bin/warm-mcp/tools.rs`, export's rendering loop, checks its byte cap only
  when `taken` is nonempty. The first memory can therefore exceed the configured
  export cap. `config.rs` permits export caps down to 1024 bytes while the
  default memory-text cap is 32768 bytes (maximum 65536). JSON, JSONL and Markdown
  have different framing sizes; current accounting adds two bytes per record.
- Export's limit lookahead calls `backend.list(limit=1)`; MCP resources and
  prompt construction also call the same listing backend. Changes must preserve
  their continuation behavior and must not manufacture an empty complete page.

## Next evidence and acceptance

1. Read the existing MCP listing/export tests and request validation before editing.
2. Reproduce excluded/lookahead retained-text failure with small local fixtures;
   verify selected-record failure still propagates and repaired text succeeds.
   Do not mutate live mmap checkpoint bytes.
3. Turn the reproduced export-cap violation into regression tests. Define
   the cap over the actual export payload and cover cap minus one, exact cap,
   and cap plus one, including framing/escaping and an oversized first record.
4. Reuse metadata enumeration and checked document reads; retain full records
   only for results. Preserve normal absence and cursor progress under changes.
   Never silently truncate a record or loop forever on an oversized first item.
5. Validate tool/resource/prompt callers and existing pagination tests. Any
   allocation or latency improvement requires a recorded before/after measure;
   source reasoning alone is not a performance result.

These are candidates for the next workstream after PR #370's exact-head gate,
merge signature/tree verification and clean main fast-forward. No advisory,
deadline, assertion or service budget may be weakened to obtain passing tests.

## Reproduced export-cap failure

`export-cap-probe.py` runs a local stdio server with a 1024-byte export cap and
stores one 3000-byte text fixture. All three export calls return success and one
record. The probe independently counts the UTF-8 bytes in the returned text
content and verifies agreement with the server's reported byte count:

| Format | Configured cap | Measured payload bytes |
|---|---:|---:|
| JSON | 1024 | 3146 |
| JSONL | 1024 | 3144 |
| Markdown | 1024 | 3056 |

The binary SHA-256 and source revision are in `export-cap-baseline.json`.
The binary postdated every current Rust source and was produced by the recorded
serial metadata contract build (`metadata-reads-20261031/contracts.log.gz`).
The initial probe checked reported bytes; the final probe additionally checks
the actual returned text bytes. Both use only one small temporary local store,
no providers, no models and no paid services. The process exits before fixture
cleanup. This demonstrates a configured payload-limit violation, not a measured
RSS, throughput or latency result.

Reproduce with the source-matched MCP executable:

```sh
WARM_MCP_BIN=/path/to/warm-mcp uv run --no-project python docs/research/technical-audit-20261029/mcp-list-bounds-20261031/export-cap-probe.py
```

This baseline probe intentionally asserts the old oversized successful result;
it must be replaced or explicitly run against the baseline when testing a fix.

## Baseline regressions and candidate (2026-11-01 UTC)

The Rust baseline at main `740e93e2a91900d7c0e0aa4163f593c458d96987`, with
`baseline-regression.patch.gz` applied, passed 42 tests and failed three:

- A 1394-byte JSON, 1392-byte JSONL and 1291-byte Markdown single-record export
  succeeded with each cap set one byte below the actual payload.
- Two-record exports split unnecessarily at the exact payload boundary (all
  formats) and one byte above it (JSONL and Markdown).
- Listing failed on damaged retained text outside the requested result due to
  page lookahead, tag exclusion, and the end of the requested prefix range.
  Selected-record reads failed as expected, and restoring the text repaired
  every case. Only the separate text cache was modified; no live checkpoint
  mappings were edited.

The official MCP SDK also reproduced the oversized successful export on both
the initially available Node 26.8.1 and the workflow's Node 22 line (22.23.4).
Both failed attempts are retained; subsequent SDK validation uses Node 22.

The candidate enumerates fallible metadata first, reads text only for selected
records, rechecks a fetched record's tag, and skips concurrent deletions. It
uses matching metadata to establish the continuation cursor. Export reuses
that cursor and incrementally renders the payload with exact format framing
and checked byte arithmetic. An oversized first record returns `too_large`
with its id and required bytes; a later oversized record preserves continuation
without truncation or silently skipping it.

All 47 Rust MCP tests pass, including deterministic replacement/deletion,
256-record scan boundaries, cap minus one/exact/plus one for every format,
Unicode and escaping, continuation, empty payloads and damaged-text recovery.
All 20 official SDK tests pass on Node 22, including the export and excluded-text
regressions, resources, review prompts, HTTP and encryption. The SDK corruption
regression separately failed against the unchanged baseline binary before the
candidate build (`sdk-corruption-baseline.log.gz`). Strict all-feature/all-target Clippy, scoped formatting and syntax checks pass.
No allocation, RSS or latency improvement is claimed from this evidence.
