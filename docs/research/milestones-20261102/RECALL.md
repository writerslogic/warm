# Original ef=128/256 failures explained

All 4,000 ordered top-ten rankings match the saved original results: 1,000 queries
for each of two corpus sizes and two beam widths. The 2,000 original exact
rankings match the independent saved oracle. Every missing neighbor has a complete
directed reachability result, all incoming neighbors' traversal states, and a
shortest-path witness with the first blocked bridge's admission/eviction history.

| Corpus rows | ef | Mean / worst recall@10 | Missing neighbors | Unreachable misses | Witness bridge evicted before expansion | Witness bridge rejected |
|---|---:|---:|---:|---:|---:|---:|
| 10,000 | 128 | 0.9847 / 0.70 | 153 | 0 | 144 | 9 |
| 10,000 | 256 | 0.9976 / 0.90 | 24 | 0 | 21 | 3 |
| 100,000 | 128 | 0.9235 / 0.10 | 765 | 0 | 749 | 16 |
| 100,000 | 256 | 0.9634 / 0.50 | 366 | 0 | 354 | 12 |

All 1,308 misses occur before evaluation, with no later retention, residual-rerank,
shortlist or final-rescore losses. Directed BFS reaches every 10K vertex and
99,998 of the 100K vertices from each observed entry. The two unreachable 100K
vertices do not account for any missing oracle neighbor. These failures are
bounded-traversal losses. This does not imply that topology cannot be improved:
different edges may provide easier routes within the same beam. A witness path
is one sufficient explanation of blocked reachability, not the only possible path.

The experimental diagnostic replays the serving `Pool` and score kernels and
checks the entire visited set, ordered final beam, expansion count and results
against actual serving search. A separate JavaScript analyzer replays the exported
events, verifies every admission threshold, eviction, best-first expansion,
adjacency order and exhausted frontier, then computes directed reachability.
No serving search code or default was changed for this investigation.

Evidence in this directory:

- [100K receipt](navigation-100k/finished.json) and [all 100K per-query witnesses](navigation-100k/misses.ndjson.gz).
- [10K receipt](navigation-10k/finished.json) and [all 10K per-query witnesses](navigation-10k/misses.ndjson.gz).
- `navigation-tests.log`, `navigation-golden-recheck.log`, `navigation-analyzer-tests.log`, and `navigation-driver-tests.log` retain validation, including the resolved fixture failure.

Complete raw events, decoded adjacency, immutable source copies, patches,
lockfiles, binary, input/reference hashes, resource observations and terminal
receipts are retained at:

- `/Volumes/A/.hms-bench-cache/warm-navigation-20261102-v1`
- `/Volumes/A/.hms-bench-cache/warm-navigation-10k-20261102-v1`

Each contains `navigation.ndjson`, `navigation.ndjson.gz`, `misses.ndjson`,
`started.json`, `finished.json`, and `sources/`. The raw hashes are bound by the
receipts above. The full protocol and commands are in
[`NAVIGATION-PLAN.md`](../../../benchmarks/research/vector_topology/NAVIGATION-PLAN.md).
Pass `10000` as the optional final argument to `navigation-run.mjs` for the 10K
run; otherwise it runs 100K. The 10K case uses the first 10,000 rows of the same
hash-verified corpus and its separately saved oracle/reference results.

These are exposed-data diagnostic results, not held-out qualification or a
competitive performance claim. Original failures and the interrupted study remain
preserved. Milestones 2–8 require their own full evidence.
