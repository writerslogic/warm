Intervals: 95 percent family-wise (each of 12 comparisons per family at 0.9958). One machine; not official results.

### fiqa (beir:fiqa, 500 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.4880 | 0.7924 | 729 | 33.0 | 1345 | 1871 | 28904 | 27087 | 1.2 | 25 | 180 |
| lucene-hnsw | 0.4552 | 0.7907 | 387 | 20.6 | 1435 | 7279 | 46305 | 44157 | 11.6 | 26 | 262 |
| lucene-hnsw+tuned | 0.4740 | 0.7907 | 485 | 32.0 | 1217 | 5768 | 29304 | 27393 | 11.6 | 26 | 262 |
| lucene-exact | 0.4568 | 0.7927 | 146 | 28.3 | 5643 | 13305 | 33628 | 27642 | 11.6 | 26 | 261 |
| lucene-exact+tuned | 0.4742 | 0.7927 | 188 | 29.2 | 5216 | 7875 | 33155 | 27144 | 11.6 | 26 | 261 |
| sqlite-vec | 0.4597 | 0.7919 | 21 | 12.6 | 46555 | 82510 | 79566 | 27258 | 1.3 | 33 | 67 |
| sqlite-vec+tuned | 0.4699 | 0.7919 | 21 | 12.6 | 46609 | 82246 | 79699 | 27175 | 1.3 | 33 | 67 |
| lancedb | 0.4610 | 0.7994 | 36 | 17.5 | 27457 | 29734 | 55874 | 27234 | 1.8 | 32 | 482 |
| lancedb+tuned | 0.4818 | 0.7994 | 37 | 17.6 | 27130 | 28775 | 55536 | 27160 | 1.8 | 32 | 482 |
| tantivy-usearch | 0.4569 | 0.7897 | 557 | 30.1 | 1793 | 2212 | 29595 | 27181 | 1.7 | 29 | 83 |
| tantivy-usearch+tuned | 0.4724 | 0.7897 | 552 | 32.4 | 1803 | 2219 | 29640 | 27169 | 1.7 | 29 | 83 |

nDCG@10: WARM 0.4880 vs best comparator lancedb+tuned 0.4818, paired difference +0.0062 [-0.0082, +0.0197] **TIE**

recall@100: WARM 0.7924 vs best comparator lancedb 0.7994, paired difference -0.0070 [-0.0200, +0.0066] **TIE**

QPS excluding embedding: WARM 728.7 vs tantivy-usearch 557.1, ratio 1.31 [1.28, 1.35] **WIN**

QPS including embedding: WARM 33.0 vs tantivy-usearch+tuned 32.4, ratio 1.02 [1.01, 1.02] **WIN**

foreign CPU max 0.0224 cores; including-pass id mismatches 0

### scifact (beir:scifact, 809 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.7686 | 0.9689 | 1416 | 26.7 | 699 | 932 | 37990 | 36975 | 0.7 | 9 | 148 |
| lucene-hnsw | 0.7490 | 0.9726 | 481 | 18.4 | 1032 | 6547 | 49908 | 48159 | 3.8 | 10 | 240 |
| lucene-hnsw+tuned | 0.7599 | 0.9726 | 667 | 24.6 | 853 | 5724 | 39044 | 37748 | 3.8 | 10 | 240 |
| lucene-exact | 0.7492 | 0.9726 | 288 | 20.7 | 3383 | 8154 | 46274 | 43570 | 3.8 | 10 | 245 |
| lucene-exact+tuned | 0.7599 | 0.9726 | 332 | 25.8 | 3297 | 7029 | 39213 | 36763 | 3.8 | 10 | 245 |
| sqlite-vec | 0.7490 | 0.9751 | 55 | 17.8 | 17984 | 30104 | 55105 | 36972 | 0.7 | 15 | 61 |
| sqlite-vec+tuned | 0.7575 | 0.9751 | 55 | 17.8 | 18076 | 30247 | 54937 | 36824 | 0.7 | 15 | 61 |
| lancedb | 0.7498 | 0.9701 | 80 | 19.8 | 12528 | 14407 | 50899 | 36799 | 1.8 | 14 | 302 |
| lancedb+tuned | 0.7592 | 0.9701 | 80 | 19.8 | 12461 | 14002 | 51040 | 36754 | 1.8 | 14 | 302 |
| tantivy-usearch | 0.7381 | 0.9683 | 755 | 24.2 | 1318 | 1577 | 39558 | 37643 | 0.8 | 11 | 65 |
| tantivy-usearch+tuned | 0.7530 | 0.9683 | 768 | 26.1 | 1300 | 1527 | 38869 | 36794 | 0.8 | 11 | 65 |

nDCG@10: WARM 0.7686 vs best comparator lucene-hnsw+tuned 0.7599, paired difference +0.0087 [-0.0023, +0.0204] **TIE**

recall@100: WARM 0.9689 vs best comparator sqlite-vec 0.9751, paired difference -0.0062 [-0.0161, +0.0000] **TIE**

QPS excluding embedding: WARM 1416.0 vs tantivy-usearch+tuned 768.3, ratio 1.84 [1.78, 1.91] **WIN**

QPS including embedding: WARM 26.7 vs tantivy-usearch+tuned 26.1, ratio 1.02 [1.02, 1.03] **WIN**

foreign CPU max 0.0285 cores; including-pass id mismatches 0

### nfcorpus (beir:nfcorpus, 324 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.3407 | 0.3082 | 2423 | 46.2 | 397 | 602 | 21965 | 21361 | 0.6 | 7 | 123 |
| lucene-hnsw | 0.3364 | 0.3077 | 670 | 29.9 | 724 | 6141 | 30861 | 29472 | 2.6 | 7 | 228 |
| lucene-hnsw+tuned | 0.3342 | 0.3077 | 854 | 37.4 | 600 | 5834 | 24316 | 22973 | 2.6 | 7 | 228 |
| lucene-exact | 0.3378 | 0.3069 | 441 | 33.8 | 1358 | 6282 | 28315 | 26042 | 2.6 | 7 | 229 |
| lucene-exact+tuned | 0.3345 | 0.3069 | 602 | 43.0 | 1227 | 4663 | 23800 | 22128 | 2.6 | 7 | 229 |
| sqlite-vec | 0.3405 | 0.3079 | 191 | 32.8 | 4010 | 13493 | 31031 | 21325 | 0.4 | 10 | 61 |
| sqlite-vec+tuned | 0.3369 | 0.3079 | 192 | 32.9 | 3986 | 13381 | 30629 | 21219 | 0.4 | 10 | 61 |
| lancedb | 0.3363 | 0.3068 | 94 | 30.3 | 10589 | 12259 | 33425 | 21462 | 1.6 | 10 | 258 |
| lancedb+tuned | 0.3372 | 0.3068 | 95 | 30.3 | 10534 | 12209 | 33488 | 21248 | 1.6 | 10 | 258 |
| tantivy-usearch | 0.3349 | 0.3071 | 1182 | 38.5 | 848 | 1225 | 23443 | 21745 | 0.6 | 8 | 61 |
| tantivy-usearch+tuned | 0.3348 | 0.3071 | 1155 | 44.3 | 864 | 1229 | 22945 | 21280 | 0.6 | 8 | 61 |

nDCG@10: WARM 0.3407 vs best comparator sqlite-vec 0.3405, paired difference +0.0003 [-0.0100, +0.0113] **TIE**

recall@100: WARM 0.3082 vs best comparator sqlite-vec 0.3079, paired difference +0.0003 [-0.0034, +0.0037] **TIE**

QPS excluding embedding: WARM 2422.5 vs tantivy-usearch 1181.8, ratio 2.05 [1.99, 2.16] **WIN**

QPS including embedding: WARM 46.2 vs tantivy-usearch+tuned 44.3, ratio 1.04 [1.04, 1.05] **WIN**

foreign CPU max 0.0307 cores; including-pass id mismatches 0

### memory (memory, 100 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.4288 | 0.8615 | 1186 | 34.1 | 814 | 1304 | 27988 | 26962 | 1.9 | 11 | 141 |
| lucene-hnsw | 0.4115 | 0.8610 | 410 | 21.5 | 1287 | 6453 | 44564 | 42795 | 5.9 | 17 | 233 |
| lucene-hnsw+tuned | 0.4344 | 0.8610 | 464 | 29.2 | 1054 | 6705 | 32226 | 30390 | 5.9 | 17 | 233 |
| lucene-exact | 0.4142 | 0.8610 | 245 | 22.3 | 4884 | 7693 | 43927 | 38877 | 5.9 | 17 | 233 |
| lucene-exact+tuned | 0.4332 | 0.8610 | 380 | 32.3 | 1879 | 7340 | 29340 | 27243 | 5.9 | 17 | 233 |
| sqlite-vec | 0.4109 | 0.8595 | 24 | 15.9 | 36173 | 127805 | 60711 | 27061 | 2.6 | 27 | 61 |
| sqlite-vec+tuned | 0.4234 | 0.8595 | 24 | 15.9 | 36254 | 128113 | 60846 | 27071 | 2.6 | 27 | 61 |
| lancedb | 0.4160 | 0.8585 | 81 | 24.0 | 12198 | 14159 | 40108 | 27126 | 4.8 | 39 | 275 |
| lancedb+tuned | 0.4163 | 0.8585 | 83 | 23.9 | 12067 | 13448 | 40617 | 27122 | 4.8 | 39 | 275 |
| tantivy-usearch | 0.4156 | 0.8610 | 742 | 30.1 | 1319 | 1788 | 28823 | 26973 | 3.3 | 21 | 65 |
| tantivy-usearch+tuned | 0.4302 | 0.8610 | 737 | 33.1 | 1328 | 1800 | 28917 | 27060 | 3.3 | 21 | 65 |

nDCG@10: WARM 0.4288 vs best comparator lucene-hnsw+tuned 0.4344, paired difference -0.0056 [-0.0408, +0.0300] **TIE**

recall@100: WARM 0.8615 vs best comparator lucene-hnsw 0.8610, paired difference +0.0005 [-0.0140, +0.0200] **TIE**

QPS excluding embedding: WARM 1186.0 vs tantivy-usearch 741.9, ratio 1.60 [1.51, 1.65] **WIN**

QPS including embedding: WARM 34.1 vs tantivy-usearch+tuned 33.1, ratio 1.03 [1.03, 1.04] **WIN**

foreign CPU max 0.0189 cores; including-pass id mismatches 0

### arguana (beir:arguana, 1406 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.4094 | 0.9886 | 392 | 2.8 | 2467 | 4162 | 299982 | 301273 | 1.0 | 15 | 217 |
| lucene-hnsw | 0.3822 | 0.9851 | 229 | 2.7 | 4175 | 10902 | 305393 | 302033 | 5.0 | 15 | 269 |
| lucene-hnsw+tuned | 0.3946 | 0.9851 | 399 | 2.8 | 2281 | 6561 | 300844 | 298703 | 5.0 | 15 | 269 |
| lucene-exact | 0.3821 | 0.9851 | 187 | 2.8 | 4661 | 11536 | 304427 | 299750 | 5.0 | 15 | 268 |
| lucene-exact+tuned | 0.3944 | 0.9851 | 227 | 2.8 | 4214 | 7647 | 302427 | 298004 | 5.0 | 15 | 268 |
| sqlite-vec | 0.3938 | 0.9844 | 5 | 1.9 | 170006 | 411548 | 443571 | 297478 | 0.9 | 22 | 75 |
| sqlite-vec+tuned | 0.4045 | 0.9844 | 5 | 1.9 | 170301 | 411334 | 443178 | 298694 | 0.9 | 22 | 75 |
| lancedb | 0.3979 | 0.9858 | 43 | 2.6 | 22679 | 30576 | 319763 | 298145 | 9.2 | 20 | 399 |
| lancedb+tuned | 0.4055 | 0.9858 | 44 | 2.6 | 22244 | 29682 | 319736 | 297498 | 9.2 | 20 | 399 |
| tantivy-usearch | 0.3811 | 0.9815 | 322 | 2.7 | 2973 | 5712 | 300242 | 297044 | 1.4 | 18 | 78 |
| tantivy-usearch+tuned | 0.3937 | 0.9815 | 324 | 2.8 | 2964 | 5571 | 302666 | 299376 | 1.4 | 18 | 78 |

nDCG@10: WARM 0.4094 vs best comparator lancedb+tuned 0.4055, paired difference +0.0039 [-0.0042, +0.0128] **TIE**

recall@100: WARM 0.9886 vs best comparator lancedb 0.9858, paired difference +0.0028 [-0.0028, +0.0092] **TIE**

QPS excluding embedding: WARM 391.8 vs lucene-hnsw+tuned 398.7, ratio 0.98 [0.96, 0.99] **LOSS**

QPS including embedding: WARM 2.8 vs lucene-hnsw+tuned 2.8, ratio 0.99 [0.99, 1.00] **LOSS**

foreign CPU max 0.0081 cores; including-pass id mismatches 2000

### scidocs (beir:scidocs, 1000 queries)

| row | nDCG@10 | recall@100 | QPS excl | QPS incl | p50 excl us | p99 excl us | p50 incl us | embed p50 us | build s | index MiB | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| warm | 0.2956 | 0.5767 | 1035 | 33.9 | 952 | 1257 | 28397 | 27213 | 1.0 | 18 | 180 |
| lucene-hnsw | 0.2874 | 0.5798 | 425 | 21.1 | 1197 | 7263 | 46342 | 44593 | 7.6 | 19 | 254 |
| lucene-hnsw+tuned | 0.2835 | 0.5798 | 618 | 31.0 | 988 | 6203 | 29700 | 28255 | 7.6 | 19 | 254 |
| lucene-exact | 0.2876 | 0.5794 | 209 | 28.9 | 4571 | 10458 | 33115 | 28388 | 7.6 | 19 | 256 |
| lucene-exact+tuned | 0.2834 | 0.5794 | 280 | 31.0 | 3354 | 6230 | 31650 | 27068 | 7.6 | 19 | 256 |
| sqlite-vec | 0.2885 | 0.5781 | 35 | 17.4 | 29174 | 42356 | 56760 | 27334 | 1.0 | 25 | 61 |
| sqlite-vec+tuned | 0.2832 | 0.5781 | 35 | 17.4 | 29165 | 42666 | 56735 | 27206 | 1.0 | 25 | 61 |
| lancedb | 0.2888 | 0.5801 | 48 | 20.0 | 20930 | 22735 | 49247 | 27135 | 2.0 | 24 | 449 |
| lancedb+tuned | 0.2921 | 0.5801 | 48 | 20.0 | 20638 | 22173 | 49000 | 27084 | 2.0 | 24 | 449 |
| tantivy-usearch | 0.2863 | 0.5780 | 643 | 30.3 | 1562 | 1864 | 30335 | 28096 | 1.4 | 21 | 77 |
| tantivy-usearch+tuned | 0.2850 | 0.5780 | 636 | 32.9 | 1576 | 1881 | 29251 | 27083 | 1.4 | 21 | 77 |

nDCG@10: WARM 0.2956 vs best comparator lancedb+tuned 0.2921, paired difference +0.0034 [-0.0013, +0.0084] **TIE**

recall@100: WARM 0.5767 vs best comparator lancedb 0.5801, paired difference -0.0034 [-0.0085, +0.0015] **TIE**

QPS excluding embedding: WARM 1035.1 vs tantivy-usearch 642.9, ratio 1.61 [1.57, 1.70] **WIN**

QPS including embedding: WARM 33.9 vs tantivy-usearch+tuned 32.9, ratio 1.03 [1.03, 1.03] **WIN**

foreign CPU max 0.0350 cores; including-pass id mismatches 0

