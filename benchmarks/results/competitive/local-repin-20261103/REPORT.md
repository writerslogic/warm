# Comparator re-pin: version effect and verdict changes

## beir:nfcorpus (local-macos-arm64, load_1m 5.82, 5 rounds)

Version effect, new / previous (time, RSS and size: lower is better; qps and nDCG: higher is better):

| Metric | sqlite-fts5 3.53.1 / 3.50.4 | tantivy 0.27.0 / 0.26.2 | lucene 10.6.0 / 9.12.3 |
|---|---|---|---|
| qps | 0.997 | 0.939 | 1.025 |
| p50_latency | 0.996 | 1.045 | 0.976 |
| p99_latency | 1.028 | 1.131 | 0.931 |
| build_seconds | 1.446 | 1.961 | 0.846 |
| build_peak_rss_mib | 1.003 | 1.006 | 0.997 |
| serving_rss_mib | 1.005 | 1.005 | 1.299 |
| cold_start_s | 0.958 | 1.199 | 0.992 |
| disk_mib | 1.000 | 1.000 | 1.008 |
| replace_ms_per_op | n/a | n/a | n/a |
| delete_ms_per_op | n/a | n/a | n/a |
| ndcg@10 | 1.000 | 1.001 | 1.000 |
| recall@100 | 1.000 | 1.000 | 1.000 |

WARM verdict against the previous pin and the new pin (advantage factor above 1 means WARM is better):

| Metric | Comparator | vs previous | vs new | Changed |
|---|---|---|---|---|
| qps | sqlite-fts5 | WIN 21.95x | WIN 22.01x |  |
| qps | tantivy | WIN 1.92x | WIN 2.00x |  |
| qps | lucene | WIN 8.07x | WIN 7.50x |  |
| p50_latency | sqlite-fts5 | WIN 6.66x | WIN 6.70x |  |
| p50_latency | tantivy | WIN 1.78x | WIN 1.88x |  |
| p50_latency | lucene | WIN 7.84x | WIN 7.53x |  |
| p99_latency | sqlite-fts5 | WIN 39.47x | WIN 40.03x |  |
| p99_latency | tantivy | WIN 1.98x | WIN 2.08x |  |
| p99_latency | lucene | WIN 10.51x | WIN 10.37x |  |
| build_seconds | sqlite-fts5 | WIN 5.01x | WIN 7.25x |  |
| build_seconds | tantivy | TIE 1.21x | WIN 2.37x | FLIP |
| build_seconds | lucene | WIN 4.40x | WIN 3.72x |  |
| build_peak_rss_mib | sqlite-fts5 | LOSS 0.77x | LOSS 0.77x |  |
| build_peak_rss_mib | tantivy | WIN 1.06x | WIN 1.07x |  |
| build_peak_rss_mib | lucene | WIN 2.18x | WIN 2.18x |  |
| serving_rss_mib | sqlite-fts5 | WIN 2.50x | WIN 2.51x |  |
| serving_rss_mib | tantivy | WIN 2.71x | WIN 2.72x |  |
| serving_rss_mib | lucene | WIN 14.47x | WIN 18.70x |  |
| cold_start_s | sqlite-fts5 | TIE 0.17x | TIE 0.17x |  |
| cold_start_s | tantivy | LOSS 0.22x | TIE 0.21x | FLIP |
| cold_start_s | lucene | TIE 1.34x | TIE 1.36x |  |
| disk_mib | sqlite-fts5 | WIN 4.17x | WIN 4.17x |  |
| disk_mib | tantivy | WIN 2.04x | WIN 2.04x |  |
| disk_mib | lucene | WIN 1.54x | WIN 1.55x |  |
| replace_ms_per_op | sqlite-fts5 | n/m n/m | n/m n/m |  |
| replace_ms_per_op | tantivy | n/m n/m | n/m n/m |  |
| replace_ms_per_op | lucene | n/m n/m | n/m n/m |  |
| delete_ms_per_op | sqlite-fts5 | n/m n/m | n/m n/m |  |
| delete_ms_per_op | tantivy | n/m n/m | n/m n/m |  |
| delete_ms_per_op | lucene | n/m n/m | n/m n/m |  |
| ndcg@10 | sqlite-fts5 | TIE 1.00x | TIE 1.00x |  |
| ndcg@10 | tantivy | TIE 1.01x | TIE 1.00x |  |
| ndcg@10 | lucene | TIE 1.01x | TIE 1.01x |  |
| recall@100 | sqlite-fts5 | TIE 1.00x | TIE 1.00x |  |
| recall@100 | tantivy | TIE 1.00x | TIE 1.00x |  |
| recall@100 | lucene | TIE 1.00x | TIE 1.00x |  |

## memory (local-macos-arm64, load_1m 4.62, 5 rounds)

Version effect, new / previous (time, RSS and size: lower is better; qps and nDCG: higher is better):

| Metric | sqlite-fts5 3.53.1 / 3.50.4 | tantivy 0.27.0 / 0.26.2 | lucene 10.6.0 / 9.12.3 |
|---|---|---|---|
| qps | 0.975 | 1.035 | 1.238 |
| p50_latency | 1.008 | 0.972 | 0.831 |
| p99_latency | 0.834 | 0.945 | 0.897 |
| build_seconds | 2.038 | 1.227 | 1.477 |
| build_peak_rss_mib | 0.984 | 1.005 | 0.853 |
| serving_rss_mib | 1.002 | 1.014 | 1.039 |
| cold_start_s | 1.660 | 1.107 | 1.039 |
| disk_mib | 1.000 | 1.000 | 1.001 |
| replace_ms_per_op | n/a | n/a | n/a |
| delete_ms_per_op | n/a | n/a | n/a |
| ndcg@10 | 1.000 | 1.009 | 1.000 |
| recall@100 | 1.000 | 1.000 | 1.000 |

WARM verdict against the previous pin and the new pin (advantage factor above 1 means WARM is better):

| Metric | Comparator | vs previous | vs new | Changed |
|---|---|---|---|---|
| qps | sqlite-fts5 | WIN 167.98x | WIN 150.22x |  |
| qps | tantivy | WIN 1.84x | WIN 1.68x |  |
| qps | lucene | WIN 7.81x | WIN 5.62x |  |
| p50_latency | sqlite-fts5 | WIN 141.86x | WIN 133.78x |  |
| p50_latency | tantivy | WIN 2.03x | WIN 1.91x |  |
| p50_latency | lucene | WIN 7.40x | WIN 5.66x |  |
| p99_latency | sqlite-fts5 | WIN 301.70x | WIN 254.75x |  |
| p99_latency | tantivy | TIE 1.41x | TIE 1.46x |  |
| p99_latency | lucene | WIN 7.72x | WIN 8.22x |  |
| build_seconds | sqlite-fts5 | WIN 1.14x | WIN 2.32x |  |
| build_seconds | tantivy | TIE 0.92x | WIN 1.13x | FLIP |
| build_seconds | lucene | WIN 2.07x | WIN 3.06x |  |
| build_peak_rss_mib | sqlite-fts5 | LOSS 0.93x | LOSS 0.92x |  |
| build_peak_rss_mib | tantivy | WIN 1.41x | WIN 1.42x |  |
| build_peak_rss_mib | lucene | WIN 1.78x | WIN 1.52x |  |
| serving_rss_mib | sqlite-fts5 | WIN 2.01x | WIN 2.01x |  |
| serving_rss_mib | tantivy | WIN 2.33x | WIN 2.36x |  |
| serving_rss_mib | lucene | WIN 9.61x | WIN 9.93x |  |
| cold_start_s | sqlite-fts5 | TIE 2.96x | TIE 3.24x |  |
| cold_start_s | tantivy | TIE 3.53x | TIE 2.79x |  |
| cold_start_s | lucene | TIE 1.77x | TIE 2.38x |  |
| disk_mib | sqlite-fts5 | WIN 5.44x | WIN 5.44x |  |
| disk_mib | tantivy | WIN 3.87x | WIN 3.87x |  |
| disk_mib | lucene | WIN 2.92x | WIN 2.92x |  |
| replace_ms_per_op | sqlite-fts5 | n/m n/m | n/m n/m |  |
| replace_ms_per_op | tantivy | n/m n/m | n/m n/m |  |
| replace_ms_per_op | lucene | n/m n/m | n/m n/m |  |
| delete_ms_per_op | sqlite-fts5 | n/m n/m | n/m n/m |  |
| delete_ms_per_op | tantivy | n/m n/m | n/m n/m |  |
| delete_ms_per_op | lucene | n/m n/m | n/m n/m |  |
| ndcg@10 | sqlite-fts5 | TIE 1.01x | TIE 1.01x |  |
| ndcg@10 | tantivy | TIE 1.02x | TIE 1.01x |  |
| ndcg@10 | lucene | TIE 0.99x | TIE 0.99x |  |
| recall@100 | sqlite-fts5 | TIE 0.98x | TIE 0.98x |  |
| recall@100 | tantivy | TIE 1.00x | TIE 1.00x |  |
| recall@100 | lucene | TIE 1.00x | TIE 1.00x |  |

