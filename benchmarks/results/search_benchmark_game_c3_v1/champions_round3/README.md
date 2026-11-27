# Round 3: compact heap, champion postings, champion floor

Same-session A/B on GCP c3-highcpu-8 (`warm-sbg-c3`), 5,032,104-document Wikipedia index, `qb7x` in-process harness: mean over queries of the best of N passes, in microseconds, base first then candidate first (`order A`, `order B`). `differing` counts queries whose ordered ids, chunk indexes and score bits differ between the two binaries.

Commits (stacked):
- A `0d05add3` compact top-k heap keys (from `f2c04b4e`).
- B `842ca817` champion postings: the top 4096 postings of every list of 32768 postings or more, in ranking order, built at open. A single-term query with k <= 4096 reads its hits from the list; a required term with exclusions (k >= 100) applies them to the champions and falls back when fewer than k survive.
- C `f9e08386` the champions' k-th best weight raises the union floor in `impact::rank`.

Files in `ab/`: `A-*` is main vs A, `B-*` is A vs B, `C-*` is B vs C; `full` is the 962-query set, `tpuf` the 31-query set (5 to 8 passes, 2 to 3 rounds per order).

Champion cost: no index format change and no index size change. They are derived at open, in parallel (8 threads): open of the 5M index goes from 0.65 s to 1.67 s. They are not persisted in the index.

Headline (TOP_1000, both orders agree, differing 0): term 1900 -> 105 (-94%), negated 436 -> 376 (-14%), union 1226 -> 1172 (-4.4%), union:num_tokens_2 733 -> 660 (-10%), intersection_union 2560 -> 2500 (-2.2%), union:num_tokens_>3 -4% with C.

Known gate failures (cells that move the wrong way in both orders): A at TOP_10, negated 38 -> 39 us (+4.0%, +5.4%); B at tpuf TOP_100/TOP_1000, intersection +1.6% to +2.5% (B does not touch intersection; C reverses it). Cells under 5 us at 5 passes are quantised; at 3000 passes the term cell at TOP_100 reads 1.8 us in both builds.

Tried and dropped: seeding the heap floor from the lone required term's champions in `required_optional` (intersection_union). Same-session A/B against the final tree, both orders, 0 differing: TOP_1000 intersection_union 2487 -> 2530 (+1.7%) and 2488 -> 2531 (+1.8%), TOP_100 1250 -> 1269 (+1.5%) and 1253 -> 1268 (+1.2%), every other cell within 1%. It regresses the target cell in both orders, so it is not in any PR. Only 10 of the 40 intersection_union queries have two tokens, the shape where the lacking class is a pure champion read.

Not done: persisting champions in the index; intersection_union restructuring (lever 2); COUNT union (lever 4: `union_count` already counts by inclusion-exclusion and bitmap popcount).

## Final stack (one commit)

`ab-final/` is main vs the squashed tree (`perf/sbg-round3-final`), both orders, 3 rounds, all modes, both query sets, differing 0 in every cell. With the heap reading tie keys only on equal scores, TOP_10 negated no longer regresses (37 -> 36 us, both orders).

TOP_1000 (us, both orders agree): term 1896 -> 105, negated 435 -> 382, union 1223 -> 1150, union:num_tokens_>3 5003 -> 4793, intersection_union 2558 -> 2492, ALL 918 -> 886. TOP_100: negated 118 -> 102, term 248 -> 16, ALL -0.6%. tpuf TOP_1000: ALL -2.6%, union -1.4%. COUNT and TOP_10: within 1.5%, no cell moves the wrong way in both orders beyond 0.5%.

Open question from review: the champions are derived at open, which the benchmark owners count as index time; they are to move into the persisted checkpoint section being added on perf/index-time-tables.

Exactness of the final tree (`differential/`, `qb7d` served vs `LexicalExecution::Exhaustive`, ordered ids, chunk indexes and score bits): 0 differences in TOP_10, TOP_100, TOP_1000, COUNT and TOP_100_COUNT on the 600k index (`diff3-600k.log`, 962 and 31 queries) and on the 5,032,104-document index on warm-textscale-5 (`diff3-5m.log` 962 queries, `diff3-5m-tpuf.log` 31 queries).
