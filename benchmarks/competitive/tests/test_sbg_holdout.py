# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""The held-out query rules of sbg_holdout/MANIFEST.md: shapes, tags, deduplication and the source filter."""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sbg_holdout"))

import holdout  # noqa: E402


def test_shapes_follow_queries_full():
    rows = holdout.shapes(["griffith", "observatory", "los", "angeles"], holdout.Rng(1), 0)
    by_shape = {row["tags"][0]: row for row in rows}
    assert by_shape["intersection"]["query"] == "+griffith +observatory +los +angeles"
    assert by_shape["phrase"]["query"] == '"griffith observatory los angeles"'
    assert by_shape["phrase"]["tags"] == ["phrase", "phrase:num_tokens_4"]
    assert by_shape["union"]["tags"] == ["union", "global", "union:num_tokens_4"]
    assert by_shape["intersection_union"]["query"] == "+griffith observatory los angeles"
    assert by_shape["negated"]["query"] == "+griffith -observatory"
    assert by_shape["term"]["tags"] == ["term"]


def test_alternating_rules_and_degenerate_negation():
    second = {r["tags"][0]: r["query"] for r in holdout.shapes(["a", "b", "c"], holdout.Rng(1), 1)}
    assert second["intersection_union"] == "a +b c"
    assert second["negated"] == "+a -b -c"
    repeated = {r["tags"][0] for r in holdout.shapes(["who", "is", "who"], holdout.Rng(1), 1)}
    assert "negated" not in repeated


def test_source_filter_and_dedup(tmp_path):
    for question in ("who sings it", "don't stop", "when 2 dogs", "one"):
        assert bool(holdout.LETTERS_ONLY.match(question)) == (question == "who sings it" or question == "one")
    rows = [holdout.entry("a b", "union", 2), holdout.entry("a b", "union", 2), holdout.entry("c d", "union", 2)]
    seen = tmp_path / "seen.jsonl"
    seen.write_text(json.dumps(holdout.entry("c d", "union", 2)) + "\n")
    out = tmp_path / "out.jsonl"
    counter = holdout.write_set(rows, [str(seen)], str(out))
    assert counter["union"] == 1
    assert [json.loads(line)["query"] for line in out.read_text().splitlines()] == ["a b"]


def test_rng_is_deterministic_across_runs():
    a, b = holdout.Rng(7), holdout.Rng(7)
    assert [a.below(1000) for _ in range(5)] == [b.below(1000) for _ in range(5)]
