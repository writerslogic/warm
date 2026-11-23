---
description: Rank documents with learned-sparse vectors you encode yourself, alone or fused with BM25, with the measured gain, costs and limits.
---

# Learned-sparse ranking (opt-in)

A learned sparse encoder (SPLADE style) maps a text to weights over a vocabulary of up to 65,536
terms. WARM runs no encoder here: you encode documents and queries yourself and pass the vectors in.
Nothing changes for a store that never receives one; no sparse file is written and lexical search is
byte for byte what it was.

The Python `Warm` class used here needs `warm-python` 0.7.1 or later.

```python
--8<-- "examples/quickstart/python/sparse.py:sparse"
```

`ranking` is `"lexical"` (default, ignores the sparse query), `"sparse"` (inner product with the
stored vectors, text unused) or `"fused"` (reciprocal rank fusion, k = 60, of the lexical and sparse
document rankings over `candidateLimit` candidates, default 100). The Rust calls are
`memorize_documents_sparse`, `memorize_document_sparse` and `search_documents_ranked`. A document
ingested without a vector, or replaced without one, is absent from sparse ranking; deleting a
document removes its vector. `filter` and `sourceUri` are rejected in sparse and fused modes;
`documentIds` is honoured.

Measured gain, Apple M4, one thread, with `opensearch-neural-sparse-encoding-v2-distill`
(Apache-2.0) encodings ingested through this API, the benchmark harness's train/dev splits
(nDCG@10 against WARM BM25 on the same corpus):

| Dataset | BM25 | sparse | fused |
|---|---|---|---|
| BEIR corpus of 5,183 docs (train queries) | 0.7027 | 0.7252 | 0.7344 |
| BEIR corpus of 3,633 docs (dev queries) | 0.2916 | 0.3384 | 0.3352 |

A standalone `SparseIndex` run on FiQA (dev queries, 57,600 docs) gave 0.3664 against 0.2654 for
BM25; it was not repeated through the document API. Fusion helped on the 5,183-document corpus and not on the 3,633-document one,
so test both modes on your data.

Costs. Encoding is yours to pay: 21 to 47 ms per document on an Apple GPU for that model, against
0.1 to 0.4 ms per document for BM25 ingest. Storing the vectors adds 0.2 to 0.3 ms per document
(noisy measurement) and about 1.5 KB per document at 250 terms (5,183 documents wrote a 7.9 MB
file); rows are also held in memory at 6 bytes per entry. Search below 100,000 rows scans every
row exactly (863 queries per second on the 5,183-document corpus and 518 on the 3,633-document one, against about 12,600 and
24,800 for BM25 on the same machine). From 100,000 rows a pruned impact index is built from the
stored rows on first search and rebuilt after changes touch a tenth of them; it is approximate
and the recall of its default parameters is not measured through this API.

Limits: term ids below 65,536; at most 4,096 entries per document and 1,024 per query; at most
536,870,912 entries and 67,108,864 documents per store; `k` up to 10,000. Duplicate term ids in a
document, negative or non-finite weights are errors. An encrypted store refuses sparse vectors
because the file is plaintext. The file is `sparse-docs.wsp`
([format](../file-format/ANCILLARY.md#sparse-docswsp-learned-sparse-document-vectors)).
