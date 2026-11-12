# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Tests for writerslogic_warm.Warm: in-process document search, memory and vectors.

Run against an installed build of the extension:
    maturin develop --release && pytest tests/python
"""
import numpy as np

import writerslogic_warm as hm


def test_document_search_memory_and_vectors(tmp_path):
    store = hm.Warm(1024, str(tmp_path / "store"))
    assert store.memorize_document({"id": "harbor", "text": "The harbor opens at six in the morning."}) >= 1
    store.memorize_document({"id": "market", "text": "The market sells fresh bread and cheese on Sundays."})
    hits = store.search_documents("when does the harbor open", {"k": 2})
    assert hits[0]["documentId"] == "harbor"

    remembered = store.memory_call("remember", {"text": "Ada prefers tea", "subject": "ada"})
    assert remembered["remembered"][0]["created"]
    recalled = store.recall("what does Ada prefer")
    assert recalled["results"][0]["text"] == "Ada prefers tea"

    rng = np.random.default_rng(3)
    rows = rng.normal(size=(50, 16)).astype(np.float32)
    for i, row in enumerate(rows):
        store.memorize_vector(f"v{i}", row.tolist())
    best = store.query_vector(rows[7].tolist(), 3)
    assert best[0][0] == "v7" and best[0][1] > 0.999

    del store
    reopened = hm.Warm(1024, str(tmp_path / "store"))
    assert reopened.search_documents("harbor", {"k": 1})[0]["documentId"] == "harbor"
    assert reopened.query_vector(rows[7].tolist(), 1)[0][0] == "v7"
