# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""Learned-sparse ranking: you supply the (term id, weight) vectors; WARM stores and ranks them.

Needs the in-process `writerslogic_warm.Warm` class: `maturin develop --release` from a checkout.
Run: python examples/quickstart/python/sparse.py
"""

import os
from tempfile import TemporaryDirectory

scratch = TemporaryDirectory()
os.chdir(scratch.name)

# --8<-- [start:sparse]
from writerslogic_warm import Warm

w = Warm(path="store")
# One (term ids, weights) pair per document, or None. Weights are finite and >= 0.
w.memorize_documents_sparse(
    [{"id": "d1", "text": "harbor opens at six"}, {"id": "d2", "text": "market closes at noon"}],
    [([101, 2045, 9120], [1.2, 0.4, 2.0]), None],
)
hits = w.search_documents_ranked(
    "harbor hours", {"k": 10}, ranking="fused", sparse_query=([2045, 9120], [0.9, 1.4])
)
# --8<-- [end:sparse]
print([hit["documentId"] for hit in hits])
