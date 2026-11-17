# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Tests for writerslogic_warm.Warm: in-process document search, memory and vectors.

Run against an installed build of the extension:
    maturin develop --release && pytest tests/python
"""
import os

import numpy as np
import pytest

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


FACTS = [
    ("car", "the car broke down on the highway", [1.0, 0.0, 0.0]),
    ("auto", "my automobile stopped working", [0.9, 0.1, 0.0]),
    ("order", "order zx81q7 shipped on monday", [0.0, 1.0, 0.0]),
    ("garden", "tomatoes need watering twice a week", [0.0, 0.0, 1.0]),
]


def _ids(result):
    return [r["id"] for r in result["results"]]


def _vector_store(path):
    store = hm.Warm(256, str(path), embedding_dimensions=3)
    for fact_id, text, vector in FACTS:
        store.remember({"id": fact_id, "text": text, "embedding": vector})
    return store


def test_recall_is_hybrid_when_the_caller_supplies_vectors(tmp_path):
    store = _vector_store(tmp_path / "s")
    lexical = store.recall("the vehicle failed", {"k": 2, "embedding": [1, 0, 0], "semanticWeight": 0})
    assert "auto" not in _ids(lexical)
    hybrid = store.recall("the vehicle failed", {"k": 2, "embedding": np.array([1.0, 0.0, 0.0])})
    assert set(_ids(hybrid)) == {"car", "auto"}
    # The exact term is in "order"; the query vector points at "garden". Dense alone misses it.
    dense = store.recall("zx81q7", {"k": 1, "embedding": [0, 0.2, 1], "lexicalWeight": 0})
    assert _ids(dense) == ["garden"]
    fused = store.recall("zx81q7", {"k": 2, "embedding": [0, 0.2, 1], "fusion": "rrf"})
    assert _ids(fused) == ["order", "garden"]
    # A query without an embedding on a store that has vectors ranks lexically.
    assert _ids(store.recall("zx81q7", {"k": 1})) == ["order"]


def test_embedding_errors_are_clear(tmp_path):
    store = hm.Warm(256, str(tmp_path / "s"), embedding_dimensions=3)
    with pytest.raises(ValueError, match="2 values; the store's embedding space has 3 dimensions"):
        store.remember({"id": "a", "text": "some text", "embedding": [1.0, 0.0]})
    with pytest.raises(ValueError, match="finite|JSON compliant"):
        store.remember({"id": "a", "text": "some text", "embedding": [1.0, float("nan"), 0.0]})
    with pytest.raises(ValueError, match="finite|JSON compliant"):
        store.recall("some", {"embedding": np.array([1.0, np.inf, 0.0])})
    with pytest.raises(ValueError, match="all zeros"):
        store.recall("some", {"embedding": [0, 0, 0]})
    with pytest.raises(ValueError, match="4 values"):
        store.recall("some", {"embedding": [1, 0, 0, 0]})
    assert store.memory_call("get", {"id": "a"})["fact"] is None
    plain = hm.Warm(256, str(tmp_path / "plain"))
    with pytest.raises(ValueError, match="embedding space"):
        plain.remember({"id": "a", "text": "some text", "embedding": [1.0, 0.0, 0.0]})
    with pytest.raises(ValueError, match="not both"):
        hm.Warm(256, str(tmp_path / "both"), embedder=object(), embedding_dimensions=3)


def test_stores_without_vectors_rank_as_before(tmp_path):
    plain = hm.Warm(256, str(tmp_path / "plain"))
    spaced = hm.Warm(256, str(tmp_path / "spaced"), embedding_dimensions=3)
    for store in (plain, spaced):
        for fact_id, text, _ in FACTS:
            store.remember({"id": fact_id, "text": text})
    for query in ("car highway", "zx81q7 monday", "tomatoes", "unseen words"):
        a, b = plain.recall(query, {"k": 4}), spaced.recall(query, {"k": 4})
        assert [(r["id"], r["score"]) for r in a["results"]] == [(r["id"], r["score"]) for r in b["results"]]


@pytest.mark.skipif(not os.environ.get("WARM_TEST_EMBEDDER"), reason="needs the local embedding model")
def test_embedder_makes_recall_hybrid_in_the_engine(tmp_path):
    embedder = hm.Embedder()
    store = hm.Warm(1024, str(tmp_path / "s"), embedder=embedder)
    store.remember({"id": "a", "text": "The harbor opens at six in the morning."})
    store.remember({"id": "b", "text": "The market sells fresh bread and cheese on Sundays."})
    hits = store.recall("when do the boats start sailing", {"k": 1, "explain": True})
    assert hits["results"][0]["id"] == "a"
    assert hits["results"][0]["explanation"]["semanticScore"] is not None
    lexical = store.recall("harbor", {"k": 1, "semanticWeight": 0, "explain": True})
    assert lexical["results"][0]["explanation"]["semanticScore"] is None
