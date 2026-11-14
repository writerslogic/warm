<!-- BENCH:START -->

_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). W = WARM ahead (95% interval excludes parity), T = tie, L = a comparator is ahead, - = no verdict (unpinned source revision, no interval, or not comparable). Rows are compared only within one machine._

| Dataset | Machine | Metric | WARM | Best other | Advantage | |
|---|---|---|---|---|---|---|
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS @ recall@10 >= 0.95 (QPS) | 26,688 | hnswlib (native C++) 9,266 | 2.88x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency @ 0.95 (us) | 67 | hnswlib 178 | 2.66x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Index size (MiB) | 576.3 | hnswlib 751.8 | 1.30x | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 10 / T 0 / L 2 | |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 3,108 | Apache Lucene 10.6.0 (JVM) 2,851 | 1.09x | T |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM lexical-only API that hydrates the store (reference, feeds no verdict) | 715 | - | - | - |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 659 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 860 | 1.30x | W |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 1.44 | SQLite FTS5 1.29 | 0.90x | L |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 30 | tantivy-py 0.27.0 (CPython binding) 52 | 1.72x | W |
| beir:fiqa | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 9.0 | Apache Lucene 10.6.0 (JVM) 12.1 | 1.34x | W |
| memory | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 4,138 | tantivy-py 0.27.0 (CPython binding) 2,185 | 1.89x | W |
| memory | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 4,840 | - | - | - |
| memory | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 533 | tantivy-py 0.27.0 (CPython binding) 798 | 1.50x | W |
| memory | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 1.91 | SQLite FTS5 1.75 | 0.92x | L |
| memory | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 17 | SQLite FTS5 41 | 2.42x | W |
| memory | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 3.5 | Apache Lucene 10.6.0 (JVM) 10.3 | 2.92x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 10,973 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 8,784 | 1.25x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM lexical-only API that hydrates the store (reference, feeds no verdict) | 2,588 | - | - | - |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 174 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 260 | 1.50x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.90 | SQLite FTS5 0.89 | 0.99x | L |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 14 | tantivy 0.27.0 (native Rust, pruning collector) 25 | 1.78x | W |
| memory | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 3.1 | Apache Lucene 10.6.0 (JVM) 10.3 | 3.29x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 12,373 | tantivy-py 0.27.0 (CPython binding) 7,333 | 1.69x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 19,896 | - | - | - |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 164 | tantivy-py 0.27.0 (CPython binding) 280 | 1.71x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.44 | SQLite FTS5 0.20 | 0.46x | L |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 15 | SQLite FTS5 45 | 2.91x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.0 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.55x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 30,265 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 43,275 | 0.70x | L |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM lexical-only API that hydrates the store (reference, feeds no verdict) | 23,961 | - | - | - |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 72 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 73 | 1.02x | T |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.16 | SQLite FTS5 0.14 | 0.87x | L |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 13 | SQLite FTS5 29 | 2.20x | W |
| beir:nfcorpus | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 0.9 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.63x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 5,534 | tantivy-py 0.27.0 (CPython binding) 3,106 | 1.78x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 7,018 | - | - | - |
| beir:scifact | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 236 | tantivy-py 0.27.0 (CPython binding) 474 | 2.01x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.58 | SQLite FTS5 0.30 | 0.52x | L |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 31 | SQLite FTS5 70 | 2.23x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.53x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 14,563 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 11,941 | 1.22x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM lexical-only API that hydrates the store (reference, feeds no verdict) | 4,326 | - | - | - |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 118 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 151 | 1.28x | T |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.23 | SQLite FTS5 0.20 | 0.89x | L |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 29 | SQLite FTS5 51 | 1.77x | W |
| beir:scifact | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.60x | W |

<!-- BENCH:END -->
