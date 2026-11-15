<!-- BENCH:START -->

_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). W = WARM ahead (95% interval excludes parity), T = tie, L = a comparator is ahead, - = no verdict (unpinned source revision, no interval, or not comparable). Rows are compared only within one machine._

| Dataset | Machine | Metric | WARM | Best other | Advantage | |
|---|---|---|---|---|---|---|
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | QPS @ recall@10 >= 0.95 (QPS) | 2,372 | FAISS IVF-PQ fast-scan (+refine) 1,402 | 1.69x | W |
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | p99 latency @ 0.95 (us) | 559 | FAISS IVF-PQ fast-scan (+refine) 793 | 1.42x | W |
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | Index size (MiB) | 912.8 | FAISS IVF-PQ fast-scan (+refine) 490.3 | 0.54x | - |
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 9 / T 0 / L 0 | |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | QPS @ recall@10 >= 0.95 (QPS) | 16,856 | hnswlib (native C++) 7,427 | 2.27x | W |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | p99 latency @ 0.95 (us) | 85 | hnswlib (native C++) 194 | 2.28x | W |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | Index size (MiB) | 576.3 | FAISS IVF-PQ fast-scan (+refine) 528.4 | 0.92x | - |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 16 / T 0 / L 0 | |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS @ recall@10 >= 0.95 (QPS) | 26,688 | hnswlib (native C++) 9,266 | 2.88x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency @ 0.95 (us) | 67 | hnswlib 178 | 2.66x | W |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Index size (MiB) | 576.3 | hnswlib 751.8 | 1.30x | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 10 / T 0 / L 2 | |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.2654 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 1,802 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 1,156 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 2.99 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 77 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 94.1 | n/m | n/m | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.2654 | Apache Lucene 10.6.0 (JVM) 0.2657 | n/v | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 2,199 | Apache Lucene 10.6.0 (JVM) 1,623 | n/v | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 1,041 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 1,419 | n/v | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 2.17 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 2.87 | n/v | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 28 | tantivy 0.27.0 (native Rust, pruning collector) 92 | n/v | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 9.5 | Apache Lucene 10.6.0 (JVM) 12.1 | n/v | - |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2654 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2678 | 0.99x | T |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 1,011 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 1,164 | 0.87x | T |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 632 | - | - | - |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 1,976 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 2,333 | 1.18x | W |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 3.19 | SQLite FTS5 (documents contract: docstore table read per hit) 2.97 | 0.93x | L |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 72 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 108 | 1.50x | W |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 93.6 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 41.2 | 0.44x | L |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 1,159 | Apache Lucene 10.6.0 (JVM) 1,184 | 0.98x | T |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 1,854 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 2,101 | 1.13x | W |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 2.65 | SQLite FTS5 2.49 | 0.94x | L |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 23 | tantivy-py 0.27.0 (CPython binding) 56 | 2.43x | W |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 9.0 | Apache Lucene 10.6.0 (JVM) 12.1 | 1.34x | W |
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
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2654 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2678 | 0.99x | T |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 1,032 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 1,439 | 0.72x | T |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 659 | - | - | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 2,047 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,624 | 0.79x | L |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 2.59 | SQLite FTS5 (documents contract: docstore table read per hit) 2.46 | 0.95x | L |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 74 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 107 | 1.46x | W |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 93.6 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 41.2 | 0.44x | L |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 1,168 | Apache Lucene 10.6.0 (JVM) 1,684 | 0.69x | T |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 1,907 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 1,416 | 0.74x | L |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 2.15 | SQLite FTS5 2.07 | 0.96x | L |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 24 | tantivy-py 0.27.0 (CPython binding) 55 | 2.24x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 9.0 | Apache Lucene 10.6.0 (JVM) 12.1 | 1.34x | W |
| memory (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.4627 | 0.99x | T |
| memory (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 2,550 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,065 | 2.39x | W |
| memory (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 912 | - | - | - |
| memory (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 577 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,343 | 2.33x | W |
| memory (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 1.78 | SQLite FTS5 (documents contract: docstore table read per hit) 1.97 | 1.11x | W |
| memory (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 60 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 69 | 1.16x | W |
| memory (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 92.8 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 35.6 | 0.38x | L |
| memory (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 8,104 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 3,933 | 2.06x | W |
| memory (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 295 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 563 | 1.91x | W |
| memory (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 1.55 | SQLite FTS5 1.62 | 1.04x | T |
| memory (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 12 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 25 | 2.08x | W |
| memory (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 3.1 | Apache Lucene 10.6.0 (JVM) 10.3 | 3.29x | W |
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
| memory (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.4627 | 0.99x | T |
| memory (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 2,631 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,250 | 2.10x | W |
| memory (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 1,089 | - | - | - |
| memory (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 576 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,112 | 1.93x | W |
| memory (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 1.55 | SQLite FTS5 (documents contract: docstore table read per hit) 1.75 | 1.13x | W |
| memory (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 61 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 68 | 1.12x | W |
| memory (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 92.8 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 35.6 | 0.38x | L |
| memory (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| memory (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 9,360 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 5,373 | 1.74x | W |
| memory (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 272 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 396 | 1.46x | W |
| memory (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 1.31 | SQLite FTS5 1.38 | 1.05x | W |
| memory (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 13 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 26 | 1.94x | W |
| memory (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 3.1 | Apache Lucene 10.6.0 (JVM) 10.3 | 3.29x | W |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.7283 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 3,284 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 783 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 3.60 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 80 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 76.6 | n/m | n/m | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.7283 | tantivy 0.27.0 (native Rust, pruning collector) 0.7279 | n/v | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 4,395 | Apache Lucene 10.6.0 (JVM) 2,574 | n/v | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 704 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 1,329 | n/v | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 2.91 | tantivy 0.27.0 (native Rust, pruning collector) 2.60 | n/v | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 35 | Apache Lucene 10.6.0 (JVM) 126 | n/v | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 12.0 | Apache Lucene 10.6.0 (JVM) 10.8 | n/v | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.4746 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 768 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 4,304 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 40.24 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 500 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 755.8 | n/m | n/m | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.4746 | tantivy 0.27.0 (native Rust, pruning collector) 0.4779 | n/v | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 830 | Apache Lucene 10.6.0 (JVM) 799 | n/v | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 4,181 | Apache Lucene 10.6.0 (JVM) 4,578 | n/v | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 32.06 | Apache Lucene 10.6.0 (JVM) 29.42 | n/v | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 164 | Apache Lucene 10.6.0 (JVM) 233 | n/v | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 110.6 | Apache Lucene 10.6.0 (JVM) 98.5 | n/v | - |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2916 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2914 | 1.00x | T |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 9,693 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 6,694 | 1.45x | W |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 4,595 | - | - | - |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 198 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 305 | 1.54x | W |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.33 | SQLite FTS5 (documents contract: docstore table read per hit) 0.29 | 0.87x | L |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 18 | SQLite FTS5 (documents contract: docstore table read per hit) 49 | 2.74x | W |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 12.0 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 4.8 | 0.40x | L |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 33,428 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 22,438 | 1.49x | W |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 84 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 138 | 1.64x | W |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.27 | SQLite FTS5 0.23 | 0.84x | L |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 9 | SQLite FTS5 31 | 3.39x | W |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 0.9 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.63x | W |
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
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2916 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2914 | 1.00x | T |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 11,605 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 8,814 | 1.32x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 5,931 | - | - | - |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 157 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 229 | 1.46x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.27 | SQLite FTS5 (documents contract: docstore table read per hit) 0.24 | 0.88x | L |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 19 | SQLite FTS5 (documents contract: docstore table read per hit) 47 | 2.43x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 12.0 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 4.8 | 0.40x | L |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2916 | SQLite FTS5 0.2914 | 1.00x | T |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 40,574 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 27,155 | 1.49x | W |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 62 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 109 | 1.75x | W |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.22 | SQLite FTS5 0.19 | 0.87x | L |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 11 | SQLite FTS5 29 | 2.71x | W |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 0.9 | Apache Lucene 10.6.0 (JVM) 1.5 | 1.63x | W |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.7022 | 1.00x | T |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 4,959 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 2,893 | 1.71x | W |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 2,266 | - | - | - |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 269 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 496 | 1.84x | W |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.47 | SQLite FTS5 (documents contract: docstore table read per hit) 0.40 | 0.85x | L |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 35 | SQLite FTS5 (documents contract: docstore table read per hit) 102 | 2.87x | W |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 16.1 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 6.6 | 0.41x | L |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 11,437 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 5,566 | 2.05x | W |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 140 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 315 | 2.25x | W |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.39 | SQLite FTS5 0.33 | 0.87x | L |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 17 | SQLite FTS5 53 | 3.05x | W |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.60x | W |
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
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.7022 | 1.00x | T |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 5,920 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 3,856 | 1.54x | W |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 2,768 | - | - | - |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 221 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 371 | 1.68x | W |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.39 | SQLite FTS5 (documents contract: docstore table read per hit) 0.33 | 0.85x | L |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 37 | SQLite FTS5 (documents contract: docstore table read per hit) 100 | 2.69x | W |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 16.1 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 6.6 | 0.41x | L |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 13,922 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 7,322 | 1.90x | W |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 112 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 233 | 2.08x | W |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.32 | SQLite FTS5 0.28 | 0.87x | L |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 19 | SQLite FTS5 51 | 2.73x | W |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 1.3 | Apache Lucene 10.6.0 (JVM) 2.0 | 1.60x | W |

<!-- BENCH:END -->
