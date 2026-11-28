# Exactness evidence for #554 (perf/index-time-tables)

Served path (`LexicalExecution::Auto`) against `LexicalExecution::Exhaustive` with qb7d (count, FNV-1a digest over
ordered ids, chunk indexes and score bits), 5 modes (TOP_10, TOP_100, TOP_1000, COUNT, TOP_100_COUNT), on GCP
n2-highmem-8. Builds: `18ca145f` (full gate), `81315b0a` (carryover gate) and `aa6463d8` (final, smoke). Indexes: (a) new format, built by the
build's own `build_index` from the benchmark corpus (persisted section present: checkpoint feature word 0x2); (b) old
format, built by origin/main 225c2f25 (`idxmain-600k`) or earlier (`idx9`, 5M), feature word 0x0, which exercises the
open-time fallback.

## Scope of 18ca145f..81315b0a (src, Cargo.toml, Cargo.lock)

19 files, +366 -31: merge of main (#558 metadata filters: `filter.rs`, `index.rs` metadata_keys, `query.rs`
matches_metadata, `search.rs` validate_filter, `analysis.rs` stem/analyze helpers, bindings, warm-server, warm-mcp,
`document_api.rs` embedding_model), three `chunks_exact` to `as_chunks` in `image/derived.rs`, and the artefact rename
`arena` to `arena-manifest` in `format/upgrade.rs`. No change to BM25 ranking, postings, derived tables or the
checkpoint layout.

## Results (differences per mode; 0 in every cell unless stated)

| build | index | format | 962 queries | 31 queries |
|---|---|---|---|---|
| 18ca145f | 600k | new (a) | 0,0,0,0,0 | 0,0,0,0,0 |
| 18ca145f | 600k | old (b) | 0,0,0,0,0 | 0,0,0,0,0 |
| 81315b0a | 600k | new (a) | 0,0,0,0,0 | 0,0,0,0,0 |
| 81315b0a | 600k | old (b) | 0,0,0,0,0 | 0,0,0,0,0 |
| 81315b0a | 5M | new (a) | 0,0,0,0,0 (3,356 s) | 0,0,0,0,0 (559 s) |
| 81315b0a | 5M | old (b) | not run | not run |
| aa6463d8 | 600k | new (a), rebuilt | not run (carried over) | 0,0,0,0,0 (smoke, 89 s) |

Fuzz at 18ca145f on the new-format 600k index: 20,000 generated plus 720 boundary and length cases, 0 mismatches
(2,469 s). Not rerun at 81315b0a: the 600k index built by 81315b0a equals the 18ca145f one except 8 bytes (header
words 1 and 2 of `documents.ckpt`, the generation identity, 528,605,198 bytes in all), and every qb7d digest agrees.

Old indexes are not rewritten by an open: after every run, each file of `idxmain-600k` is byte-identical to before
except `.hms.lock`. A main build refuses a new-format index (`checkpoint feature bits 0x2 are newer than supported`).

## Cross-version write check (copy of the old 600k index, 18ca145f and 81315b0a each)

Open read-write, memorize 3 documents, delete 1, compact, flush (exit 0). `format.json` byte-identical (SHA-256
`8e29b545...8139`); checkpoint feature word 0x0 afterwards (no bit 0x2); origin/main 225c2f25 opens the copy and
answers COUNT and TOP_10 for 7 queries identically, served and exhaustive, to the 18ca145f and 81315b0a builds.

## Not run, and why

- 5M qb7d on the old-format index (`idx9`) and the 5M fuzz: scope cut. The old-format fallback passed qb7d at 600k on
  18ca145f and 81315b0a, the 600k fuzz passed (20,720 cases, 0 mismatches), and the 5M new-format qb7d covers the
  persisted section at scale. `idx9` files hash identically before and after (it was not opened by these builds).
- 18ca145f at 5M: superseded by 81315b0a on the same new-format 5M index (built by 18ca145f).

## aa6463d8 (postings/list.rs, write-side seal fix)

`git diff --stat 81315b0a..aa6463d8 -- src/ Cargo.toml Cargo.lock` is `postings/list.rs` and `postings/tests.rs` only.
The 600k index built by aa6463d8 matches the 81315b0a and 18ca145f builds in `documents.ckpt` size (528,605,198
bytes) and in every byte except 8 (header words 1 and 2); `seg_0.bin`, `format.json(.crc32)` and `store.json` are
identical. The 31-query qb7d on it (5 modes) has 0 differences, so the 600k fuzz and the 5M qb7d carry over.
