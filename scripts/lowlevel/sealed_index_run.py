# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Run a complete benchmark while publishing an immutable shared index snapshot."""
import argparse
import fcntl
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path


# Linux UAPI values are stable; standalone Python builds may omit these names.
F_ADD_SEALS, F_GET_SEALS = 1033, 1034
F_SEAL_SEAL, F_SEAL_SHRINK, F_SEAL_GROW, F_SEAL_WRITE = 1, 2, 4, 8


def file_identity(info, *, times):
    fields = [info.st_dev, info.st_ino, info.st_size]
    return fields + [info.st_mtime_ns, info.st_ctime_ns] if times else fields


def verify_binding(encoded, source, snapshot):
    """Check the publisher's binding against the exact opened files, without rereading their data."""
    if type(encoded) is not str:
        raise ValueError("shared WARM snapshot binding is invalid")
    if len(encoded) > 2048:
        raise ValueError("shared WARM snapshot binding is too large")
    try:
        binding = json.loads(encoded)
    except ValueError as error:
        raise ValueError("shared WARM snapshot binding is invalid") from error
    if (type(binding) is not dict or set(binding) != {"version", "source", "snapshot"}
            or type(binding["version"]) is not int or binding["version"] != 1):
        raise ValueError("shared WARM snapshot binding is invalid")
    required = F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK
    if fcntl.fcntl(snapshot.fileno(), F_GET_SEALS) & required != required:
        raise ValueError("shared WARM snapshot is not immutable")
    actual = {"source": file_identity(os.fstat(source.fileno()), times=True),
              "snapshot": file_identity(os.fstat(snapshot.fileno()), times=False)}
    for name, expected in actual.items():
        supplied = binding[name]
        if (type(supplied) is not list or len(supplied) != len(expected)
                or any(type(value) is not int for value in supplied) or supplied != expected):
            raise ValueError("shared WARM snapshot differs from its published binding")
    if actual["source"][2] != actual["snapshot"][2]:
        raise ValueError("shared WARM snapshot has the wrong size")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--index", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a benchmark command is required")

    started = time.perf_counter()
    fd = os.memfd_create("warm-index-snapshot", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    receipt = {"kind": "sealed immutable index snapshot", "source": str(args.index)}
    try:
        digest = hashlib.sha256()
        copied = 0
        with args.index.open("rb") as source, os.fdopen(os.dup(fd), "wb") as target:
            source_identity = file_identity(os.fstat(source.fileno()), times=True)
            expected = source_identity[2]
            if not 0 < expected <= sys.maxsize:
                raise ValueError("index size must fit the process address space")
            while copied < expected:
                block = source.read(min(1 << 20, expected - copied))
                if not block:
                    raise OSError("index shrank while publishing its snapshot")
                if target.write(block) != len(block):
                    raise OSError("short write while publishing index snapshot")
                digest.update(block)
                copied += len(block)
            if source.read(1) or file_identity(os.fstat(source.fileno()), times=True) != source_identity:
                raise OSError("index changed while publishing its snapshot")
        seals = F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL
        fcntl.fcntl(fd, F_ADD_SEALS, seals)
        binding = {"version": 1, "source": source_identity,
                   "snapshot": file_identity(os.fstat(fd), times=False)}
        receipt.update(bytes=copied, sha256=digest.hexdigest(), seals=fcntl.fcntl(fd, F_GET_SEALS),
                       snapshot_seconds=time.perf_counter() - started, command=command, binding=binding)
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
        environment = os.environ.copy()
        environment["WARM_SHARED_INDEX_PATH"] = f"/proc/{os.getpid()}/fd/{fd}"
        environment["WARM_SHARED_INDEX_BINDING"] = json.dumps(binding, separators=(",", ":"))
        result = subprocess.run(command, env=environment, check=False)
        receipt.update(exit_code=result.returncode, total_seconds=time.perf_counter() - started)
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
        return result.returncode if result.returncode >= 0 else 128 - result.returncode
    finally:
        os.close(fd)


if __name__ == "__main__":
    raise SystemExit(main())
