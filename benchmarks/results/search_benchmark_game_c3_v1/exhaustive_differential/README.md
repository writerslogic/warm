# Served BM25 top-k vs exhaustive scoring (search-benchmark-game queries)

`qb7d.rs` runs every query of the search-benchmark-game 962-query set twice in one process: once
through the served path (`LexicalExecution::Auto`: term classes, windowed impact ranking, deep-k
union classes) and once through `LexicalExecution::Exhaustive`, which scores every posting. For each
query and mode it compares the hit count and an FNV-1a digest over the ordered document ids, chunk
indexes and score bit patterns, so any difference in membership, order or score is counted.

Result (`differences.log`): 0 differences in each of TOP_10, TOP_100, TOP_1000, COUNT and
TOP_100_COUNT (962 queries each, 4,810 comparisons).

- Date: 2026-11-25. Machine: GCP n2-highmem-8 (64 GB; the exhaustive path hydrates about 31 GB, more
  than the c3-highcpu-8 used for timing holds). 7 threads.
- Index: the English Wikipedia corpus of search-benchmark-game (5,032,104 documents), built by the
  warm engine of that harness.
- Source tree: the ranking changes (term-class unions, integer tie keys, id validation at open) that
  merged as #531 and shipped in 0.7.4; the huge-page serving change in the same PR does not touch
  ranking.
- Build: `cargo run --release --bin qb7d -- <index> <queries.jsonl> 7` with this Cargo.toml next to a
  checkout of the tree.
