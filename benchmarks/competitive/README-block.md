<!-- BENCH:START -->

_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). W = WARM ahead (95% interval excludes parity), T = tie, L = a comparator is ahead, - = no verdict (unpinned source revision, no interval, or not comparable). Rows are compared only within one machine._

| Dataset | Machine | Metric | WARM | Best other | Advantage | |
|---|---|---|---|---|---|---|
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS @ recall@10 >= 0.95 (QPS) | 23,918 | hnswlib 8,125 | 2.94x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency @ 0.95 (us) | 97 | hnswlib 251 | 2.60x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Build time (s) | 121.44 | hnswlib 170.02 | 1.40x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | RSS, serving (MiB) | 490 | hnswlib 898 | 1.83x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Index size (MiB) | 1,389.1 | hnswlib 751.8 | 0.54x | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 1,269 | Apache Lucene 10.6.0 (JVM) 1,625 | 0.78x | L |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 1,828 | Apache Lucene 10.6.0 (JVM) 1,486 | 0.81x | L |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 1.55 | SQLite FTS5 1.44 | 0.93x | T |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 31 | SQLite FTS5 39 | 1.26x | W |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 9.7 | Apache Lucene 10.6.0 (JVM) 12.1 | 1.25x | W |
| memory | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 4,138 | tantivy 2,185 | 1.89x | W |
| memory | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 533 | tantivy 798 | 1.50x | W |
| memory | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 1.91 | SQLite FTS5 1.75 | 0.92x | L |
| memory | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 17 | SQLite FTS5 41 | 2.42x | W |
| memory | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 3.5 | Apache Lucene 10.6.0 (JVM) 10.3 | 2.92x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 8,417 | tantivy 4,962 | 1.70x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 284 | tantivy 482 | 1.70x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.87 | SQLite FTS5 0.84 | 0.97x | L |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 14 | SQLite FTS5 24 | 1.69x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 3.5 | Apache Lucene 10.6.0 (JVM) 10.3 | 2.92x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 12,373 | tantivy 7,333 | 1.69x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 164 | tantivy 280 | 1.71x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.44 | SQLite FTS5 0.20 | 0.46x | L |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 15 | SQLite FTS5 45 | 2.91x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.0 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.55x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 23,585 | tantivy 14,207 | 1.66x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 153 | tantivy 245 | 1.60x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.15 | SQLite FTS5 0.14 | 0.94x | L |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 13 | SQLite FTS5 28 | 2.22x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 1.0 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.55x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 5,534 | tantivy 3,106 | 1.78x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 236 | tantivy 474 | 2.01x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.58 | SQLite FTS5 0.30 | 0.52x | L |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 31 | SQLite FTS5 70 | 2.23x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.53x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 12,254 | tantivy 6,133 | 2.00x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 220 | tantivy 402 | 1.83x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.22 | SQLite FTS5 0.20 | 0.91x | L |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 29 | SQLite FTS5 53 | 1.84x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.53x | W |

<!-- BENCH:END -->
