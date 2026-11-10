# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""A publisher's immutable inode binding replaces redundant per-worker content reads."""
import importlib.util
import json
import os
from pathlib import Path
import sys

import pytest

pytestmark = pytest.mark.skipif(sys.platform != "linux", reason="Linux sealed snapshots")


@pytest.fixture(scope="module")
def protocol():
    path = Path(__file__).resolve().parents[2] / "scripts/lowlevel/sealed_index_run.py"
    spec = importlib.util.spec_from_file_location("warm_snapshot_protocol", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture
def published(tmp_path, protocol):
    path = tmp_path / "source"
    path.write_bytes(b"snapshot identity fixture")
    fd = os.memfd_create("warm-binding-test", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    with path.open("r+b") as source, os.fdopen(fd, "w+b") as snapshot:
        snapshot.write(path.read_bytes())
        snapshot.flush()
        protocol.fcntl.fcntl(fd, protocol.F_ADD_SEALS, 15)
        binding = {"version": 1, "source": protocol.file_identity(os.fstat(source.fileno()), times=True),
                   "snapshot": protocol.file_identity(os.fstat(fd), times=False)}
        yield source, snapshot, binding


@pytest.mark.parametrize("length", [2047, 2048, 2049])
def test_binding_size_boundary(protocol, published, length):
    source, snapshot, binding = published
    encoded = json.dumps(binding)
    encoded += " " * (length - len(encoded))
    if length > 2048:
        with pytest.raises(ValueError, match="too large"):
            protocol.verify_binding(encoded, source, snapshot)
    else:
        protocol.verify_binding(encoded, source, snapshot)


@pytest.mark.parametrize("field,value", [
    ("version", True), ("version", 1.0), ("version", 2), ("source", []),
    ("snapshot", "wrong"), ("source", [True, 1, 1, 1, 1]), ("extra", 1),
])
def test_malformed_binding_is_rejected(protocol, published, field, value):
    source, snapshot, binding = published
    binding[field] = value
    with pytest.raises(ValueError):
        protocol.verify_binding(json.dumps(binding), source, snapshot)


@pytest.mark.parametrize("encoded", ["", "null", "[]", "{}", "not-json", None, b"{}", 1])
def test_missing_binding_is_rejected(protocol, published, encoded):
    source, snapshot, _ = published
    with pytest.raises(ValueError):
        protocol.verify_binding(encoded, source, snapshot)


def test_same_size_source_change_invalidates_binding(protocol, published):
    source, snapshot, binding = published
    source.seek(0)
    source.write(b"changed!")
    source.flush()
    info = os.fstat(source.fileno())
    os.utime(source.fileno(), ns=(info.st_atime_ns, binding["source"][3] + 1))
    with pytest.raises(ValueError, match="differs"):
        protocol.verify_binding(json.dumps(binding), source, snapshot)


def test_another_sealed_inode_is_rejected_even_with_identical_contents(protocol, published):
    source, snapshot, binding = published
    source.seek(0)
    fd = os.memfd_create("warm-other-snapshot", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    with os.fdopen(fd, "w+b") as other:
        other.write(source.read())
        other.flush()
        protocol.fcntl.fcntl(fd, protocol.F_ADD_SEALS, 15)
        with pytest.raises(ValueError, match="differs"):
            protocol.verify_binding(json.dumps(binding), source, other)


@pytest.mark.parametrize("seals", [0, 6, 10, 12])
def test_binding_requires_immutable_storage(protocol, published, seals):
    source, _, _ = published
    fd = os.memfd_create("warm-incomplete-seals", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    with os.fdopen(fd, "w+b") as snapshot:
        source.seek(0)
        snapshot.write(source.read())
        snapshot.flush()
        protocol.fcntl.fcntl(fd, protocol.F_ADD_SEALS, seals)
        binding = {"version": 1, "source": protocol.file_identity(os.fstat(source.fileno()), times=True),
                   "snapshot": protocol.file_identity(os.fstat(fd), times=False)}
        with pytest.raises(ValueError, match="not immutable"):
            protocol.verify_binding(json.dumps(binding), source, snapshot)
