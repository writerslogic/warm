# Paired same-session comparison and insert-cost sweep for the index-time-tables change (PR 554), c3-highcpu-8, 2026-11-28

Machine: GCP c3-highcpu-8, us-central1-a. Same session, search-benchmark-game 962-query set over 5,032,104 documents; the harness's per-query best of 10 runs; both engine orders (`final-A.json`: main, new, new over the old index; `final-B.json`: the reverse). Scripts: `final1.sh` (unpack, build, index), `final2.sh` (sweep, then A and B), `final3.sh` and `final4.sh` (sweep reruns), `fair_final.py` (tables, count diff); output of the analysis: `ab-summary.txt`; index and build log: `index-and-build.txt`.

Engines: `warm-main-final` is the main branch at ce09ae91; `warm-new-final` is the PR 554 line at 81315b0a (the later commits change only the write-side seal); `warm-new-oldidx` is the new build opened over the index the main build wrote (the older checkpoint format, where tables are computed at open). The same adapter source drives all three.

## Query cost (median of per-query medians, microseconds; A order then B order)

| command | main | new | new over old index | main / new (A, B) |
|---|---|---|---|---|
| TOP_10 | 172.0, 172.0 | 195.0, 198.5 | 207.8, 195.8 | 0.88, 0.87 |
| TOP_100 | 285.2, 285.5 | 341.2, 337.5 | 338.5, 340.5 | 0.84, 0.85 |
| TOP_1000 | 591.5, 594.5 | 694.0, 693.0 | 689.5, 693.2 | 0.85, 0.86 |
| TOP_100_COUNT | 373.0, 371.0 | 461.0, 462.0 | 461.5, 461.5 | 0.81, 0.80 |
| COUNT | 193.0, 194.5 | 246.2, 247.2 | 250.0, 247.2 | 0.78, 0.79 |

The new build is 12 to 28 percent slower than main on this harness, in both orders. The old-index path costs no more than the stored-table path. COUNT and TOP_100_COUNT results are identical on all 962 queries between the three engines, in both orders. Main keeps per-thread hot lists and a norm memo between queries, which a harness that repeats queries rewards; the new build carries none. This is a measurement of the difference, not an attribution test of the cause.

Index: new 3,800,683,426 bytes, main 3,474,585,007 bytes (+9.4 percent).

## Embedded single-insert cost against store size (microseconds, median; three alternating runs per tree)

`sweep-81315b0a.txt`: the new build grew with the store, +4.5 us at 100k chunks to +13 us at 1M over main (attributed to a per-seal fold over every block of a long list; not profiled). `sweep-aa6463d8.txt`: carrying the head maxima halved the growth (+2.5, +4, +6 us). `sweep-7712d1b0.txt`: with the run list edited in place the gap is a constant:

| chunks | main | new (7712d1b0) |
|---|---|---|
| 100k | 29.4, 28.4, 29.3 | 32.4, 32.5, 31.6 |
| 300k | 28.4, 26.6, 28.2 | 30.8, 30.9, 31.7 |
| 1M | 28.8, 28.7, 29.3 | 32.7, 32.1, 31.4 |

p99 stays within 2 to 7 us of main at each size.
