<!-- BENCH:START -->

_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). W = WARM ahead (95% interval excludes parity), T = tie, L = a comparator is ahead, - = no verdict (unpinned source revision, no interval, or not comparable). Rows are compared only within one machine._

| Dataset | Machine | Metric | WARM | Best other | Advantage | |
|---|---|---|---|---|---|---|
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS @ recall@10 >= 0.95 (QPS) | 26,688 | hnswlib (native C++) 9,266 | 2.88x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency @ 0.95 (us) | 67 | hnswlib 178 | 2.66x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Index size (MiB) | 576.3 | hnswlib 751.8 | 1.30x | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 10 / T 0 / L 2 | |
| beir:fiqa (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2654 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2678 | 0.99x | T |
| beir:fiqa (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 2,760 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,969 | 1.40x | W |
| beir:fiqa (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 1,873 | - | - | - |
| beir:fiqa (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 701 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,022 | 1.46x | W |
| beir:fiqa (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 1.75 | SQLite FTS5 (documents contract: docstore table read per hit) 1.42 | 0.81x | L |
| beir:fiqa (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 73 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 105 | 1.44x | W |
| beir:fiqa (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 93.6 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 41.2 | 0.44x | L |
| beir:fiqa (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 3,458 | Apache Lucene 10.6.0 (JVM) 2,975 | 1.16x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 643 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 827 | 1.29x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 1.39 | SQLite FTS5 1.28 | 0.92x | L |
| beir:fiqa (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 24 | tantivy-py 0.27.0 (CPython binding) 53 | 2.21x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 9.0 | Apache Lucene 10.6.0 (JVM) 12.1 | 1.34x | W |
| memory | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 4,138 | tantivy-py 0.27.0 (CPython binding) 2,185 | 1.89x | W |
| memory | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 4,840 | - | - | - |
| memory | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 533 | tantivy-py 0.27.0 (CPython binding) 798 | 1.50x | W |
| memory | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 1.91 | SQLite FTS5 1.75 | 0.92x | L |
| memory | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 17 | SQLite FTS5 41 | 2.42x | W |
| memory | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 3.5 | Apache Lucene 10.6.0 (JVM) 10.3 | 2.92x | W |
| memory (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.4627 | 0.99x | T |
| memory (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 4,140 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 2,190 | 1.89x | W |
| memory (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 2,023 | - | - | - |
| memory (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 382 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 675 | 1.77x | W |
| memory (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 1.05 | SQLite FTS5 (documents contract: docstore table read per hit) 0.94 | 0.89x | L |
| memory (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 61 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 66 | 1.10x | W |
| memory (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 92.8 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 35.6 | 0.38x | L |
| memory (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 18,016 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 8,657 | 2.08x | W |
| memory (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 129 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 247 | 1.92x | W |
| memory (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.85 | SQLite FTS5 0.87 | 1.02x | T |
| memory (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 13 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 25 | 1.99x | W |
| memory (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 3.1 | Apache Lucene 10.6.0 (JVM) 10.3 | 3.29x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 12,373 | tantivy-py 0.27.0 (CPython binding) 7,333 | 1.69x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 19,896 | - | - | - |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 164 | tantivy-py 0.27.0 (CPython binding) 280 | 1.71x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.44 | SQLite FTS5 0.20 | 0.46x | L |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 15 | SQLite FTS5 45 | 2.91x | W |
| beir:nfcorpus | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.0 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.55x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2916 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2914 | 1.00x | T |
| beir:nfcorpus (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 20,489 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 15,058 | 1.36x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 12,702 | - | - | - |
| beir:nfcorpus (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 102 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 145 | 1.42x | T |
| beir:nfcorpus (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.21 | SQLite FTS5 (documents contract: docstore table read per hit) 0.19 | 0.88x | L |
| beir:nfcorpus (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 19 | SQLite FTS5 (documents contract: docstore table read per hit) 47 | 2.52x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 12.0 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 4.8 | 0.40x | L |
| beir:nfcorpus (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 62,518 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 42,293 | 1.48x | W |
| beir:nfcorpus (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 47 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 72 | 1.53x | W |
| beir:nfcorpus (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.15 | SQLite FTS5 0.13 | 0.90x | L |
| beir:nfcorpus (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 10 | SQLite FTS5 29 | 2.90x | W |
| beir:nfcorpus (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 0.9 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.63x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS) | 5,534 | tantivy-py 0.27.0 (CPython binding) 3,106 | 1.78x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 7,018 | - | - | - |
| beir:scifact | `modal-x86_64-8vcpu-32g` | p99 latency (us) | 236 | tantivy-py 0.27.0 (CPython binding) 474 | 2.01x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Ingest/build time (s) | 0.58 | SQLite FTS5 0.30 | 0.52x | L |
| beir:scifact | `modal-x86_64-8vcpu-32g` | Peak RSS, serving (MiB) | 31 | SQLite FTS5 70 | 2.23x | W |
| beir:scifact | `modal-x86_64-8vcpu-32g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.53x | W |
| beir:scifact (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.7022 | 1.00x | T |
| beir:scifact (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 10,251 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 6,606 | 1.55x | W |
| beir:scifact (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 6,078 | - | - | - |
| beir:scifact (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 202 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 246 | 1.22x | T |
| beir:scifact (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.28 | SQLite FTS5 (documents contract: docstore table read per hit) 0.25 | 0.88x | L |
| beir:scifact (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 36 | SQLite FTS5 (documents contract: docstore table read per hit) 100 | 2.74x | W |
| beir:scifact (documents) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 16.1 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 6.6 | 0.41x | L |
| beir:scifact (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS (single thread) (QPS) | 22,711 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 11,709 | 1.94x | W |
| beir:scifact (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency (us) | 95 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 162 | 1.70x | W |
| beir:scifact (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Ingest/build time (s) | 0.22 | SQLite FTS5 0.20 | 0.92x | L |
| beir:scifact (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Peak RSS, serving (MiB) | 18 | SQLite FTS5 51 | 2.83x | W |
| beir:scifact (ids) | `linux-x86_64-intel-ultra-9-185h-22c-27g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.60x | W |

<!-- BENCH:END -->
