# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from warmbench import text_worker  # noqa: E402


@pytest.mark.parametrize("system, package", [("sqlite-fts5", "sqlite3"), ("tantivy", "tantivy"), ("whoosh", "whoosh")])
def test_text_mutations_are_visible_and_committed(tmp_path, system, package):
    pytest.importorskip(package)
    docs = [{"id": "one/α", "text": "apricot orchard"}, {"id": "two", "text": "banana orchard"}]
    adapter = text_worker.SYSTEMS[system](tmp_path / "store")
    adapter.build(docs)
    adapter.open()
    adapter.replace(docs[0])
    assert [i for i, _ in adapter.search(adapter.prepare("benchmarkrevision"))] == ["one/α"]
    adapter.delete(docs[1])
    assert adapter.search(adapter.prepare("banana")) == []
    adapter.open()
    assert [i for i, _ in adapter.search(adapter.prepare("benchmarkrevision"))] == ["one/α"]
    assert adapter.search(adapter.prepare("banana")) == []


def test_version_controls_are_selectable_but_never_default():
    from warmbench import envs, jars, registry

    controls = set(registry.TEXT_CONTROL_SYSTEMS)
    assert controls == {"sqlite-fts5-prev", "tantivy-prev", "lucene-prev"}
    assert not controls & set(registry.TEXT_SYSTEMS)
    assert controls - {"lucene-prev"} <= set(text_worker.SYSTEMS)
    for control in controls - {"lucene-prev"}:
        assert registry.SYSTEMS[control]["env"] in envs.PINS
        assert registry.SYSTEMS[control]["env"] != registry.SYSTEMS[control.removesuffix("-prev")]["env"]
    assert set(jars.PINNED) == set(jars.JAVA_RELEASE) == {jars.VERSION, jars.PREVIOUS}
    assert jars.PINNED[jars.VERSION] != jars.PINNED[jars.PREVIOUS]
