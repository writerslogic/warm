<!-- BENCH:START -->

_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). W = WARM ahead (95% interval excludes parity), T = tie, L = a comparator is ahead, - = no verdict (unpinned source revision, no interval, or not comparable). Rows are compared only within one machine._

| Dataset | Machine | Metric | WARM | Best other | Advantage | |
|---|---|---|---|---|---|---|
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS @ recall@10 >= 0.95 (QPS) | 26,688 | hnswlib (native C++) 9,266 | 2.88x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency @ 0.95 (us) | 67 | hnswlib 178 | 2.66x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Build time (s) | 119.28 | hnswlib 162.87 | 1.37x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | RSS, serving (MiB) | 489 | hnswlib (native C++) 853 | 1.74x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Index size (MiB) | 576.3 | hnswlib 751.8 | 1.30x | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 3,377 | tantivy 1,669 | 2.02x | W |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 619 | tantivy 1,061 | 1.71x | W |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 1.46 | SQLite FTS5 1.35 | 0.93x | L |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 30 | SQLite FTS5 39 | 1.28x | W |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 9.0 | Apache Lucene 10.6.0 (JVM) 12.1 | 1.34x | W |
| memory | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 4,138 | tantivy 2,185 | 1.89x | W |
| memory | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 533 | tantivy 798 | 1.50x | W |
| memory | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 1.91 | SQLite FTS5 1.75 | 0.92x | L |
| memory | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 17 | SQLite FTS5 41 | 2.42x | W |
| memory | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 3.5 | Apache Lucene 10.6.0 (JVM) 10.3 | 2.92x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 11,499 | tantivy 4,990 | 2.30x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 197 | tantivy 377 | 1.91x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.88 | SQLite FTS5 0.84 | 0.96x | L |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 14 | SQLite FTS5 23 | 1.68x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 3.1 | Apache Lucene 10.6.0 (JVM) 10.3 | 3.29x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 12,373 | tantivy 7,333 | 1.69x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 164 | tantivy 280 | 1.71x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.44 | SQLite FTS5 0.20 | 0.46x | L |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 15 | SQLite FTS5 45 | 2.91x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.0 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.55x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 34,306 | tantivy 14,425 | 2.38x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 63 | tantivy 165 | 2.63x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.15 | SQLite FTS5 0.14 | 0.94x | L |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 13 | SQLite FTS5 28 | 2.19x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 0.9 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.63x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 5,534 | tantivy 3,106 | 1.78x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 236 | tantivy 474 | 2.01x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.58 | SQLite FTS5 0.30 | 0.52x | L |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 31 | SQLite FTS5 70 | 2.23x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.53x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 15,932 | tantivy 6,316 | 2.52x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 120 | tantivy 246 | 2.04x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.22 | SQLite FTS5 0.22 | 1.01x | T |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 29 | SQLite FTS5 52 | 1.82x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.60x | W |

<!-- BENCH:END -->
