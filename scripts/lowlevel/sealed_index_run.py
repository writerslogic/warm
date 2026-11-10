# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Run a complete benchmark while publishing an immutable shared index snapshot."""
import argparse
import fcntl
import hashlib
import json
import os
import subprocess
import time
from pathlib import Path


# Linux UAPI values are stable; standalone Python builds may omit these names.
F_ADD_SEALS, F_GET_SEALS = 1033, 1034
F_SEAL_SEAL, F_SEAL_SHRINK, F_SEAL_GROW, F_SEAL_WRITE = 1, 2, 4, 8


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
            expected = os.fstat(source.fileno()).st_size
            while block := source.read(1 << 20):
                if target.write(block) != len(block):
                    raise OSError("short write while publishing index snapshot")
                digest.update(block)
                copied += len(block)
            if copied != expected:
                raise OSError("index size changed while publishing its snapshot")
        seals = F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL
        fcntl.fcntl(fd, F_ADD_SEALS, seals)
        receipt.update(bytes=copied, sha256=digest.hexdigest(), seals=fcntl.fcntl(fd, F_GET_SEALS),
                       snapshot_seconds=time.perf_counter() - started, command=command)
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
        environment = os.environ.copy()
        environment["WARM_SHARED_INDEX_PATH"] = f"/proc/{os.getpid()}/fd/{fd}"
        result = subprocess.run(command, env=environment, check=False)
        receipt.update(exit_code=result.returncode, total_seconds=time.perf_counter() - started)
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
        return result.returncode if result.returncode >= 0 else 128 - result.returncode
    finally:
        os.close(fd)


if __name__ == "__main__":
    raise SystemExit(main())
