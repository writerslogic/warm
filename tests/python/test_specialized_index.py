# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Result allocation and padding at the specialized index boundaries."""
import sys

import numpy as np
import pytest

import writerslogic_warm as hm


@pytest.fixture
def filtered():
    base = np.array([[0, 1, 2, 3], [9, 8, 7, 6]], dtype=np.uint8)
    index = hm.FilteredIndex(base, [0, 1, 2], [0, 0], labels=1, threads=1)
    return index, base


@pytest.mark.parametrize("rows", [0, 1, 2])
@pytest.mark.parametrize("offset", [-1, 0, 1])
def test_filtered_result_byte_limit(filtered, rows, offset):
    index, base = filtered
    k = sys.maxsize // 8 + offset
    if rows == 0 and offset <= 0:
        assert index.search(base[:rows], [0], [], k).shape == (0, k)
        return
    expected = MemoryError if k <= sys.maxsize // 8 and rows * k <= sys.maxsize // 8 else ValueError
    with pytest.raises(expected):
        index.search(base[:rows], np.arange(rows + 1), np.zeros(rows), k)


def test_filtered_result_padding_and_deletes(filtered):
    index, base = filtered
    np.testing.assert_array_equal(index.search(base[:1], [0, 1], [0], 3), [[0, 1, -1]])
    assert index.delete(0)
    np.testing.assert_array_equal(index.search(base[:1], [0, 1], [0], 3), [[1, -1, -1]])


@pytest.mark.parametrize("k", [4095, 4096, 4097])
@pytest.mark.skipif(not hasattr(hm, "PartitionIndex"), reason="requires experimental feature")
def test_partition_result_limit_and_padding(k):
    index = hm.PartitionIndex.create(4, lists=2, train_at=100, pq_m=2, threads=1)
    rows = np.array([[0, 1, 2, 3], [9, 8, 7, 6]], dtype=np.float32)
    index.insert([8, 9], rows)
    if k > 4096:
        with pytest.raises(ValueError, match="k must be in"):
            index.search(rows[:1], k)
    else:
        result = index.search(rows[:1], k)
        assert result.shape == (1, k)
        np.testing.assert_array_equal(result[0, :2], [8, 9])
        assert (result[0, 2:] == -1).all()
