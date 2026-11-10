# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""Build and check one frozen H-B-7 correctness target with durable receipts."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path


SECONDS_CAP = 180
FLAGS = ("-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-fno-fast-math",
         "-ffp-model=strict", "-ffp-contract=off", "-fPIC")
SOURCES = ("next-card-b7.md", "native_interval.c", "check_native_interval.py", "run_native_interval.py")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    source = Path(__file__).resolve().parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise SystemExit("H-B-7 requires a fresh empty output directory; preserve earlier receipts")
    started = time.monotonic()
    receipt = {
        "hypothesis": "H-B-7",
        "claim": "native arithmetic gate only; no throughput measurement",
        "seconds_cap": SECONDS_CAP,
        "source_directory": str(source),
        "source_sha256": {name: digest(source / name) for name in SOURCES},
        "platform": {"system": platform.system(), "machine": platform.machine(), "release": platform.release()},
        "python": {"executable": sys.executable, "version": sys.version},
        "compiler_flags": list(FLAGS),
        "compiler_convention_source": "benchmarks/public/native_recovery/run.py:113 uses C11, Wall, Wextra and Werror; this gate adds strict FP and explicitly selects clang",
        "commands": [],
        "decision": "not-run",
    }

    def save():
        temporary = output / "receipt.json.pending"
        temporary.write_text(json.dumps(receipt, indent=2) + "\n")
        temporary.replace(output / "receipt.json")

    def run(command, log_name):
        remaining = SECONDS_CAP - (time.monotonic() - started)
        if remaining <= 0:
            raise TimeoutError("180-second combined compiler/check cap exceeded")
        command = [str(argument) for argument in command]
        entry = {"argv": command, "log": log_name, "status": "starting"}
        receipt["commands"].append(entry)
        save()
        environment = os.environ.copy()
        environment.update({"LC_ALL": "C", "PYTHONDONTWRITEBYTECODE": "1", "PYTHONHASHSEED": "0"})
        with (output / log_name).open("wb") as log:
            child = subprocess.Popen(command, cwd=source, env=environment, stdout=log,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            try:
                returncode = child.wait(timeout=remaining)
            except subprocess.TimeoutExpired as error:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
                entry.update({"status": "timeout", "returncode": child.returncode})
                save()
                raise TimeoutError("180-second combined compiler/check cap exceeded") from error
        entry.update({"status": "passed" if returncode == 0 else "failed", "returncode": returncode})
        save()
        if returncode != 0:
            raise RuntimeError(f"command failed with exit {returncode}; see {log_name}")

    save()
    try:
        compiler = shutil.which("clang")
        if compiler is None:
            raise RuntimeError("clang is required; no install or compiler fallback is permitted")
        system = platform.system()
        if system == "Darwin":
            library = output / "libnative_interval.dylib"
            shared_flags = ["-dynamiclib"]
            disassembler = shutil.which("otool")
            disassembly_args = ["-tvV"]
        elif system == "Linux":
            library = output / "libnative_interval.so"
            shared_flags = ["-shared"]
            disassembler = shutil.which("llvm-objdump") or shutil.which("objdump")
            disassembly_args = ["-d"]
        else:
            raise RuntimeError("this fixed gate supports Darwin or Linux only")
        if disassembler is None:
            raise RuntimeError("an existing platform disassembler is required; no install is permitted")
        receipt.update({"compiler": compiler, "disassembler": disassembler,
                        "shared_library_flags": shared_flags, "library": str(library)})
        save()
        run([compiler, "--version"], "compiler-identity.log")
        run(["git", "rev-parse", "HEAD"], "source-head.log")
        run(["git", "status", "--short", "--untracked-files=all", "--", *SOURCES], "source-status.log")
        run([compiler, *FLAGS, *shared_flags, source / "native_interval.c", "-o", library, "-lm"], "compiler.log")
        run([disassembler, *disassembly_args, library], "disassembly.log")
        run([sys.executable, source / "check_native_interval.py", "--library", library,
             "--output", output / "check.json"], "check.log")
        checked = json.loads((output / "check.json").read_text())
        if (checked.get("decision") != "accept-native-arithmetic-only" or checked.get("failures")
                or checked.get("native_prunes", 0) <= 0
                or checked.get("exact_fallback_decisions", 0) <= 0
                or checked.get("rounding_modes") != [0, 1, 2, 3]
                or checked.get("environment_error_paths", 0) != 12
                or not checked.get("uint128_maximum_seen")):
            raise RuntimeError("fixed native arithmetic acceptance conditions failed; see check.json")
        after = {name: digest(source / name) for name in SOURCES}
        if after != receipt["source_sha256"]:
            raise RuntimeError("source changed during the frozen native arithmetic gate")
        receipt["decision"] = "accept-native-arithmetic-only"
    except Exception as error:
        receipt.update({"decision": "failed-native-gate", "failure_type": type(error).__name__, "failure": str(error)})
    finally:
        receipt["artifact_sha256"] = {
            path.name: digest(path) for path in sorted(output.iterdir())
            if path.is_file() and path.name not in ("receipt.json", "receipt.json.pending")
        }
        save()
        print(json.dumps({"hypothesis": "H-B-7", "decision": receipt["decision"], "receipt": str(output / "receipt.json")}))
    if receipt["decision"] != "accept-native-arithmetic-only":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
