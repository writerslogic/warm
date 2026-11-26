# VIBE llama-128-ip and yi-128-ip with query lists

Host: GCP `warm-bench-icelake` (n2, 8 vCPU Ice Lake Xeon 2.6 GHz, 16 GB), Linux, `-C target-cpu=native`, one query at a time on one thread.
Data: VIBE `07b387891a221b7b073b83d2f752b76462e5fa03`, full corpora, official 1000 test queries, k=100.
Truth: recomputed per run by float64 inner product (descending score, base-id ties). Recall is strict neighbour-ID recall, the minimum over 3 repeats; QPS is the median of 3 repeats.
Build: adapter defaults (degree 32, build_ef 500, 8-bit codes, seeded learn sample, projected degree 32). `query_lists` 512 on 32768 learn rows (`512`), 2048 on 131072 (`2048`). Test queries and truth never build or tune.

- `ab/`: graph-only rows, `origin/main` wheel (`base-*`) against this branch (`new-*`), 3 alternating blocks (`ab2/` repeats the yi and llama rows after marking the cold paths `#[inline(never)]`); `ab/sift-*.log` is SIFT-1M (euclidean, k=10, degree 32, 8-bit) on both wheels, 2 alternating blocks.
- `f-<set>-<lists>.json`: list rows (`probes` > 0; `probe_only` true scans the probed lists, false seeds the graph search with them).
- `base-*.json`: first graph-only run of the `origin/main` wheel before any change.
- Harness: `benchmarks/vibe/ood_lists.py`; commands in the PR description.

Fastest row meeting each recall (same host, same protocol):

| set | recall | graph-only `origin/main` | with lists | rows |
|---|---|---|---|---|
| llama | 0.90 | 6958 | 17393 | P1 ef200 rr150 probe-only, 512 lists |
| llama | 0.95 | 5871 | 11693 | P2 ef200 rr150 probe-only, 512 lists |
| llama | 0.99 | 2360 | 4824 | P8 ef300 rr300 probe-only, 512 lists |
| yi | 0.90 | 6940 | 8718 | P2 ef300 rr150 probe-only, 512 lists |
| yi | 0.95 | 4064 | 6029 | P3 ef300 rr150 probe-only, 2048 lists |
| yi | 0.99 | 1225 | 3218 | P4 ef150 rr150 graph-seeded, 2048 lists |

Graph-only rows of this branch against `origin/main`: llama within +2.0% to +0.2% at every point, yi +0.5% to -0.8% up to ef 500, and -5% to -6% at ef 1000 rr 400 (1143 to 1187 against 1209 to 1252 QPS, consistent in all 6 blocks; the 0.99 row above no longer uses it). SIFT-1M within run-to-run noise (-2.1% to +2.2% per point across two blocks).
