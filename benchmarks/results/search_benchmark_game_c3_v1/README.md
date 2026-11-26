# search-benchmark-game: WARM 0.7.4 vs infino, tantivy and iresearch on c3-highcpu-8 (2026-11-26)

Machine: GCP c3-highcpu-8 (8 vCPU Intel Xeon Platinum 8481C, 16 GB), us-central1-a. Harness: search-benchmark-game fork dcondrey/infino-search-benchmark-game, branch add-warm-0.7.1-bp at 9308ea1 (upstream PR infino-ai/search-benchmark-game#83). Raw harness output: results-full.json (962-query set) and results-tpuf.json (31-query tpuf set). Corpus: wikipedia 5,032,104 docs. Metric: harness per-query best of 10 runs, then mean / median over queries of each tag (us). All engines ran in one `make bench-full` invocation, 2026-11-26 07:28-07:58 UTC; the tpuf comparison ran earlier the same boot.

Engines: warm-rust =0.7.4 (tag v0.7.4, 005c27d4), infino-0.11.26 (crates.io =0.11.26), infino-main @ 9efe399f, tantivy-0.27, iresearch-26.10.3. Lucene 10.5.0 and 10.5.0-bp FAILED (engine exited during TOP_10 on query "business consultants", both), so they are absent; the failed make exit left results in results.json (copied as results-full.json).

Commands: `make bench-full ENGINES="infino-0.11.26 infino-main tantivy-0.27 lucene-10.5.0 lucene-10.5.0-bp iresearch-26.10.3 warm-0.7.4-bp"` (962 queries; TOP_10 TOP_100 TOP_1000 TOP_100_COUNT COUNT); tpuf: `python3 src/client.py queries-tpuf.txt warm-0.7.4-bp infino-0.11.26 infino-main` (31 queries; commands TOP_10 TOP_100 TOP_1000 COUNT; no turbopuffer column merged). Tables below are mean / median us. Full tables: cells-full.txt, cells-tpuf.txt.

Columns: warm-0.7.4-bp | infino-0.11.26 | infino-main | tantivy-0.27 | iresearch.

## Full set (962), headline
| cmd | warm-0.7.4-bp | infino-0.11.26 | infino-main | tantivy | iresearch |
|---|---|---|---|---|---|
| TOP_10 ALL | 319 / 169 | 470 / 234 | 511 / 236 | 1324 / 444 | 542 / 229 |
| TOP_100 ALL | 553 / 284 | 703 / 346 | 717 / 354 | 1633 / 597 | 727 / 338 |
| TOP_1000 ALL | 994 / 590 | 1011 / 571 | 1015 / 581 | 2369 / 1013 | 1027 / 606 |
| TOP_100_COUNT ALL | 744 / 368 | 1298 / 578 | 1274 / 556 | 2661 / 816 | 1358 / 602 |
| COUNT ALL | 448 / 192 | 624 / 227 | 623 / 232 | 1332 / 333 | 592 / 212 |

## TOP_1000 by category (mean / median)
| category | n | warm | infino-0.11.26 | infino-main | result vs both infino |
|---|---|---|---|---|---|
| ALL | 962 | 994 / 590 | 1011 / 571 | 1015 / 581 | mean win 1.7% / 2.1%; median LOSS |
| union | 301 | 1376 / 912 | 1115 / 729 | 1106 / 734 | LOSS |
| intersection | 300 | 507 / 360 | 501 / 346 | 508 / 347 | tie (mean +1% vs 0.11.26) |
| phrase | 300 | 852 / 378 | 1308 / 568 | 1337 / 562 | WIN |
| intersection_union | 40 | 2614 / 2354 | 1613 / 1102 | 1503 / 1110 | LOSS |
| negated | 19 | 494 / 419 | 331 / 310 | 353 / 329 | LOSS |
| term | 1 | 1939 | 1299 | 1278 | LOSS |
| global (657) | 657 | 972 / 660 | 798 / 567 | 807 / 581 | LOSS |

## TOP_100 by category (mean)
union 522 vs 551/564 win; intersection 306 vs 343/360 win; phrase 698 vs 1124/1156 win; negated 150 vs 153/151 tie; intersection_union 1301 vs 1114/1003 LOSS; term 267 vs 284/274 win.
## TOP_10 by category (mean)
warm wins every category: union 261 vs 321/335, intersection 200 vs 223/239, phrase 435 vs 781/893, intersection_union 537 vs 751/681, negated 58 vs 81/75, term 52 vs 87/88.
## COUNT and TOP_100_COUNT (mean)
warm has the lowest mean of all five engines in every category of COUNT and TOP_100_COUNT (rows in cells-full.txt); no loss observed there.

## tpuf set (31 queries; 18 union, 11 intersection, 2 term), mean / median
| cmd | warm | infino-0.11.26 | infino-main |
|---|---|---|---|
| TOP_10 ALL | 2410 / 452 | 2260 / 445 | 2107 / 502 |
| TOP_100 ALL | 4223 / 701 | 2966 / 814 | 2744 / 842 |
| TOP_1000 ALL | 7495 / 1863 | 4633 / 2086 | 4124 / 2062 |
| COUNT ALL | 694 / 442 | 1037 / 526 | 1030 / 518 |
Losses: warm mean is slower at TOP_10/100/1000 ALL (driven by long unions: TOP_1000 union 11474 vs 6716/5975, TOP_100 union 6602 vs 4439/4111); warm median is faster at TOP_100, TOP_1000 and COUNT. warm wins COUNT in all categories and term at TOP_10.

Plain statement: at TOP_1000 the full-set ALL mean is a 1.7% win but every non-phrase category except intersection (tie) loses; the win comes from phrase queries. Claims apply only to these pinned versions, this VM and this run (single run per engine, no repeat of the whole sweep).
