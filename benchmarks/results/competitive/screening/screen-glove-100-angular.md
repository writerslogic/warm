screening: glove-100-angular n=1183514 queries=10000 (prefix), 1 round

| system | build | QPS@0.90 | QPS@0.95 | QPS@0.99 | max recall | sweep rows | last param | selected |
|---|---|---|---|---|---|---|---|---|
| hnswlib | m16 | 1,337 | 546 | not reached | 0.9766 | 16 | 2048 |  |
| hnswlib | m32 | 1,636 | 851 | 236 | 0.9941 | 16 | 2048 | yes |
| hnswlib | m16e500 | 1,305 | 690 | not reached | 0.9828 | 16 | 2048 |  |
| hnswlib | m32e500 | 1,470 | 781 | 216 | 0.9970 | 16 | 2048 |  |
| hnswlib-cpp | m16 | 1,366 | 545 | not reached | 0.9766 | 16 | 2048 |  |
| hnswlib-cpp | m32 | 1,605 | 866 | 239 | 0.9941 | 16 | 2048 | yes |
| hnswlib-cpp | m16e500 | 1,344 | 697 | not reached | 0.9828 | 16 | 2048 |  |
| hnswlib-cpp | m32e500 | 1,483 | 776 | 214 | 0.9970 | 16 | 2048 |  |
| faiss-hnsw | m16 | 981 | 398 | not reached | 0.9794 | 16 | 2048 |  |
| faiss-hnsw | m32 | 1,220 | 678 | 188 | 0.9942 | 16 | 2048 |  |
| faiss-hnsw | m16e500 | 1,277 | 489 | not reached | 0.9845 | 16 | 2048 |  |
| faiss-hnsw | m32e500 | 1,502 | 829 | 236 | 0.9977 | 16 | 2048 | yes |
| usearch | m16 | 436 | 176 | not reached | 0.9766 | 16 | 2048 |  |
| usearch | m32 | 504 | 270 | 77 | 0.9941 | 16 | 2048 | yes |
| usearch | m16e500 | 418 | 170 | not reached | 0.9817 | 16 | 2048 |  |
| usearch | m32e500 | 464 | 245 | 101 | 0.9969 | 16 | 2048 | yes |
