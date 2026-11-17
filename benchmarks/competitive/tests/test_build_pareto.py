# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""The QPS-at-recall read-off of the build-profile table."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_pareto as bp  # noqa: E402


def test_envelope_ignores_a_slower_point_at_higher_recall_gap():
    # (0.95, 100) is dominated by (0.97, 150): the envelope keeps the better QPS at every recall.
    pts = [(0.90, 400.0), (0.95, 100.0), (0.97, 150.0), (0.99, 50.0)]
    assert bp.qps_at(pts, 0.95) == 150.0


def test_interpolates_log_linear_and_reports_unreachable_recall():
    pts = [(0.90, 400.0), (0.99, 100.0)]
    mid = bp.qps_at(pts, 0.945)
    assert abs(mid - 200.0) < 1e-6  # geometric midpoint
    assert bp.qps_at(pts, 0.995) is None
    assert bp.qps_at(pts, 0.5) == 400.0
