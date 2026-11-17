# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""The two text timing contracts (ids and documents): each verdict is drawn against the comparators of its own contract,
documents rows are checked against the corpus, and the stored-variant workers return what the corpus holds."""

import sys
import zlib
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from warmbench import registry, runner, table, text_worker  # noqa: E402


def fake_text(qps):
    """A text results file whose systems carry the given single-thread QPS, ten rounds each with a 1% spread."""
    def system(rate):
        rounds = [{"passes": [{"queries": 1000, "total_ns": int(1000 / (rate * (1 + 0.005 * (i % 3 - 1))) * 1e9), "p50_ns": 1000, "p99_ns": 5000}] * 3,
                   "peak_rss_bytes": 2**20, "cold_start_s": 0.1, "warmup_steady": True, "warmup_passes": 20} for i in range(10)]
        return {"serve_rounds": rounds, "build_runs": [{"ingest_s": 1.0, "close_s": 0.1, "peak_rss_bytes": 2**20, "disk_bytes": 2**20}] * 3,
                "contract": None, "ops": {}}
    systems = {name: system(rate) for name, rate in qps.items()}
    for name, entry in systems.items():
        entry["contract"] = registry.contract(name)
    return {"schema_version": 1, "kind": "text", "suite": "standard", "commit": "a" * 40, "date": "2026-11-14T00:00:00+0000",
            "hardware": {"class": "c", "cpu": "c", "logical_cores": 1, "memory_bytes": 2**30, "os": "o"},
            "dataset": {"id": "x", "documents": 10, "queries": 1}, "facts": {}, "timing_boundary": {}, "not_run": {},
            "protocol": {"rounds": 10, "contracts": {c: {"reference": registry.CONTRACT_REFERENCE[c]} for c in (registry.IDS, registry.DOCUMENTS)}},
            "systems": systems, "skipped": {}}


def test_each_contract_is_judged_against_its_own_comparators():
    # WARM's ids row beats the ids comparators; WARM's documents row loses to the documents comparator.
    data = fake_text({"warm-lexical-api": 2000, "lucene": 1000, "tantivy-rs-count": 1500,
                      "warm": 1000, "lucene-docs": 1500, "tantivy-rs-count-docs": 800})
    views = dict((d["contract"], d) for _, d in table.contract_views(Path("f.json"), data))
    assert set(views) == {registry.IDS, registry.DOCUMENTS}
    assert set(views[registry.IDS]["systems"]) == {"warm-lexical-api", "lucene", "tantivy-rs-count"}
    assert set(views[registry.DOCUMENTS]["systems"]) == {"warm", "lucene-docs", "tantivy-rs-count-docs"}
    result = {}
    for contract, view in views.items():
        metrics, _, _ = table.metrics_for(Path("f.json"), view)
        cells = metrics["qps"]
        name, other = table.best_other("qps", cells)
        result[contract] = (name, table.verdict(table.advantage("qps", cells[table.reference(view)], other)))
    assert result == {registry.IDS: ("tantivy-rs-count", "WIN"), registry.DOCUMENTS: ("lucene-docs", "LOSS")}


def test_both_contracts_share_one_bonferroni_family():
    data = fake_text({"warm-lexical-api": 2000, "lucene": 1000, "warm": 2000, "lucene-docs": 1000})
    views = [d for _, d in table.contract_views(Path("f.json"), data)]
    _, _, alone = table.metrics_for(Path("f.json"), views[0])
    _, level, together = table.metrics_for(Path("f.json"), views[0], views)
    assert together == 2 * alone and level > 0.95 and level == pytest.approx(1 - 0.05 / together)


def test_a_file_without_contracts_is_one_unchanged_view():
    data = fake_text({"warm": 2000, "lucene": 1000})
    data["protocol"].pop("contracts")
    assert table.contract_views(Path("f.json"), data) == [(Path("f.json"), data)]


def test_section_titles_and_text_name_the_contract_and_its_reference_row():
    data = fake_text({"warm-lexical-api": 2000, "lucene": 1000, "warm": 2000, "lucene-docs": 1000})
    for _, view in table.contract_views(Path("f.json"), data):
        sec = table.section(Path("f.json"), view)
        lines, _ = table.render_section(sec, [])
        text = "\n".join(lines)
        assert f"{view['contract']} contract" in sec["title"]
        assert registry.CONTRACT_LABEL[view["contract"]] in text
        assert f"`{registry.CONTRACT_REFERENCE[view['contract']]}` row" in text


def test_contracts_of_registered_rows():
    assert registry.contract("warm") == registry.contract("warm-eager") == registry.DOCUMENTS
    assert registry.contract("warm-lexical-api") == registry.IDS
    for base in ("lucene", "tantivy-rs", "tantivy-rs-count", "tantivy-py", "sqlite-fts5"):
        assert registry.contract(base) == registry.IDS and registry.contract(base + "-docs") == registry.DOCUMENTS
        assert base + "-docs" in registry.SYSTEMS and base + "-docs" in registry.TIMING_BOUNDARY and base + "-docs" in registry.COMPARATOR_SETTINGS
        assert registry.commit_contract(base + "-docs")[0] == registry.commit_contract(base)[0]


CORPUS = {"a": ("alpha text".encode(), 3), "b": ("δ beta".encode(), 7)}


def rows_for(checks):
    return [{"id": "q", "results": [{"documentId": i, "lexicalScore": 1.0, "check": c} for i, c in checks]}]


def good(i):
    text, bucket = CORPUS[i]
    return {"crc": zlib.crc32(text), "len": len(text), "start": 0, "end": len(text), "bucket": bucket}


def test_documents_check_accepts_the_corpus_and_flags_each_kind_of_difference():
    ok = runner._check_documents(rows_for([("a", good("a")), ("b", good("b"))]), CORPUS)
    assert ok["hits"] == 2 and not any(v for k, v in ok.items() if k != "hits")
    wrong_text = {**good("a"), "crc": good("a")["crc"] ^ 1}
    wrong_len = {**good("a"), "len": good("a")["len"] - 1}
    wrong_bucket = {**good("b"), "bucket": 8}
    counts = runner._check_documents(rows_for([("a", wrong_text), ("a", wrong_len), ("b", wrong_bucket), ("zz", good("a")), ("a", None)]), CORPUS)
    assert counts["text_mismatches"] == 2 and counts["bucket_mismatches"] == 1 and counts["unknown_ids"] == 2
    assert runner._check_documents(rows_for([("a", {**good("a"), "len": 0, "crc": 0, "end": 0})]), CORPUS)["empty_text"] == 1


def test_documents_check_compares_a_chunk_with_its_byte_range():
    text, bucket = CORPUS["a"]
    chunk = {"crc": zlib.crc32(text[:5]), "len": 5, "start": 0, "end": 5, "bucket": bucket}
    counts = runner._check_documents(rows_for([("a", chunk)]), CORPUS)
    assert counts["text_mismatches"] == 0 and counts["partial_ranges"] == 1


def test_pair_check_reports_order_and_score_differences():
    def row(ids):
        return {"id": "q", "results": [{"documentId": i, "lexicalScore": 2.0 - n} for n, i in enumerate(ids)]}
    assert runner._pair_ids([row("abc")], [row("abc")]) == {"queries": 1, "queries_with_different_id_lists": 0, "queries_differing_only_in_tie_order": 0, "max_abs_score_gap": 0.0}
    assert runner._pair_ids([row("abc")], [row("acb")])["queries_with_different_id_lists"] == 1
    assert runner._pair_ids([row("abc")], [row("ab")])["queries_with_different_id_lists"] == 1


def test_pair_check_passes_a_reordering_of_equal_scores_and_nothing_else():
    def row(ids, scores):
        return {"id": "q", "results": [{"documentId": i, "lexicalScore": s} for i, s in zip(ids, scores)]}
    tied = [3.0, 2.0, 2.0, 1.0]
    swapped = runner._pair_ids([row("abcd", tied)], [row("acbd", tied)])
    assert (swapped["queries_with_different_id_lists"], swapped["queries_differing_only_in_tie_order"]) == (0, 1)
    assert runner._pair_ids([row("abcd", tied)], [row("bacd", tied)])["queries_with_different_id_lists"] == 1
    assert runner._pair_ids([row("abcd", tied)], [row("axcd", tied)])["queries_with_different_id_lists"] == 1
    assert runner._pair_ids([row("abcd", tied)], [row("acbd", [3.0, 2.0, 1.5, 1.0])])["queries_with_different_id_lists"] == 1
    cut = [3.0, 2.0, 2.0]
    assert runner._pair_ids([row("abc", cut)], [row("abd", cut)])["queries_differing_only_in_tie_order"] == 1


def test_stored_rows_reach_their_binaries_with_the_stored_flags(monkeypatch, tmp_path):
    seen = {}
    monkeypatch.setattr(runner, "warm_binary", lambda name, example=False: tmp_path / name)
    monkeypatch.setattr(runner, "run_measured", lambda cmd, env=None, timeout=0: seen.setdefault("cmd", cmd) and ("", "", 0.0, 0))
    monkeypatch.setattr(runner.jars, "ensure", lambda version: ("cp", []))
    expect = {
        ("warm", "build"): ["--store-text", "true"], ("warm", "serve"): ["--digest", "true", "--api", "documents"],
        ("warm-lexical-api", "build"): ["--store-text", "false"], ("warm-lexical-api", "serve"): ["--api", "lexical"],
        ("tantivy-rs-docs", "build"): ["--stored", "true", "--docstore", registry.TANTIVY_DOCSTORE],
        ("tantivy-rs-count-docs", "serve"): ["--stored", "true", "--digest", "true", "--doc-cache", str(registry.TANTIVY_DOC_CACHE), "--collector", "count"],
        ("tantivy-rs", "serve"): ["--collector", "topk"], ("lucene-docs", "serve"): ["english", "stored"], ("lucene-dv-docs", "serve"): ["english", "stored-dv"], ("lucene-dv-docs", "build"): ["english", "stored-dv"], ("lucene", "serve"): ["3", "100"],
    }
    for (system, phase), flags in expect.items():
        seen.clear()
        runner._text_cmd(system, phase, tmp_path / "in.json", tmp_path / "store", tmp_path / "out.json", {})
        cmd = seen["cmd"]
        for flag in flags:
            assert flag in cmd, (system, phase, flag)
        if system in ("tantivy-rs", "lucene", "warm-lexical-api") or (system, phase) == ("warm", "build"):
            assert not {"--digest", "--stored", "stored"} & set(cmd) or (system, phase) == ("warm", "build") and "--digest" not in cmd


DOCS = [{"id": "one/α", "text": "apricot orchard in the valley", "metadata": {"bucket": 4}},
        {"id": "two", "text": "banana orchard near the river", "metadata": {"bucket": 9}},
        {"id": "three", "text": "cherry grove", "metadata": {"bucket": 1}}]


@pytest.mark.parametrize("system, base, package", [("sqlite-fts5-docs", "sqlite-fts5", "sqlite3"), ("tantivy-py-docs", "tantivy-py", "tantivy")])
def test_stored_variants_return_the_corpus_text_and_rank_like_their_ids_rows(tmp_path, system, base, package):
    pytest.importorskip(package)
    stored = text_worker.SYSTEMS[system](tmp_path / "docs")
    plain = text_worker.SYSTEMS[base](tmp_path / "ids")
    for adapter in (stored, plain):
        adapter.build(DOCS)
        getattr(adapter, "open_serving", adapter.open)()
    for query in ("orchard", "orchard valley river", "cherry"):
        got = stored.search(stored.prepare(query))
        ref = plain.search(plain.prepare(query))
        assert [h[0] for h in got] == [h[0] for h in ref]
        for id_, score, body, bucket in got:
            doc = next(d for d in DOCS if d["id"] == id_)
            assert (body, bucket) == (doc["text"], doc["metadata"]["bucket"])
            assert text_worker.check(body, bucket)["crc"] == zlib.crc32(doc["text"].encode())
    stored.open()
    stored.replace(DOCS[0])
    assert [h[0] for h in stored.search(stored.prepare("benchmarkrevision"))] == ["one/α"]
    stored.delete(DOCS[1])
    assert stored.search(stored.prepare("banana")) == []


def test_cross_hardware_summary_lists_each_class_and_names_verdicts_that_differ():
    """The same corpus on two hardware classes: WARM wins the ids contract on both, and wins the documents contract on one only."""
    secs = []
    for hw, comparator_rate in (("hw-a", 1200), ("hw-b", 2600)):
        data = fake_text({"warm-lexical-api": 2000, "lucene": 1000, "warm": 2000, "lucene-docs": comparator_rate})
        data["hardware"]["class"] = hw
        for _, view in table.contract_views(Path("f.json"), data):
            secs.append(table.section(Path("f.json"), view))
    text = "\n".join(table.across_hardware_lines(secs))
    assert "| x | ids | `hw-a` |" in text and "| x | ids | `hw-b` |" in text
    assert "x, documents contract: `hw-a` WIN; `hw-b` LOSS" in text
    assert "x, ids" not in text.split("differ between hardware classes:")[1]
    assert table.across_hardware_lines(secs[:2]) == []
