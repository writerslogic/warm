Intervals: 95 percent family-wise (each of 10 comparisons per family at 0.9950). One machine; not official results.

### fiqa (beir:fiqa, 500 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.4880 | 0.7924 | 62 | 21.7 | 15990 | 17229 | 44697 | 27122 | 1.3 | 25 | 169 |
| lucene-hnsw | 0.4552 | 0.7907 | 310 | 20.1 | 2875 | 7735 | 46064 | 44034 | 14.1 | 26 | 259 |
| lucene-hnsw+tuned | 0.4740 | 0.7907 | 408 | 31.3 | 1476 | 6056 | 29295 | 27332 | 14.1 | 26 | 259 |
| lucene-exact | 0.4568 | 0.7927 | 117 | 27.4 | 8963 | 15483 | 35615 | 27451 | 14.1 | 26 | 262 |
| lucene-exact+tuned | 0.4742 | 0.7927 | 164 | 28.3 | 6014 | 9472 | 33353 | 27110 | 14.1 | 26 | 262 |
| sqlite-vec | 0.4597 | 0.7919 | 20 | 12.3 | 47867 | 83594 | 81226 | 27128 | 1.4 | 33 | 72 |
| sqlite-vec+tuned | 0.4699 | 0.7919 | 20 | 12.4 | 47771 | 83346 | 80861 | 27173 | 1.4 | 33 | 72 |
| lancedb | 0.4610 | 0.7994 | 33 | 16.6 | 29811 | 32528 | 59059 | 27371 | 1.9 | 32 | 538 |
| lancedb+tuned | 0.4818 | 0.7994 | 34 | 16.8 | 29330 | 31632 | 58109 | 27215 | 1.9 | 32 | 538 |
| tantivy-usearch | 0.4591 | 0.7923 | 482 | 29.8 | 2071 | 2524 | 29922 | 27240 | 1.8 | 29 | 90 |
| tantivy-usearch+tuned | 0.4727 | 0.7923 | 483 | 31.7 | 2062 | 2490 | 29673 | 27020 | 1.8 | 29 | 90 |

nDCG@10: WARM 0.4880 vs best comparator lancedb+tuned 0.4818, paired difference +0.0062 [-0.0077, +0.0197] **TIE**

recall@100: WARM 0.7924 vs best comparator lancedb 0.7994, paired difference -0.0070 [-0.0199, +0.0066] **TIE**

QPS excluding embedding: WARM 62.5 vs tantivy-usearch+tuned 483.1, ratio 0.13 [0.12, 0.15] **LOSS**

QPS including embedding: WARM 21.7 vs tantivy-usearch+tuned 31.7, ratio 0.69 [0.67, 0.70] **LOSS**

foreign CPU max 0.0226 cores; including-pass id mismatches 0

### scifact (beir:scifact, 809 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.7686 | 0.9689 | 213 | 22.8 | 4676 | 5364 | 43795 | 37404 | 0.8 | 9 | 145 |
| lucene-hnsw | 0.7490 | 0.9726 | 393 | 17.9 | 1336 | 7402 | 53084 | 51078 | 4.6 | 10 | 241 |
| lucene-hnsw+tuned | 0.7599 | 0.9726 | 635 | 23.4 | 1007 | 5480 | 40266 | 38797 | 4.6 | 10 | 241 |
| lucene-exact | 0.7492 | 0.9726 | 262 | 20.7 | 3541 | 8863 | 44907 | 42101 | 4.6 | 10 | 242 |
| lucene-exact+tuned | 0.7599 | 0.9726 | 364 | 24.7 | 2885 | 6740 | 40423 | 37760 | 4.6 | 10 | 242 |
| sqlite-vec | 0.7490 | 0.9751 | 53 | 17.2 | 18648 | 30771 | 57816 | 37387 | 0.7 | 15 | 62 |
| sqlite-vec+tuned | 0.7575 | 0.9751 | 53 | 17.2 | 18666 | 30782 | 57641 | 37218 | 0.7 | 15 | 62 |
| lancedb | 0.7498 | 0.9701 | 69 | 18.8 | 14386 | 16123 | 53018 | 37401 | 1.8 | 14 | 356 |
| lancedb+tuned | 0.7592 | 0.9701 | 69 | 18.8 | 14400 | 15893 | 53166 | 37481 | 1.8 | 14 | 356 |
| tantivy-usearch | 0.7475 | 0.9701 | 659 | 23.4 | 1510 | 1794 | 40429 | 38373 | 0.9 | 11 | 72 |
| tantivy-usearch+tuned | 0.7571 | 0.9701 | 656 | 25.1 | 1510 | 1840 | 39572 | 37358 | 0.9 | 11 | 72 |

nDCG@10: WARM 0.7686 vs best comparator lucene-hnsw+tuned 0.7599, paired difference +0.0087 [-0.0023, +0.0204] **TIE**

recall@100: WARM 0.9689 vs best comparator sqlite-vec 0.9751, paired difference -0.0062 [-0.0161, +0.0000] **TIE**

QPS excluding embedding: WARM 212.9 vs tantivy-usearch 659.4, ratio 0.32 [0.30, 0.34] **LOSS**

QPS including embedding: WARM 22.8 vs tantivy-usearch+tuned 25.1, ratio 0.91 [0.90, 0.91] **LOSS**

foreign CPU max 0.0265 cores; including-pass id mismatches 0

### nfcorpus (beir:nfcorpus, 324 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.3407 | 0.3082 | 339 | 39.8 | 2941 | 3445 | 24419 | 20276 | 0.8 | 7 | 123 |
| lucene-hnsw | 0.3364 | 0.3077 | 552 | 29.3 | 960 | 6399 | 31452 | 29968 | 2.9 | 7 | 231 |
| lucene-hnsw+tuned | 0.3342 | 0.3077 | 774 | 39.1 | 748 | 5628 | 23036 | 21705 | 2.9 | 7 | 231 |
| lucene-exact | 0.3378 | 0.3069 | 397 | 35.3 | 1646 | 6358 | 26902 | 24532 | 2.9 | 7 | 231 |
| lucene-exact+tuned | 0.3345 | 0.3069 | 596 | 43.1 | 1370 | 4311 | 23216 | 21336 | 2.9 | 7 | 231 |
| sqlite-vec | 0.3405 | 0.3079 | 178 | 32.5 | 4391 | 14014 | 31417 | 20487 | 0.4 | 10 | 61 |
| sqlite-vec+tuned | 0.3369 | 0.3079 | 178 | 32.6 | 4367 | 14035 | 31457 | 20029 | 0.4 | 10 | 61 |
| lancedb | 0.3363 | 0.3068 | 81 | 29.1 | 12381 | 14107 | 34373 | 20585 | 1.7 | 10 | 307 |
| lancedb+tuned | 0.3372 | 0.3068 | 81 | 29.1 | 12299 | 13595 | 34008 | 20196 | 1.7 | 10 | 307 |
| tantivy-usearch | 0.3345 | 0.3077 | 1016 | 38.6 | 965 | 1428 | 23075 | 21290 | 0.6 | 8 | 68 |
| tantivy-usearch+tuned | 0.3349 | 0.3077 | 1006 | 44.2 | 969 | 1425 | 22027 | 20195 | 0.6 | 8 | 68 |

nDCG@10: WARM 0.3407 vs best comparator sqlite-vec 0.3405, paired difference +0.0003 [-0.0100, +0.0113] **TIE**

recall@100: WARM 0.3082 vs best comparator sqlite-vec 0.3079, paired difference +0.0003 [-0.0033, +0.0037] **TIE**

QPS excluding embedding: WARM 338.9 vs tantivy-usearch 1016.0, ratio 0.33 [0.31, 0.35] **LOSS**

QPS including embedding: WARM 39.8 vs tantivy-usearch+tuned 44.2, ratio 0.90 [0.89, 0.91] **LOSS**

foreign CPU max 0.0276 cores; including-pass id mismatches 0

### memory (memory, 100 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.4288 | 0.8615 | 186 | 28.2 | 5388 | 6112 | 33709 | 27008 | 2.0 | 11 | 142 |
| lucene-hnsw | 0.4115 | 0.8610 | 340 | 20.9 | 1866 | 6557 | 45888 | 43636 | 6.5 | 17 | 234 |
| lucene-hnsw+tuned | 0.4344 | 0.8610 | 396 | 30.1 | 1316 | 6976 | 31241 | 29192 | 6.5 | 17 | 234 |
| lucene-exact | 0.4142 | 0.8610 | 208 | 21.8 | 5401 | 8708 | 42728 | 38220 | 6.5 | 17 | 233 |
| lucene-exact+tuned | 0.4332 | 0.8610 | 328 | 31.5 | 2228 | 8420 | 29951 | 27443 | 6.5 | 17 | 233 |
| sqlite-vec | 0.4109 | 0.8595 | 23 | 15.6 | 36989 | 129072 | 62969 | 27013 | 2.8 | 27 | 64 |
| sqlite-vec+tuned | 0.4234 | 0.8595 | 23 | 15.6 | 36896 | 129193 | 63125 | 27014 | 2.8 | 27 | 64 |
| lancedb | 0.4160 | 0.8585 | 70 | 22.8 | 14224 | 16268 | 41858 | 27061 | 4.9 | 39 | 326 |
| lancedb+tuned | 0.4163 | 0.8585 | 71 | 22.8 | 13987 | 15489 | 41997 | 27037 | 4.9 | 39 | 326 |
| tantivy-usearch | 0.4014 | 0.8610 | 630 | 29.7 | 1566 | 2082 | 28928 | 26935 | 3.4 | 21 | 72 |
| tantivy-usearch+tuned | 0.4224 | 0.8610 | 636 | 32.5 | 1545 | 2072 | 28935 | 26968 | 3.4 | 21 | 72 |

nDCG@10: WARM 0.4288 vs best comparator lucene-hnsw+tuned 0.4344, paired difference -0.0056 [-0.0407, +0.0300] **TIE**

recall@100: WARM 0.8615 vs best comparator lucene-hnsw 0.8610, paired difference +0.0005 [-0.0135, +0.0200] **TIE**

QPS excluding embedding: WARM 185.6 vs tantivy-usearch+tuned 636.2, ratio 0.29 [0.28, 0.31] **LOSS**

QPS including embedding: WARM 28.2 vs tantivy-usearch+tuned 32.5, ratio 0.87 [0.86, 0.87] **LOSS**

foreign CPU max 0.0224 cores; including-pass id mismatches 0

### scidocs (beir:scidocs, 1000 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.2956 | 0.5767 | 82 | 23.7 | 12139 | 13446 | 41098 | 27206 | 1.1 | 18 | 172 |
| lucene-hnsw | 0.2874 | 0.5798 | 370 | 20.4 | 1496 | 7813 | 47753 | 45973 | 9.2 | 19 | 256 |
| lucene-hnsw+tuned | 0.2835 | 0.5798 | 501 | 31.7 | 1203 | 6203 | 30549 | 29164 | 9.2 | 19 | 256 |
| lucene-exact | 0.2876 | 0.5794 | 164 | 28.1 | 5519 | 11534 | 34253 | 29356 | 9.2 | 19 | 257 |
| lucene-exact+tuned | 0.2834 | 0.5794 | 246 | 30.5 | 3945 | 6264 | 32042 | 27247 | 9.2 | 19 | 257 |
| sqlite-vec | 0.2885 | 0.5781 | 34 | 17.1 | 29959 | 43377 | 57686 | 28193 | 1.1 | 25 | 63 |
| sqlite-vec+tuned | 0.2832 | 0.5781 | 34 | 17.2 | 29972 | 43403 | 57756 | 27129 | 1.1 | 25 | 63 |
| lancedb | 0.2888 | 0.5801 | 44 | 19.3 | 22629 | 24748 | 52103 | 28313 | 2.1 | 24 | 461 |
| lancedb+tuned | 0.2921 | 0.5801 | 45 | 19.3 | 22341 | 24056 | 51367 | 27300 | 2.1 | 24 | 461 |
| tantivy-usearch | 0.2856 | 0.5786 | 561 | 30.1 | 1788 | 2147 | 31303 | 29013 | 1.5 | 21 | 84 |
| tantivy-usearch+tuned | 0.2841 | 0.5786 | 552 | 32.6 | 1816 | 2239 | 29279 | 26989 | 1.5 | 21 | 84 |

nDCG@10: WARM 0.2956 vs best comparator lancedb+tuned 0.2921, paired difference +0.0034 [-0.0013, +0.0084] **TIE**

recall@100: WARM 0.5767 vs best comparator lancedb 0.5801, paired difference -0.0034 [-0.0083, +0.0015] **TIE**

QPS excluding embedding: WARM 82.2 vs tantivy-usearch 560.5, ratio 0.15 [0.14, 0.16] **LOSS**

QPS including embedding: WARM 23.7 vs tantivy-usearch+tuned 32.6, ratio 0.73 [0.72, 0.73] **LOSS**

foreign CPU max 0.0346 cores; including-pass id mismatches 0

