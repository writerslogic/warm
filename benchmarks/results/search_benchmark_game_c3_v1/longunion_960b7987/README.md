# Long-union best-first (960b7987, PR #542)

Same-session search-benchmark-game run on GCP c3-highcpu-8 (Wikipedia, 5,032,104 docs), warm-0.7.4-bp against warm-lu3-bp (commit 960b7987) and infino-0.11.26. Values are mean / median microseconds of the per-query best of 10.

- `tables-full3.md`, `results-full3.json`: 962-query set.
- `tables-tpuf3.md`, `results-tpuf3.json`: 31-query tpuf set.
- `tables.py`: generates the tables from the JSON.

Exactness: `qb7d.rs` (../exhaustive_differential/) compares the served path with `LexicalExecution::Exhaustive` (ordered ids, chunk indexes, score bits) for TOP_10, TOP_100, TOP_1000, COUNT and TOP_100_COUNT.
- `diff-600k.log`, `diff-600k-tpuf.log`: 600k-document index, both query sets, 0 differences.
- `diff-5m.log`: the 5,032,104-document index on warm-textscale-5, 962-query set then 31-query set, 0 differences.
