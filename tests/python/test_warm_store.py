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


FORM_PAGE = "\n".join([
    "RootWebArea 'Checkout'",
    "\t[12] textbox 'Email', required, value='a@b.c'",
    "\t[13] combobox 'Country', expanded=False",
    "\t\t[14] option 'Chile', selected=True",
    "\t\t[15] option 'Peru'",
    "\t[16] button 'Pay'",
])


def _observation(step, page):
    return {"episode": "run-1", "step": step, "url": "http://shop/checkout",
            "tree": {"role": "RootWebArea", "name": "Checkout"}, "accessibilityTree": page}


def test_experience_form_tables_round_trip(tmp_path):
    store = hm.Warm(1024, str(tmp_path / "store"))
    store.remember(_observation(0, FORM_PAGE))
    store.remember(_observation(1, FORM_PAGE.replace("a@b.c", "x@y.z")))
    header = "- Form controls (structured from the page text; * = selected option):\n"
    expected = [
        header + "  textbox 'Email' value='a@b.c', required\n"
        "  combobox 'Country' expanded=False options: *Chile | Peru\n  button 'Pay'\n",
        header + "  textbox 'Email' value='x@y.z', required\n"
        "  [2 controls unchanged from an earlier state omitted]\n",
        "",
    ]
    assert store.experience_form_tables([("run-1", 0), ("run-1", 1), ("run-1", 9)]) == expected
    del store
    assert hm.Warm(1024, str(tmp_path / "store")).experience_form_tables(
        [("run-1", 0), ("run-1", 1), ("run-1", 9)]) == expected


def test_experience_form_state_switch_off_stores_no_controls(tmp_path):
    store = hm.Warm(1024, str(tmp_path / "store"), experience_form_state=False)
    store.remember(_observation(0, FORM_PAGE))
    assert store.experience_form_tables([("run-1", 0)]) == [""]


def test_read_only_reader_refresh_and_context_manager(tmp_path):
    path = str(tmp_path / "store")
    with hm.Warm(1024, path) as writer:
        writer.memorize_document({"id": "one", "text": "first committed document"})
        reader = hm.Warm.open_read_only(1024, path)
        assert reader.read_only and not writer.read_only
        assert reader.refresh() is False

        writer.memorize_document({"id": "two", "text": "second committed document"})
        with pytest.raises(hm.ReadOnlyError):
            reader.memorize_document({"id": "x", "text": "denied"})
        assert reader.refresh() is True
        assert reader.refresh() is False
        hits = reader.search_documents("second committed", {"k": 2})
        assert "two" in {hit["documentId"] for hit in hits}
        reader.close()
        assert not os.path.exists(os.path.join(path, ".hms.readers"))


def test_close_releases_the_lock_and_later_calls_fail(tmp_path):
    path = str(tmp_path / "store")
    store = hm.Warm(1024, path)
    store.memorize_document({"id": "one", "text": "committed before close"})
    with pytest.raises(hm.StoreLockedError, match="already open"):
        hm.Warm(1024, path)
    store.close()
    store.close()
    assert store.closed
    with pytest.raises(hm.StoreClosedError):
        store.memorize_document({"id": "two", "text": "late"})
    with hm.Warm(1024, path) as again:
        assert not again.closed
    assert again.closed


def test_write_batch_is_atomic_ordered_and_reports_each_operation(tmp_path):
    path = str(tmp_path / "store")
    store = hm.Warm(1024, path)
    store.memorize_document({"id": "old", "text": "old document to remove"})
    put = lambda i, t: {"op": "memorize", "document": {"id": i, "text": t}}
    delete = lambda i: {"op": "delete", "id": i}

    results = store.write_batch(
        [put("a", "first document"), delete("old"), put("a", "final version of a"), delete("missing")]
    )
    assert results == [
        {"op": "memorize", "chunks": 1},
        {"op": "delete", "existed": True},
        {"op": "memorize", "chunks": 1},
        {"op": "delete", "existed": False},
    ]
    assert store.get_document("old") is None
    assert store.search_documents("final version", {"k": 1})[0]["documentId"] == "a"

    before = store.search_documents("final version", {"k": 5})
    for bad in (put("", "no id"), delete(""), {"op": "upsert", "id": "x"}, {"op": "delete"}):
        with pytest.raises(ValueError):
            store.write_batch([put("new", "would be added"), delete("a"), bad])
        assert store.get_document("new") is None
        assert store.get_document("a") is not None
    with pytest.raises(ValueError):
        store.write_batch([])
    with pytest.raises(ValueError, match="4096"):
        store.write_batch([delete(f"x{i}") for i in range(4097)])
    assert store.search_documents("final version", {"k": 5}) == before

    del store
    reopened = hm.Warm(1024, path)
    assert reopened.get_document("old") is None
    assert reopened.search_documents("final version", {"k": 1})[0]["documentId"] == "a"


def test_write_batch_is_visible_whole_to_a_reader_and_denied_to_it(tmp_path):
    path = str(tmp_path / "store")
    writer = hm.Warm(1024, path)
    writer.memorize_document({"id": "seed", "text": "seed document"})
    reader = hm.Warm.open_read_only(1024, path)
    with pytest.raises(hm.ReadOnlyError):
        reader.write_batch([{"op": "delete", "id": "seed"}])
    writer.write_batch(
        [
            {"op": "memorize", "document": {"id": "p", "text": "pair zebra"}},
            {"op": "memorize", "document": {"id": "q", "text": "pair zebra"}},
            {"op": "delete", "id": "seed"},
        ]
    )
    assert reader.refresh()
    ids = sorted(h["documentId"] for h in reader.search_documents("pair zebra", {"k": 5}))
    assert ids == ["p", "q"]
    assert reader.get_document("seed") is None
