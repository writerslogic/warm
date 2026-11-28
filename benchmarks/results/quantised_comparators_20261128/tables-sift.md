
### linux-x86_64-gcp-c3d-genoa-8c-16g, sift-128-euclidean n=1000000, 8 rounds, commit dbe04ad6

| recall | comparator | build | traversal | their QPS | their params | recall (report half) | WARM / theirs, grid | WARM / theirs, interpolated |
|---|---|---|---|---|---|---|---|---|
| 0.9 | faiss-hnsw | m32e500 | float32 | 9435 | {'ef': 24} | 0.9232 | 2.69 [2.65, 2.81] | 2.42 [2.37, 2.51] |
| 0.9 | faiss-hnswsq | sq8_m32e500 | 8-bit SQ + refine | 11961 | {'ef': 24, 'k_factor': 1} | 0.9153 | 2.13 [2.09, 2.18] | 2.03 [1.98, 2.06] |
| 0.9 | faiss-hnswsq | sq4_m32e500 | 4-bit SQ + refine | 10259 | {'ef': 32, 'k_factor': 2} | 0.9159 | 2.48 [2.41, 2.54] | 2.40 [2.34, 2.43] |
| 0.9 | usearch | m32e500 | float32 | 4374 | {'ef': 24} | 0.9256 | 5.81 [5.67, 5.95] | 5.23 [5.11, 5.29] |
| 0.9 | usearch | m32e500_i8 | int8 | 4500 | {'ef': 48} | 0.9032 | 5.65 [5.54, 5.76] | 5.52 [5.45, 5.60] |
| 0.9 | usearch | m32e500_f16 | float16 | 5489 | {'ef': 24} | 0.9258 | 4.63 [4.56, 4.73] | 4.18 [4.09, 4.27] |
| 0.9 | WARM | vertex-d32 | 8-bit codes + exact rerank | 25423 | {'ef': 16, 'rerank': 64} | 0.9028 | 1 | 1 |
| 0.95 | faiss-hnsw | m32e500 | float32 | 7617 | {'ef': 32} | 0.9514 | 2.09 [2.02, 2.12] | 2.28 [2.23, 2.31] |
| 0.95 | faiss-hnswsq | sq8_m32e500 | 8-bit SQ + refine | 7103 | {'ef': 48, 'k_factor': 1} | 0.9656 | 2.24 [2.21, 2.27] | 1.87 [1.84, 1.90] |
| 0.95 | faiss-hnswsq | sq4_m32e500 | 4-bit SQ + refine | 7326 | {'ef': 48, 'k_factor': 4} | 0.9607 | 2.17 [2.10, 2.20] | 2.31 [2.28, 2.33] |
| 0.95 | usearch | m32e500 | float32 | 3641 | {'ef': 32} | 0.9519 | 4.36 [4.28, 4.41] | 4.74 [4.68, 4.78] |
| 0.95 | usearch | m32e500_i8 | int8 | not reached (best recall 0.915) | | | | |
| 0.95 | usearch | m32e500_f16 | float16 | 4493 | {'ef': 32} | 0.9517 | 3.54 [3.47, 3.60] | 3.83 [3.78, 3.89] |
| 0.95 | WARM | vertex-d32 | 8-bit codes + exact rerank | 15890 | {'ef': 32, 'rerank': 64} | 0.9591 | 1 | 1 |
| 0.99 | faiss-hnsw | m32e500 | float32 | 3064 | {'ef': 96} | 0.9942 | 2.37 [2.33, 2.41] | 2.16 [2.13, 2.19] |
| 0.99 | faiss-hnswsq | sq8_m32e500 | 8-bit SQ + refine | 3940 | {'ef': 96, 'k_factor': 2} | 0.9938 | 1.85 [1.80, 1.87] | 1.71 [1.68, 1.74] |
| 0.99 | faiss-hnswsq | sq4_m32e500 | 4-bit SQ + refine | 3393 | {'ef': 128, 'k_factor': 4} | 0.9931 | 2.14 [2.08, 2.16] | 2.01 [1.98, 2.04] |
| 0.99 | usearch | m32e500 | float32 | 1592 | {'ef': 96} | 0.9942 | 4.57 [4.47, 4.62] | 4.17 [4.10, 4.23] |
| 0.99 | usearch | m32e500_i8 | int8 | not reached (best recall 0.915) | | | | |
| 0.99 | usearch | m32e500_f16 | float16 | 2006 | {'ef': 96} | 0.9942 | 3.63 [3.56, 3.69] | 3.34 [3.31, 3.37] |
| 0.99 | WARM | vertex-d32 | 8-bit codes + exact rerank | 7274 | {'ef': 96, 'rerank': 32} | 0.9926 | 1 | 1 |

### linux-x86_64-gcp-n2-icelake-8c-16g, sift-128-euclidean n=1000000, 8 rounds, commit dbe04ad6

| recall | comparator | build | traversal | their QPS | their params | recall (report half) | WARM / theirs, grid | WARM / theirs, interpolated |
|---|---|---|---|---|---|---|---|---|
| 0.9 | faiss-hnsw | m32e500 | float32 | 8297 | {'ef': 24} | 0.9232 | 3.25 [3.20, 3.31] | 2.90 [2.87, 2.96] |
| 0.9 | faiss-hnswsq | sq8_m32e500 | 8-bit SQ + refine | 10588 | {'ef': 24, 'k_factor': 1} | 0.9153 | 2.55 [2.50, 2.56] | 2.41 [2.38, 2.43] |
| 0.9 | faiss-hnswsq | sq4_m32e500 | 4-bit SQ + refine | 8636 | {'ef': 32, 'k_factor': 2} | 0.9159 | 3.12 [3.09, 3.13] | 2.99 [2.96, 3.03] |
| 0.9 | usearch | m32e500 | float32 | 3828 | {'ef': 24} | 0.9257 | 7.05 [6.89, 7.12] | 6.26 [6.17, 6.33] |
| 0.9 | usearch | m32e500_i8 | int8 | 4524 | {'ef': 48} | 0.9034 | 5.96 [5.86, 6.03] | 5.72 [5.66, 5.81] |
| 0.9 | usearch | m32e500_f16 | float16 | 5871 | {'ef': 24} | 0.9251 | 4.59 [4.49, 4.71] | 4.08 [3.99, 4.20] |
| 0.9 | WARM | vertex-d32 | 8-bit codes + exact rerank | 26976 | {'ef': 16, 'rerank': 64} | 0.9028 | 1 | 1 |
| 0.95 | faiss-hnsw | m32e500 | float32 | 6707 | {'ef': 32} | 0.9514 | 2.45 [2.40, 2.48] | 2.69 [2.66, 2.73] |
| 0.95 | faiss-hnswsq | sq8_m32e500 | 8-bit SQ + refine | 6245 | {'ef': 48, 'k_factor': 1} | 0.9656 | 2.63 [2.58, 2.67] | 2.22 [2.19, 2.23] |
| 0.95 | faiss-hnswsq | sq4_m32e500 | 4-bit SQ + refine | 6156 | {'ef': 48, 'k_factor': 4} | 0.9607 | 2.67 [2.61, 2.70] | 2.87 [2.84, 2.92] |
| 0.95 | usearch | m32e500 | float32 | 3150 | {'ef': 32} | 0.9519 | 5.21 [5.12, 5.30] | 5.68 [5.63, 5.74] |
| 0.95 | usearch | m32e500_i8 | int8 | not reached (best recall 0.915) | | | | |
| 0.95 | usearch | m32e500_f16 | float16 | 4804 | {'ef': 32} | 0.9516 | 3.42 [3.31, 3.54] | 3.73 [3.64, 3.86] |
| 0.95 | WARM | vertex-d32 | 8-bit codes + exact rerank | 16414 | {'ef': 32, 'rerank': 64} | 0.9591 | 1 | 1 |
| 0.99 | faiss-hnsw | m32e500 | float32 | 2679 | {'ef': 96} | 0.9942 | 2.67 [2.63, 2.72] | 2.45 [2.42, 2.50] |
| 0.99 | faiss-hnswsq | sq8_m32e500 | 8-bit SQ + refine | 3494 | {'ef': 96, 'k_factor': 2} | 0.9938 | 2.05 [2.03, 2.08] | 1.94 [1.92, 1.95] |
| 0.99 | faiss-hnswsq | sq4_m32e500 | 4-bit SQ + refine | 2819 | {'ef': 128, 'k_factor': 4} | 0.9931 | 2.54 [2.50, 2.60] | 2.40 [2.36, 2.43] |
| 0.99 | usearch | m32e500 | float32 | 1365 | {'ef': 96} | 0.9941 | 5.25 [5.17, 5.35] | 4.83 [4.76, 4.94] |
| 0.99 | usearch | m32e500_i8 | int8 | not reached (best recall 0.915) | | | | |
| 0.99 | usearch | m32e500_f16 | float16 | 2039 | {'ef': 96} | 0.9942 | 3.51 [3.45, 3.62] | 3.25 [3.20, 3.32] |
| 0.99 | WARM | vertex-d32 | 8-bit codes + exact rerank | 7163 | {'ef': 96, 'rerank': 32} | 0.9926 | 1 | 1 |
