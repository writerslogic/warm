<!-- BENCH:START -->

_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). W = WARM ahead (95% interval excludes parity), T = tie, L = a comparator is ahead, - = no verdict (unpinned source revision, no interval, or not comparable). Rows are compared only within one machine._

| Dataset | Machine | Metric | WARM | Best other | Advantage | |
|---|---|---|---|---|---|---|
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS @ recall@10 >= 0.95 (QPS) | 17,481 | hnswlib 7,630 | n/v | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency @ 0.95 (us) | 105 | hnswlib 220 | n/v | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Build time (s) | 312.60 | hnswlib 182.49 | n/v | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | RSS, serving (MiB) | 2,204 | hnswlib 1,019 | n/v | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Index size (MiB) | 446.3 | hnswlib 873.8 | n/v | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | n/v | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 3,612 | tantivy 1,617 | n/v | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 509 | tantivy 1,269 | n/v | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 2.16 | SQLite FTS5 1.18 | n/v | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | n/m | n/m | n/m | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 20.6 | Apache Lucene 9.12.3 (JVM) 12.2 | n/v | - |
| memory | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.4561 | Apache Lucene 9.12.3 (JVM) 0.4627 | 0.99x | T |
| memory | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 2,289 | tantivy 2,504 | 0.91x | T |
| memory | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 775 | tantivy 720 | 0.93x | T |
| memory | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 2.52 | SQLite FTS5 2.06 | 0.82x | L |
| memory | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 66 | SQLite FTS5 26 | 0.39x | L |
| memory | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 9.8 | Apache Lucene 9.12.3 (JVM) 10.3 | 1.05x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.4561 | Apache Lucene 9.12.3 (JVM) 0.4627 | n/v | - |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 12,629 | tantivy 4,850 | n/v | - |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 145 | tantivy 396 | n/v | - |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 1.21 | SQLite FTS5 0.88 | n/v | - |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | n/m | n/m | n/m | - |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 9.8 | Apache Lucene 9.12.3 (JVM) 10.3 | n/v | - |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | n/v | - |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 40,784 | tantivy 15,413 | n/v | - |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 57 | tantivy 150 | n/v | - |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.23 | SQLite FTS5 0.14 | n/v | - |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | n/m | n/m | n/m | - |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 2.1 | Apache Lucene 9.12.3 (JVM) 1.5 | n/v | - |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 10,572 | tantivy 7,151 | 1.48x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 279 | tantivy 294 | 1.05x | T |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.59 | SQLite FTS5 0.29 | 0.48x | L |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 57 | SQLite FTS5 30 | 0.53x | L |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 2.1 | Apache Lucene 9.12.3 (JVM) 1.5 | 0.72x | L |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.7027 | Apache Lucene 9.12.3 (JVM) 0.7022 | n/v | - |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 17,782 | tantivy 6,450 | n/v | - |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 95 | tantivy 287 | n/v | - |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.31 | SQLite FTS5 0.20 | n/v | - |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 196 | tantivy 174 | n/v | - |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 2.9 | Apache Lucene 9.12.3 (JVM) 2.0 | n/v | - |
| beir:scifact | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.7027 | Apache Lucene 9.12.3 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 2,733 | tantivy 2,965 | 0.92x | L |
| beir:scifact | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 518 | tantivy 508 | 0.98x | T |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 1.42 | SQLite FTS5 0.43 | 0.30x | L |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 160 | SQLite FTS5 54 | 0.34x | L |
| beir:scifact | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 2.9 | Apache Lucene 9.12.3 (JVM) 2.0 | 0.68x | L |

<!-- BENCH:END -->
