# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Fairness rules of the generated comparison: per-build ANN verdicts, held-out operating points, commit
contracts, honest intervals, and the raw-data records."""

import copy
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from test_table import fake_ann  # noqa: E402
from warmbench import analyze, rawdata, registry, table  # noqa: E402


def two_build_file():
    """hnswlib with a fast-building, slower-serving build `m16` and a slow-building, faster `m32`."""
    data = fake_ann(2000, 1500)
    entry = data["systems"]["hnswlib"]
    slow = copy.deepcopy(entry["builds"]["b"])
    entry["builds"] = {"m16": entry["builds"]["b"], "m32": slow}
    for round_ in slow["serve_rounds"]:
        for c in round_["configs"]:
            for r in c["repeats"]:
                r["qps"] *= 1.1
    for run in slow["build_runs"]:
        run["build_s"] = 9.0
    data["systems"][registry.WARM]["builds"] = {"vertex-d32": data["systems"][registry.WARM]["builds"]["b"]}
    return data


def test_per_build_entries_never_mix_builds():
    data = two_build_file()
    entries, _ = table.per_build_for(Path("x.json"), data)
    assert entries
    for e in entries:
        if e["metric"].startswith("qps") or e["metric"] in ("build_seconds", "serving_rss_mib", "index_bytes_mib"):
            assert f"builds.{e['build']}" in e["theirs"]["source"]
    seconds = {e["build"]: e["theirs"]["value"] for e in entries if e["metric"] == "build_seconds"}
    assert seconds == {"m16": 2.0, "m32": 9.0}
    qps = {e["build"]: e["theirs"]["value"] for e in entries if e["metric"] == "qps@recall0.95"}
    assert qps["m32"] > qps["m16"]


def test_main_table_draws_no_verdict_on_per_build_metrics():
    assert analyze.no_verdict("ann", "build_seconds") and analyze.no_verdict("ann", "serving_rss_mib")
    assert not analyze.no_verdict("text", "build_seconds") and not analyze.no_verdict("ann", "qps@recall0.95")
    assert not analyze.no_verdict("ann", "build_seconds", "server")


def split_config(select_recall, report_recall, select_qps, report_qps):
    return {"params": {"ef": 1}, "recall_at_10": 0.9,
            "select": {"recall_at_10": select_recall, "repeats": [{"qps": select_qps}]},
            "report": {"recall_at_10": report_recall, "repeats": [{"qps": report_qps}]}}


def test_operating_point_is_chosen_on_one_half_and_scored_on_the_other():
    rounds = [{"configs": [split_config(0.96, 0.96, 1000, 700), split_config(0.96, 0.96, 800, 900)]} for _ in range(3)]
    best, _ = analyze.pick_config({"b": {"serve_rounds": rounds}}, 0.95)
    assert best[2] == 0 and best[0] == 1000  # chosen by the selection half's QPS
    data = fake_ann(1, 1)
    data["systems"]["hnswlib"]["builds"]["b"]["serve_rounds"] = rounds
    cell = analyze.ann_metrics("x.json", data)["qps@recall0.95"]["hnswlib"]
    assert cell["value"] == 700 and "odd-indexed" in cell["note"]  # scored on the report half


def test_report_half_below_target_is_not_reported_as_reaching_it():
    rounds = [{"configs": [split_config(0.96, 0.94, 1000, 900)]} for _ in range(3)]
    data = fake_ann(1, 1)
    data["systems"]["hnswlib"]["builds"]["b"]["serve_rounds"] = rounds
    cell = analyze.ann_metrics("x.json", data)["qps@recall0.95"]["hnswlib"]
    assert cell["value"] is None and "report-half recall" in cell["note"]


def test_few_rounds_give_a_range_not_a_bootstrap():
    assert analyze.interval([3.0, 1.0, 2.0], 0.95) == [1.0, 3.0]
    wide = [float(i) for i in range(10)]
    low, high = analyze.interval(wide, 0.95)
    assert 0 < low and high < 9  # a bootstrap interval is inside the range


def ops_file(comparator_class_system):
    data = fake_ann(2000, 1000)
    data["systems"] = {registry.WARM: data["systems"][registry.WARM], comparator_class_system: data["systems"]["hnswlib"]}
    data["systems"][registry.WARM]["ops"] = {n: {"supported": True, "per_op_ms_durable": 1.0} for n in ("replace", "delete")}
    data["systems"][comparator_class_system]["ops"] = {n: {"supported": True, "per_op_ms_durable": 300.0} for n in ("replace", "delete")}
    data["hardware"].update(cpu="c", logical_cores=1, memory_bytes=2**30, os="o")
    data["dataset"].update(documents=1, queries=1)
    data["facts"] = {}
    return data


@pytest.mark.parametrize("comparator, rows_present", [("hnswlib", False), ("sqlite-vec", True)])
def test_durable_rows_exist_only_against_the_same_commit_contract(comparator, rows_present, monkeypatch):
    data = ops_file(comparator)
    monkeypatch.setattr(table, "load_results", lambda: [(Path("x.json"), data)])
    markdown, cells, _, _ = table.build()
    assert ("Replace one vector (durable)" in markdown.split("<details>")[0]) is rows_present
    assert any(c["metric"] == "replace_ms_per_op" for c in cells) is rows_present
    assert registry.commit_contract(comparator)[0] in markdown  # the contract is printed per system
    if not rows_present:
        assert "Row not generated" in markdown and "69" not in markdown.split("Row not generated")[0].split("Replace")[-1]


def test_raw_arrays_round_trip_with_their_digest(tmp_path):
    store = rawdata.RawStore()
    store.add_latency("a/round0/pass0", [5, 7, 9])
    info = store.write(tmp_path / "result.json")
    path = tmp_path / info["file"]
    assert rawdata.sha256_file(path) == info["sha256"]
    with np.load(path) as loaded:
        assert loaded["a|round0|pass0"].tolist() == [5, 7, 9]


def test_serve_round_split_scores_each_half_separately():
    hits = [10, 0, 10, 0]  # even queries perfect, odd queries all wrong
    serve = {"configs": [{"params": {}, "recall_at_10": 0.5, "hits_per_query": hits,
                          "repeats": [{"qps": 1.0, "lat_ns": [100, 200, 100, 200]}]}]}
    out = rawdata.split_serve_round(serve, rawdata.RawStore(), "k")["configs"][0]
    assert out["select"]["recall_at_10"] == 1.0 and out["report"]["recall_at_10"] == 0.0
    assert out["select"]["repeats"][0]["qps"] == pytest.approx(1e7) and out["report"]["repeats"][0]["qps"] == pytest.approx(5e6)
    assert "hits_per_query" not in out and "lat_ns" not in out["repeats"][0]


def test_null_dataset_digests_stop_the_run():
    with pytest.raises(RuntimeError):
        rawdata.require_digests({"train_sha256": None, "test_sha256": "x"}, ("train_sha256", "test_sha256"))
    rawdata.require_digests({"train_sha256": "a", "test_sha256": "b"}, ("train_sha256", "test_sha256"))
