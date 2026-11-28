# search-benchmark-game: published WARM 0.7.5 against infino, Lucene, tantivy and iresearch on c3-highcpu-8 (2026-11-28)

Machine: GCP c3-highcpu-8 (8 vCPU Intel Xeon Platinum 8481C, 16 GB), us-central1-a. Harness: search-benchmark-game fork dcondrey/infino-search-benchmark-game, branch add-warm-0.7.1-bp at 79f7d4b (upstream PR infino-ai/search-benchmark-game#83; the same adapter is on the branch behind quickwit-oss/search-benchmark-game#99). Corpus: wikipedia, 5,032,104 documents (the adapter skips and counts the 391,611 with no indexable words). Metric: the harness's per-query best of 10 runs, then mean / median over the queries of each tag, in microseconds. Raw: `results-full.json` (962 queries) and `results-tpuf.json` (31 queries); tables: `cells-full.txt`, `cells-tpuf.txt`; derivation: `analyze.py`; run script: `pin1.sh`; run log: `pin.log`.

WARM is `warm-rust = "=0.7.5"` from crates.io (tag v0.7.5), with `Cargo.lock` committed in the adapter; nothing measured here comes from an unpublished build. The other engines are infino-0.11.26, infino-main at 9efe399f, tantivy-0.27, lucene-10.5.0, lucene-10.5.0-bp and iresearch-26.10.3.

Run: one `make bench-full` invocation over all seven engines (962 queries; TOP_10, TOP_100, TOP_1000, TOP_100_COUNT, COUNT), started 10:03 UTC, finished 10:47 UTC, exit 0; then the 31-query tpuf set over WARM and both infino builds, with `COMMANDS="TOP_10 TOP_100 TOP_1000 COUNT"`. The harness starts each engine in its own process and warms it before timing. This is a single run per engine; no repeat of the whole sweep. WARM's index was built fresh in this session (682.7 s, 3,800,683,055 bytes); the other engines' indexes were built earlier on the same VM and reused, so the index times for them in `results-full.json` are from those earlier builds. An earlier attempt at the tpuf set failed because the harness needs `COMMANDS` set; it was rerun with it and the result is the one here.

## Full set (962 queries), mean / median us

| cmd | warm-0.7.5 | infino-0.11.26 | infino-main | tantivy | lucene-10.5.0 | lucene-10.5.0-bp | iresearch |
|---|---|---|---|---|---|---|---|
| TOP_10 | 383 / 198 | 476 / 239 | 522 / 246 | 1353 / 473 | 1119 / 470 | 801 / 354 | 565 / 245 |
| TOP_100 | 682 / 344 | 722 / 361 | 737 / 371 | 1670 / 642 | 1558 / 670 | 1214 / 527 | 761 / 360 |
| TOP_1000 | 1168 / 701 | 1033 / 587 | 1038 / 595 | 2432 / 1122 | 2246 / 1231 | 1707 / 890 | 1070 / 649 |
| TOP_100_COUNT | 934 / 466 | 1323 / 598 | 1302 / 582 | 2728 / 876 | 4663 / 1142 | 3888 / 776 | 1410 / 634 |
| COUNT | 544 / 242 | 639 / 234 | 636 / 236 | 1345 / 340 | 1353 / 458 | 1115 / 327 | 604 / 214 |

Per-category tables for every command, and every cell where WARM is slower than an engine (on the mean, the median or both), are in `cells-full.txt`.

## Where WARM loses, by engine (full set)

- infino-0.11.26 and infino-main: TOP_1000 ALL (1168 / 701 against 1033 / 587 and 1038 / 595) and every TOP_1000 category except phrase (union, intersection, intersection_union, negated, term, global); TOP_100 union, intersection, intersection_union, negated and global; TOP_10 intersection (256 / 180 against 229 / 170 and 248 / 184 by mean) and, for 0.11.26, union by mean; TOP_100_COUNT intersection, and negated by mean; COUNT union, intersection, negated and global, and COUNT ALL by median (242 against 234 and 236).
- iresearch-26.10.3: TOP_1000 ALL and every TOP_1000 category except phrase; TOP_100 union by mean, intersection by median, intersection_union and global by mean; TOP_100_COUNT negated, and intersection and intersection_union by median; COUNT union, intersection, negated and global, and COUNT ALL by median (242 against 214); TOP_10 intersection by median.
- lucene-10.5.0-bp: TOP_1000 intersection_union (2967 / 2398 against 2735 / 1872) and TOP_1000 intersection by median.
- lucene-10.5.0: TOP_1000 intersection_union by median.
- tantivy-0.27: none.

WARM has the lowest mean and median at TOP_10 ALL and the lowest mean and median at TOP_100 ALL and TOP_100_COUNT ALL; at COUNT ALL it has the lowest mean but not the lowest median. It is behind infino and iresearch at TOP_1000 in every category except phrase.

## tpuf set (31 queries: 18 union, 11 intersection, 2 term), mean / median us

| cmd | warm-0.7.5 | infino-0.11.26 | infino-main |
|---|---|---|---|
| TOP_10 ALL | 3147 / 788 | 2341 / 474 | 2155 / 502 |
| TOP_100 ALL | 4908 / 1248 | 3061 / 858 | 2858 / 905 |
| TOP_1000 ALL | 7810 / 3060 | 4743 / 2152 | 4232 / 2111 |
| COUNT ALL | 1062 / 555 | 1149 / 556 | 1128 / 546 |

WARM loses TOP_10, TOP_100 and TOP_1000 ALL to both infino builds on the mean and the median, in the union and intersection groups, and at TOP_1000 and TOP_100 term. At COUNT it has the lowest mean and a median equal to infino-0.11.26 and 9 us above infino-main.

## Against the 0.7.4 run (PR 534)

These WARM figures are slower than the ones recorded for 0.7.4 (TOP_10 ALL 319 / 169, TOP_1000 ALL 994 / 590, tpuf TOP_1000 ALL 7495 / 1863). 0.7.5 keeps no state from one query to the next, which a harness that repeats queries rewards: see `../fair-final-20261128/` for the paired same-session comparison of the 0.7.5 line against the main it was branched from (12 to 28 percent slower on the 962 set, in both orders) and `../../sbg_carried_state_probe_20261128/warm-0.7.5/` for the carried-state probe on 0.7.5.
