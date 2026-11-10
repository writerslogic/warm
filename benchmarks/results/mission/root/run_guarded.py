# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Bound a self-logging counter and recover its process group on interruption."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=int, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or not 1 <= args.seconds <= 7200 or args.log.resolve() == args.receipt.resolve():
        parser.error("bounded deadline, command and distinct artifact paths required")
    if args.log.exists() or args.receipt.exists():
        parser.error("preserve previous guard artifacts")
    args.log.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    child = None
    started = time.monotonic()
    result = {"argv": command, "outer_seconds": args.seconds, "timing_claim": False}

    def interrupted(number, frame):
        raise InterruptedError(f"guard interrupted by signal {number}")

    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        with args.log.open("x") as log:
            child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            result["exit_code"] = child.wait(timeout=args.seconds)
            result["status"] = "finished"
    except (subprocess.TimeoutExpired, InterruptedError) as error:
        result.update(status="outer-timeout-or-interruption", error_type=type(error).__name__)
    finally:
        if child is not None and child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
            result["exit_code"] = child.returncode
        result["elapsed_guard_seconds"] = time.monotonic() - started
        with args.receipt.open("x") as stream:
            json.dump(result, stream, indent=2)
            stream.write("\n")
    print(json.dumps({key: result[key] for key in ("status", "exit_code") if key in result}))
    return 0 if result.get("status") == "finished" and result.get("exit_code") == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
