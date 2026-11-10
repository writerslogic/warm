# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Immutable shared index mappings; a regular mutable file must never be mapped."""
import os
import sys

import numpy as np
import pytest

import writerslogic_warm as hm


# Linux UAPI values are stable; standalone Python builds may omit these names.
F_ADD_SEALS, F_GET_SEALS = 1033, 1034
F_SEAL_SEAL, F_SEAL_SHRINK, F_SEAL_GROW, F_SEAL_WRITE = 1, 2, 4, 8
F_SEAL_FUTURE_WRITE = 16


@pytest.fixture(params=[("angular", False), ("euclidean", False), ("ip", False), ("ip", True)])
def saved(tmp_path, request):
    rows = np.random.default_rng(73).normal(size=(65, 17)).astype(np.float32)
    metric, direct_ip = request.param
    index = hm.VectorIndex(metric, degree=8, build_ef=32, threads=1, direct_ip=direct_ip)
    index.fit(rows)
    index.set_query_arguments(ef=64, rerank=64)
    path = tmp_path / "index.warm"
    index.save(path)
    return index, rows[:5], path, metric, direct_ip


def test_shared_load_rejects_mutable_file_and_preserves_existing_index(saved):
    index, queries, path, metric, direct_ip = saved
    expected = index.batch_query_array(queries, 10)
    with pytest.raises(RuntimeError):
        index.load(path, shared=True)
    np.testing.assert_array_equal(index.batch_query_array(queries, 10), expected)


@pytest.mark.skipif(sys.platform != "linux", reason="Linux seals")
@pytest.mark.parametrize("missing", [0, 1, 2, 4, 8])
def test_shared_load_requires_each_immutable_seal(saved, missing):
    import fcntl

    index, queries, path, metric, direct_ip = saved
    expected = index.batch_query_array(queries, 10)
    fd = os.memfd_create("warm-shared-test", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    try:
        with os.fdopen(os.dup(fd), "wb") as output:
            output.write(path.read_bytes())
        required = F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK
        # F_SEAL_SEAL is optional; omitting any content/size seal must reject the mapping.
        seals = (required | F_SEAL_SEAL) & ~missing
        fcntl.fcntl(fd, F_ADD_SEALS, seals)
        shared = hm.VectorIndex(metric, threads=1, direct_ip=direct_ip)
        if missing & required:
            with pytest.raises(RuntimeError, match="requires write, shrink and grow seals"):
                shared.load(f"/proc/self/fd/{fd}", shared=True)
            return
        shared.load(f"/proc/self/fd/{fd}", shared=True)
        shared.set_query_arguments(ef=64, rerank=64)
        for operation in (lambda: os.pwrite(fd, b"x", 0), lambda: os.ftruncate(fd, 0),
                          lambda: os.ftruncate(fd, path.stat().st_size + 1)):
            with pytest.raises(PermissionError):
                operation()
    finally:
        os.close(fd)
    # The mapping owns its pages after the publishing descriptor and source file disappear.
    path.unlink()
    np.testing.assert_array_equal(shared.batch_query_array(queries, 10), expected)


@pytest.mark.skipif(sys.platform != "linux", reason="Linux seals")
def test_future_write_seal_does_not_allow_a_mutable_mapping(saved):
    import fcntl
    import mmap

    index, _, path, _, _ = saved
    fd = os.memfd_create("warm-mutable-test", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    try:
        with os.fdopen(os.dup(fd), "wb") as output:
            output.write(path.read_bytes())
        with mmap.mmap(fd, 0) as writable:
            fcntl.fcntl(fd, F_ADD_SEALS,
                        F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_FUTURE_WRITE)
            with pytest.raises(RuntimeError, match="requires write, shrink and grow seals"):
                index.load(f"/proc/self/fd/{fd}", shared=True)
            writable[0] = 0
    finally:
        os.close(fd)


@pytest.mark.skipif(sys.platform != "linux", reason="Linux seals")
@pytest.mark.parametrize("corruption", ["empty", "magic", "truncated"])
def test_sealed_file_still_requires_a_valid_index(saved, corruption):
    import fcntl

    index, _, path, _, _ = saved
    data = path.read_bytes()
    data = {"empty": b"", "magic": b"BADMAGIC" + data[8:], "truncated": data[:-1]}[corruption]
    fd = os.memfd_create("warm-corrupt-test", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    try:
        with os.fdopen(os.dup(fd), "wb") as output:
            output.write(data)
        fcntl.fcntl(fd, F_ADD_SEALS,
                    F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK)
        with pytest.raises(RuntimeError):
            index.load(f"/proc/self/fd/{fd}", shared=True)
    finally:
        os.close(fd)
