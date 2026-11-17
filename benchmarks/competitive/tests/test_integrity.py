# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Regression tests of the benchmark-integrity rules: one verdict rule (a bootstrap of the factor over enough rounds),
one build boundary (durable and searchable) for every system, Lucene at steady state, full source revisions from main,
screened build subsets, pinned corpora, and no loss hidden by the stricter rule."""

import json
import re
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from test_contracts import fake_text  # noqa: E402
from test_table import fake_ann  # noqa: E402
from warmbench import analyze, common, registry, runner, table, textdata  # noqa: E402

HERE = Path(__file__).resolve().parents[1]


def qps_verdict(data):
    m, _, _ = table.metrics_for(Path("x.json"), data)
    adv = table.advantage("qps@recall0.95", m["qps@recall0.95"][registry.WARM], m["qps@recall0.95"]["hnswlib"])
    return adv, table.verdict(adv)


@pytest.mark.parametrize("rounds,expected", [(analyze.MIN_BOOTSTRAP_ROUNDS - 1, "n/v"), (analyze.MIN_BOOTSTRAP_ROUNDS, "WIN"),
                                             (analyze.MIN_BOOTSTRAP_ROUNDS + 1, "WIN")])
def test_a_verdict_needs_the_minimum_rounds_and_a_bootstrap_of_the_factor(rounds, expected):
    adv, v = qps_verdict(fake_ann(2000, 1000, rounds=rounds))
    assert v == expected
    assert adv["factor"] == pytest.approx(2.0, rel=0.05)  # the measured ratio is always printed
    kind = adv["interval_kind"]
    assert kind == ("range" if rounds < analyze.MIN_BOOTSTRAP_ROUNDS else "paired bootstrap")
    assert ("range" in table.interval_label(adv)) == (kind == "range")


def test_a_separate_session_rerun_is_resampled_unpaired():
    data = fake_ann(2000, 1000)
    adv, _ = qps_verdict(data)
    assert adv["interval_kind"] == "paired bootstrap"
    data["warm_rerun"] = {"commit": "b" * 40, "date": "2026-01-02", "note": "rerun"}
    adv, v = qps_verdict(data)
    assert adv["interval_kind"] == "bootstrap" and v == "WIN"


def test_the_factor_interval_is_not_the_ratio_of_separate_intervals():
    """Two rows that move together round by round: the paired bootstrap of the factor is far narrower than the
    cross-bounds ratio of the two separate intervals, which the old rule used and which called real leads a tie."""
    data = fake_ann(1000, 1000)
    for system, scale in ((registry.WARM, 1.06), ("hnswlib", 1.0)):
        for r, round_ in enumerate(data["systems"][system]["builds"]["b"]["serve_rounds"]):
            for rep in round_["configs"][0]["repeats"]:
                rep["qps"] = 1000 * scale * (1 + 0.2 * (r % 5))
    m, _, _ = table.metrics_for(Path("x.json"), data)
    mine, theirs = m["qps@recall0.95"][registry.WARM], m["qps@recall0.95"]["hnswlib"]
    cross = mine["ci"][0] / theirs["ci"][1]
    adv = table.advantage("qps@recall0.95", mine, theirs)
    assert cross < 1 < adv["ci"][0]
    assert table.verdict(adv) == "WIN"


def test_a_loss_without_a_verdict_stays_on_the_loss_lists(monkeypatch):
    data = fake_text({"warm-lexical-api": 1000, "lucene": 2000, "warm": 1000, "lucene-docs": 2000})
    for entry in data["systems"].values():
        entry["serve_rounds"] = entry["serve_rounds"][:3]
    data["protocol"]["rounds"] = 3
    views = [(Path("f.json"), d) for d in table.contract_views(Path("f.json"), data)]
    monkeypatch.setattr(table, "load_results", lambda: [(p, d[1]) for p, d in views])
    markdown, cells, _, gaps = table.build()
    qps = [c for c in cells if c.get("derived") and c["metric"] == "qps"]
    assert qps and {c["verdict"] for c in qps} == {"n/v"}
    behind = [g for g in gaps if g["metric"] == "qps"]
    assert behind and {g["status"] for g in behind} == {table.BEHIND}
    assert all("fewer than" in g["no_verdict_reason"] for g in behind)
    assert "### Measured behind, no verdict" in markdown and "min-max range" in markdown


def test_warm_build_time_must_include_the_durable_close():
    data = fake_text({"warm": 1000, "lucene-docs": 1000})
    m = analyze.text_metrics("x", data)
    assert "unverified_reason" not in m["build_seconds"]["warm"]
    for run in data["systems"]["warm"]["build_runs"]:
        run.pop("close_s", None)
    m = analyze.text_metrics("x", data)
    assert "durable close" in m["build_seconds"]["warm"]["unverified_reason"]
    assert "unverified_reason" not in m["build_seconds"]["lucene-docs"]  # the comparators' boundary already held it
    source = (HERE.parents[1] / "examples/competitive_text.rs").read_text()
    build = source[source.index('"build" => {'):source.index('"serve" => {')]
    assert '"ingest_s": (memorize + close)' in build and build.index("drop(core)") < build.index('"ingest_s"')


def test_lucene_rows_need_a_verified_steady_state():
    data = fake_text({"warm-lexical-api": 1000, "lucene": 1000})
    assert not analyze.steady_state_note("lucene", data["systems"]["lucene"]["serve_rounds"])
    data["systems"]["lucene"]["serve_rounds"][3] = {**data["systems"]["lucene"]["serve_rounds"][3], "warmup_steady": False, "warmup_passes": 5}
    note = analyze.steady_state_note("lucene", data["systems"]["lucene"]["serve_rounds"])
    assert "1 of 10 rounds" in note
    view = [d for d in table.contract_views(Path("f.json"), data)][0][1]
    m, _, _ = table.metrics_for(Path("f.json"), view)
    assert m["qps"]["lucene"]["validity"] == "unverified"
    assert table.verdict(table.advantage("qps", m["qps"]["warm-lexical-api"], m["qps"]["lucene"])) == "n/v"
    # The rule the JVM runs is the rule the results record.
    flags = registry.LUCENE_JVM_FLAGS["serve"]
    java = (HERE / "lucene/LuceneBm25.java").read_text()
    for key, prop in registry._WARMUP_PROPERTIES.items():
        assert f"-Dwarm.{prop}={registry.LUCENE_WARMUP[key]}" in flags
        assert f'"warm.{prop}"' in java
    assert registry.COMPARATOR_SETTINGS["lucene"]["warmup"]["min_passes"] == registry.LUCENE_WARMUP["min_passes"]


def test_an_abbreviated_revision_is_expanded_or_refused(monkeypatch):
    monkeypatch.setenv("WARM_BENCH_COMMIT", "0123abcd")
    monkeypatch.setattr(common, "ROOT", Path("/nonexistent"))
    with pytest.raises(SystemExit):
        common.commit()
    monkeypatch.setenv("WARM_BENCH_COMMIT", "a" * 40)
    assert common.commit() == "a" * 40


def test_a_draft_or_unreachable_source_is_refused_or_carries_no_verdict(monkeypatch, tmp_path):
    (tmp_path / ".git").mkdir()
    monkeypatch.setattr(common, "ROOT", tmp_path)
    outcomes = {("status", "--porcelain", "--untracked-files=no"): " M src/lib.rs\n", ("merge-base",): 1}

    class Done:
        def __init__(self, stdout="", returncode=0):
            self.stdout, self.returncode = stdout, returncode

    def fake_git(*args):
        if args[0] == "merge-base":
            return Done(returncode=outcomes[("merge-base",)])
        return Done(stdout=outcomes.get(args, ""))
    monkeypatch.setattr(common, "_git", fake_git)
    monkeypatch.delenv("WARM_BENCH_DRAFT", raising=False)
    with pytest.raises(SystemExit):
        common.source_record("standard", "a" * 40)
    monkeypatch.setenv("WARM_BENCH_DRAFT", "1")
    record = common.source_record("standard", "a" * 40)
    assert record["draft"] and len(record["draft_reasons"]) == 2
    data = fake_ann(2000, 1000) | {"source": record}
    assert qps_verdict(data)[1] == "n/v"
    outcomes.update({("status", "--porcelain", "--untracked-files=no"): "", ("merge-base",): 0})
    monkeypatch.delenv("WARM_BENCH_DRAFT")
    assert common.source_record("standard", "a" * 40)["draft"] is False


def test_a_build_subset_needs_a_screening_that_included_the_system(tmp_path):
    defined = list(runner._adapter_class("scann").BUILDS)
    assert len(defined) > 1
    assert runner.build_selection(["scann"], None)["scann"]["run"] == defined
    with pytest.raises(SystemExit):
        runner.build_selection(["scann"], {"scann": defined[:1]})
    screen = tmp_path / "screen.json"
    screen.write_text(json.dumps({"systems": {"hnswlib": {}}}))
    with pytest.raises(SystemExit):
        runner.build_selection(["scann"], {"scann": defined[:1]}, screen)
    screen.write_text(json.dumps({"systems": {"scann": {}}}))
    assert runner.build_selection(["scann"], {"scann": defined[:1]}, screen)["scann"]["run"] == defined[:1]


def test_pinned_corpora_must_rebuild_byte_for_byte():
    good = dict(zip(("input_sha256", "qrels_sha256"), textdata.DATASET_DIGESTS["msmarco:1m"]))
    assert textdata.verify_digests("msmarco:1m", good)
    with pytest.raises(SystemExit):
        textdata.verify_digests("msmarco:1m", good | {"input_sha256": "0" * 64})
    assert not textdata.verify_digests("unpinned", {})


def test_every_text_system_records_its_analyzer_and_build_threads():
    for system in registry.TEXT_SYSTEMS + [registry.WARM, registry.WARM_LEXICAL_API, "tantivy-rs-count-docs", "lucene-dv-docs"]:
        settings = registry.text_settings(system)
        assert {"analyzer", "stop_words", "build_threads"} <= set(settings), system


def test_interval_labels_say_what_they_are():
    sec_text = table.interval_text({"data": {"protocol": {"rounds": 3, "build_repeats": 2}}, "level": 0.99, "family": 4})
    assert "fewer than 8 serve rounds and build runs" in sec_text and "percentile bootstrap" in sec_text
    assert re.search(r"min-max range", sec_text)


def test_hybrid_report_states_machine_commit_and_date():
    sys.path.insert(0, str(HERE / "hybrid"))
    import hybrid_report
    hw = {"class": "linux-x86_64-x-8c-16g", "cpu": "X", "logical_cores": 8, "memory_bytes": 16 << 30, "os": "Linux", "machine": "x86_64"}
    files = {"scifact": {"hardware": hw | {"load_1m": 0.1}, "commit": "a" * 40, "source": {"draft": False}, "date": "2026-11-17T10:00:00+0000", "rounds": 10},
             "nfcorpus": {"hardware": hw | {"load_1m": 0.7}, "commit": "a" * 40, "source": {"draft": False}, "date": "2026-11-17T12:00:00+0000", "rounds": 10}}
    meta = hybrid_report.provenance(files)
    assert meta["hardware_class"] == [hw["class"]] and len(meta["hardware"]) == 1 and meta["commit"] == ["a" * 40] and not meta["draft"]
    text = "\n".join(hybrid_report.header(meta))
    assert hw["class"] in text and "a" * 40 in text and "2026-11-17T10:00:00+0000 to 2026-11-17T12:00:00+0000" in text
    # Results written before the provenance fields carry none: the report says so and marks them a draft.
    legacy = hybrid_report.provenance({"fiqa": {"hardware": hw}})
    assert legacy["draft"] and "Commit: not recorded" in "\n".join(hybrid_report.header(legacy))
