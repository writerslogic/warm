<!-- BENCH:START -->

_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). W = WARM ahead (95% bootstrap interval of the factor excludes parity, at least 8 rounds a side), T = tie, L = a comparator is ahead, - = no verdict (fewer rounds than the rule needs, unverified measurement or source revision, no interval, or not comparable): the factor is as measured. Rows are compared only within one machine._

| Dataset | Machine | Metric | WARM | Best other | Advantage | |
|---|---|---|---|---|---|---|
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | QPS @ recall@10 >= 0.95 (QPS) | 1,895 | FAISS IVF-PQ fast-scan (+refine) 1,402 | 1.35x | - |
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | p99 latency @ 0.95 (us) | 721 | FAISS IVF-PQ fast-scan (+refine) 793 | 1.10x | - |
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | Index size (MiB) | 912.8 | FAISS IVF-PQ fast-scan (+refine) 490.3 | 0.54x | - |
| glove-100-angular | `linux-x86_64-gcp-n2-icelake-8c-16g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 0 / T 0 / L 0 | |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | QPS @ recall@10 >= 0.95 (QPS) | 14,645 | hnswlib (native C++) 7,427 | 1.97x | W |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | p99 latency @ 0.95 (us) | 96 | hnswlib (native C++) 194 | 2.02x | W |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | Index size (MiB) | 576.3 | FAISS IVF-PQ fast-scan (+refine) 528.4 | 0.92x | - |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 16 / T 0 / L 0 | |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | QPS @ recall@10 >= 0.95 (QPS) | 26,688 | hnswlib (native C++) 9,266 | 2.88x | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | p99 latency @ 0.95 (us) | 67 | hnswlib 178 | 2.66x | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Index size (MiB) | 576.3 | hnswlib 751.8 | 1.30x | - |
| sift-128-euclidean | `linux-x86_64-intel-ultra-9-185h-22c-27g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 0 / T 0 / L 0 | |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | QPS @ recall@10 >= 0.95 (QPS) | 26,552 | USearch 5,765 | 4.61x | - |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | p99 latency @ 0.95 (us) | 51 | USearch 239 | 4.72x | - |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | Index size (MiB) | 144.0 | sqlite-vec (exact) 128.1 | 0.89x | - |
| sift-128-euclidean | `linux-x86_64-gcp-n2-icelake-8c-16g` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W 0 / T 0 / L 0 | |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.2654 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 1,808 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 1,223 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 3.14 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 90 | n/m | n/m | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 82.7 | n/m | n/m | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.2654 | Apache Lucene 10.6.0 (JVM) 0.2657 | 1.00x | T |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 2,137 | Apache Lucene 10.6.0 (JVM) 1,822 | 1.17x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 1,137 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 1,402 | 1.23x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 2.44 | tantivy 0.27.0 (native Rust, pruning collector) 2.81 | 1.15x | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 39 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 92 | 2.34x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 25.3 | Apache Lucene 10.6.0 (JVM) 12.1 | 0.48x | L |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2654 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2678 | 0.99x | T |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 1,380 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 1,418 | 0.97x | L |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 597 | - | - | - |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 1,583 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 1,758 | 1.11x | W |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 3.84 | SQLite FTS5 (documents contract: docstore table read per hit) 2.96 | 0.77x | - |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 88 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 106 | 1.21x | W |
| beir:fiqa (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 82.7 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 41.2 | 0.50x | L |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 1,623 | Apache Lucene 10.6.0 (JVM) 1,725 | 0.94x | L |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 1,462 | Apache Lucene 10.6.0 (JVM) 1,572 | 1.08x | W |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 3.16 | SQLite FTS5 2.44 | 0.77x | - |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 37 | tantivy-py 0.27.0 (CPython binding) 54 | 1.45x | W |
| beir:fiqa (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 25.3 | Apache Lucene 10.6.0 (JVM) 12.1 | 0.48x | L |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2654 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2678 | 0.99x | T |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 1,543 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 1,430 | 1.08x | W |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 566 | - | - | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 1,518 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 1,776 | 1.17x | W |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 3.38 | SQLite FTS5 (documents contract: docstore table read per hit) 2.58 | 0.76x | - |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 89 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 105 | 1.18x | W |
| beir:fiqa (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 82.7 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 41.2 | 0.50x | L |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2654 | SQLite FTS5 0.2678 | 0.99x | T |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 1,844 | Apache Lucene 10.6.0 (JVM) 1,672 | 1.10x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 1,388 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 1,592 | 1.15x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 2.87 | SQLite FTS5 2.18 | 0.76x | - |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 38 | tantivy-py 0.27.0 (CPython binding) 53 | 1.37x | W |
| beir:fiqa (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 25.3 | Apache Lucene 10.6.0 (JVM) 12.1 | 0.48x | L |
| LongMemEval-dev session corpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.4627 | 0.99x | T |
| LongMemEval-dev session corpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 1,712 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,077 | 1.59x | W |
| LongMemEval-dev session corpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 954 | - | - | - |
| LongMemEval-dev session corpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 912 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,327 | 1.46x | W |
| LongMemEval-dev session corpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 2.66 | SQLite FTS5 (documents contract: docstore table read per hit) 1.96 | 0.74x | - |
| LongMemEval-dev session corpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 62 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 68 | 1.09x | W |
| LongMemEval-dev session corpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 93.5 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 35.6 | 0.38x | L |
| LongMemEval-dev session corpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| LongMemEval-dev session corpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 3,723 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 3,938 | 0.95x | L |
| LongMemEval-dev session corpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 554 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 552 | 1.00x | T |
| LongMemEval-dev session corpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 1.71 | SQLite FTS5 1.61 | 0.94x | - |
| LongMemEval-dev session corpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 14 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 24 | 1.76x | W |
| LongMemEval-dev session corpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 6.6 | Apache Lucene 10.6.0 (JVM) 10.3 | 1.56x | W |
| LongMemEval-dev session corpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.4627 | 0.99x | T |
| LongMemEval-dev session corpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 1,516 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,073 | 1.41x | W |
| LongMemEval-dev session corpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 1,033 | - | - | - |
| LongMemEval-dev session corpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 1,125 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 1,477 | 1.31x | W |
| LongMemEval-dev session corpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 2.29 | SQLite FTS5 (documents contract: docstore table read per hit) 1.67 | 0.73x | - |
| LongMemEval-dev session corpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 63 | tantivy-py 0.27.0 (CPython binding, documents contract: stored document read per hit) 66 | 1.05x | W |
| LongMemEval-dev session corpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 93.5 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 35.6 | 0.38x | L |
| LongMemEval-dev session corpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.4561 | Apache Lucene 10.6.0 (JVM) 0.4627 | 0.99x | T |
| LongMemEval-dev session corpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 3,614 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 4,697 | 0.77x | L |
| LongMemEval-dev session corpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 608 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 465 | 0.76x | L |
| LongMemEval-dev session corpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 1.60 | SQLite FTS5 1.52 | 0.95x | - |
| LongMemEval-dev session corpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 15 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 25 | 1.72x | W |
| LongMemEval-dev session corpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 6.6 | Apache Lucene 10.6.0 (JVM) 10.3 | 1.56x | W |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.7283 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 3,332 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 756 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 3.53 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 88 | n/m | n/m | - |
| msmarco:100k (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 64.4 | n/m | n/m | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.7283 | tantivy 0.27.0 (native Rust, pruning collector) 0.7279 | 1.00x | T |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 4,115 | Apache Lucene 10.6.0 (JVM) 4,042 | 1.02x | W |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 695 | Apache Lucene 10.6.0 (JVM) 907 | 1.31x | W |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 2.92 | tantivy 0.27.0 (native Rust, pruning collector) 2.53 | 0.86x | - |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 40 | Apache Lucene 10.6.0 (JVM) 151 | 3.79x | W |
| msmarco:100k (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 21.4 | Apache Lucene 10.6.0 (JVM) 10.8 | 0.51x | L |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.4746 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 818 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 4,080 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 43.24 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 546 | n/m | n/m | - |
| msmarco:1m (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 518.3 | n/m | n/m | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | nDCG@10 | 0.4746 | tantivy 0.27.0 (native Rust, pruning collector) 0.4779 | 0.99x | T |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | QPS (single thread) (QPS) | 836 | Apache Lucene 10.6.0 (JVM) 807 | 1.04x | W |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | p99 latency (us) | 4,150 | Apache Lucene 10.6.0 (JVM) 4,476 | 1.08x | W |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Ingest/build time (s) | 32.40 | Apache Lucene 10.6.0 (JVM) 27.47 | 0.85x | - |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | Peak RSS, serving (MiB) | 182 | Apache Lucene 10.6.0 (JVM) 257 | 1.41x | W |
| msmarco:1m (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-47g` | On-disk size (MiB) | 160.6 | Apache Lucene 10.6.0 (JVM) 98.5 | 0.61x | L |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2916 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2914 | 1.00x | T |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 9,039 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 8,116 | 1.11x | W |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 5,284 | - | - | - |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 241 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 299 | 1.24x | W |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.39 | SQLite FTS5 (documents contract: docstore table read per hit) 0.28 | 0.72x | - |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 19 | SQLite FTS5 (documents contract: docstore table read per hit) 49 | 2.56x | W |
| beir:nfcorpus (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 12.7 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 4.8 | 0.38x | L |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.2916 | LanceDB FTS 0.2956 | 0.99x | T |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 19,321 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 22,506 | 0.86x | L |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 149 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 139 | 0.93x | T |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.31 | SQLite FTS5 0.22 | 0.72x | - |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 10 | SQLite FTS5 31 | 3.15x | W |
| beir:nfcorpus (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 1.7 | Apache Lucene 10.6.0 (JVM) 1.5 | 0.90x | L |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2916 | SQLite FTS5 (documents contract: docstore table read per hit) 0.2914 | 1.00x | T |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 10,311 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 7,916 | 1.30x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 5,780 | - | - | - |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 262 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 271 | 1.03x | T |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.36 | SQLite FTS5 (documents contract: docstore table read per hit) 0.25 | 0.69x | - |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 20 | SQLite FTS5 (documents contract: docstore table read per hit) 47 | 2.35x | W |
| beir:nfcorpus (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 12.7 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 4.8 | 0.38x | L |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.2916 | LanceDB FTS 0.2956 | 0.99x | T |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 21,308 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 24,331 | 0.88x | L |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 153 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 138 | 0.90x | T |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.28 | SQLite FTS5 0.21 | 0.74x | - |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 11 | SQLite FTS5 29 | 2.66x | W |
| beir:nfcorpus (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 1.7 | Apache Lucene 10.6.0 (JVM) 1.5 | 0.90x | L |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.7022 | 1.00x | T |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 3,575 | Apache Lucene 10.6.0 (JVM, documents contract: payload in doc values instead of stored fields) 3,465 | 1.03x | T |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 2,384 | - | - | - |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 406 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 492 | 1.21x | W |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.55 | SQLite FTS5 (documents contract: docstore table read per hit) 0.39 | 0.71x | - |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 37 | SQLite FTS5 (documents contract: docstore table read per hit) 102 | 2.79x | W |
| beir:scifact (documents) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 17.2 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 6.6 | 0.39x | L |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | QPS (single thread) (QPS) | 5,252 | Apache Lucene 10.6.0 (JVM) 5,624 | 0.93x | L |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | p99 latency (us) | 312 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 316 | 1.01x | T |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Ingest/build time (s) | 0.44 | SQLite FTS5 0.33 | 0.74x | - |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | Peak RSS, serving (MiB) | 16 | SQLite FTS5 53 | 3.31x | W |
| beir:scifact (ids) | `linux-aarch64-neoverse-n1-8c-31g` | On-disk size (MiB) | 2.4 | Apache Lucene 10.6.0 (JVM) 2.0 | 0.84x | L |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 0.7022 | 1.00x | T |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 4,145 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 3,829 | 1.08x | W |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS), WARM with eager ingest (reference, feeds no verdict) | 2,818 | - | - | - |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 381 | tantivy 0.27.0 (native Rust, exhaustive collector with Count, documents contract) 383 | 1.01x | T |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.45 | SQLite FTS5 (documents contract: docstore table read per hit) 0.31 | 0.71x | - |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 37 | SQLite FTS5 (documents contract: docstore table read per hit) 100 | 2.67x | W |
| beir:scifact (documents) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 17.2 | Apache Lucene 10.6.0 (JVM, documents contract: stored fields read per hit) 6.6 | 0.39x | L |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | nDCG@10 | 0.7027 | Apache Lucene 10.6.0 (JVM) 0.7022 | 1.00x | T |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | QPS (single thread) (QPS) | 5,932 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 7,331 | 0.81x | L |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | p99 latency (us) | 302 | tantivy 0.27.0 (native Rust, exhaustive collector with Count) 231 | 0.76x | L |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Ingest/build time (s) | 0.37 | SQLite FTS5 0.28 | 0.77x | - |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | Peak RSS, serving (MiB) | 17 | SQLite FTS5 51 | 3.02x | W |
| beir:scifact (ids) | `linux-x86_64-intel-xeon-2-60ghz-8c-16g` | On-disk size (MiB) | 2.4 | Apache Lucene 10.6.0 (JVM) 2.0 | 0.84x | L |
| fiqa (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | nDCG@10 | 0.3919 | Vectra 0.3919 | +0.0000 nDCG | T |
| fiqa (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | QPS | 71 | Vectra 31 | 2.29x | W |
| fiqa (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | p50 (us) | 14,084 | Vectra 32,343 | 2.30x | W |
| fiqa (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Build (s) | 5.56 | Vectra 14.42 | 2.59x | W |
| fiqa (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Index (MiB) | 318.3 | Vectra 447.6 | 1.41x | W |
| memory (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | nDCG@10 | 0.2848 | Vectra 0.2847 | +0.0001 nDCG | T |
| memory (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | QPS | 1,257 | Vectra 435 | 2.89x | W |
| memory (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | p50 (us) | 776 | Vectra 2,244 | 2.89x | W |
| memory (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Build (s) | 0.56 | Vectra 0.98 | 1.75x | W |
| memory (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Index (MiB) | 25.3 | Vectra 35.5 | 1.40x | W |
| nfcorpus (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | nDCG@10 | 0.3109 | Vectra 0.3106 | +0.0003 nDCG | T |
| nfcorpus (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | QPS | 1,439 | Vectra 559 | 2.57x | W |
| nfcorpus (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | p50 (us) | 659 | Vectra 1,774 | 2.69x | W |
| nfcorpus (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Build (s) | 0.43 | Vectra 0.78 | 1.79x | W |
| nfcorpus (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Index (MiB) | 20.1 | Vectra 28.2 | 1.40x | W |
| scifact (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | nDCG@10 | 0.7461 | Vectra 0.7461 | +0.0000 nDCG | T |
| scifact (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | QPS | 1,033 | Vectra 388 | 2.67x | W |
| scifact (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | p50 (us) | 929 | Vectra 2,563 | 2.76x | W |
| scifact (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Build (s) | 0.63 | Vectra 1.09 | 1.73x | W |
| scifact (JS, vs Vectra 0.15.0) | `gcp-n2-icelake-8c-16g` | Index (MiB) | 28.7 | Vectra 40.3 | 1.40x | W |

<!-- BENCH:END -->
