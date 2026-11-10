# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""One fixed cross-compilation and independent assembly count; never execute x86."""

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import time


CAP_SECONDS = 180
FLAGS = ("-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-fno-fast-math",
         "-ffp-model=strict", "-ffp-contract=off", "-ffreestanding",
         "-fno-vectorize", "-fno-slp-vectorize", "-masm=att", "-S",
         "-target", "x86_64-unknown-linux-gnu", "-march=icelake-server")
SOURCES = ("packed_scan_tile.c", "run_packed_scan_audit.py", "next-card-b9.md",
           "b9-cost.json", "native_interval.c")
SYMBOL = "warm_b9_hot"
E36_REFERENCE_SHA256 = "928437a162b85e0b0af4719b547d6cbcfcf29157bb95aabc124bf01e89cf13a0"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def normalized_register(register):
    aliases = {"eax": "rax", "ebx": "rbx", "ecx": "rcx", "edx": "rdx",
               "esi": "rsi", "edi": "rdi", "ebp": "rbp", "esp": "rsp",
               "ax": "rax", "al": "rax", "ah": "rax", "bx": "rbx", "bl": "rbx", "bh": "rbx",
               "cx": "rcx", "cl": "rcx", "ch": "rcx", "dx": "rdx", "dl": "rdx", "dh": "rdx",
               "si": "rsi", "sil": "rsi", "di": "rdi", "dil": "rdi",
               "bp": "rbp", "bpl": "rbp", "sp": "rsp", "spl": "rsp"}
    name = register.lstrip("%")
    return aliases.get(name, re.sub(r"^(r\d+)[dwb]$", r"\1", name))


def instructions(lines):
    result = []
    for line_number, original in enumerate(lines):
        line = original.split("#", 1)[0].strip()
        if not line or line.startswith(".") or line.endswith(":"):
            continue
        match = re.fullmatch(r"([A-Za-z][A-Za-z0-9.]*)\s*(.*)", line)
        if match:
            result.append({"line": line_number, "mnemonic": match[1].lower(),
                           "operands": match[2].strip(), "text": line})
        else:
            raise ValueError(f"unparsed assembly line {line_number + 1}: {line}")
    return result


def writes_register(inst, register):
    if re.match(r"^(?:cmp|test|j|nop|prefetch)", inst["mnemonic"]):
        return False
    if re.fullmatch(r"(?:mul|div|idiv)[lq]", inst["mnemonic"]):
        return register in ("rax", "rdx")
    destination = inst["operands"].split(",")[-1].strip()
    return bool(re.fullmatch(r"%[a-z0-9]+", destination)
                and normalized_register(destination) == register)


def trip_count(body, prefix):
    candidates = []
    for inst in body:
        if not re.fullmatch(r"cmp[lq]", inst["mnemonic"]):
            continue
        match = re.fullmatch(r"\$(\d+),\s*(%[a-z0-9]+)", inst["operands"])
        if not match:
            continue
        bound, register = int(match[1]), normalized_register(match[2])
        increments = []
        for update in body:
            operands = update["operands"]
            add = re.fullmatch(r"\$(\d+),\s*(%[a-z0-9]+)", operands)
            if re.fullmatch(r"add[lq]", update["mnemonic"]) and add:
                if normalized_register(add[2]) == register:
                    increments.append(int(add[1]))
            if re.fullmatch(r"inc[lq]", update["mnemonic"]):
                if normalized_register(operands) == register:
                    increments.append(1)
        writes_before = [earlier for earlier in prefix if writes_register(earlier, register)]
        zeroed = False
        for earlier in writes_before[-1:]:
            zero = re.fullmatch(r"(%[a-z0-9]+),\s*(%[a-z0-9]+)", earlier["operands"])
            if re.fullmatch(r"xor[lq]", earlier["mnemonic"]) and zero:
                zeroed |= normalized_register(zero[1]) == normalized_register(zero[2]) == register
            move = re.fullmatch(r"\$0,\s*(%[a-z0-9]+)", earlier["operands"])
            if re.fullmatch(r"mov[lq]", earlier["mnemonic"]) and move:
                zeroed |= normalized_register(move[1]) == register
        writes_inside = [update for update in body if writes_register(update, register)]
        if (zeroed and len(increments) == 1 and len(writes_inside) == 1 and body[-2] == inst
                and increments[0] > 0 and bound % increments[0] == 0):
            candidates.append({"register": register, "initial": 0, "step": increments[0],
                               "bound": bound, "trips": bound // increments[0]})
    if len(candidates) != 1:
        raise ValueError("expected one independently recognized zero-based add/compare loop counter")
    return candidates[0]


def audit_assembly(path, output):
    lines = path.read_text().splitlines()
    starts = [index for index, line in enumerate(lines)
              if re.fullmatch(rf"{SYMBOL}:\s*(?:#.*)?", line.strip())]
    if len(starts) != 1:
        raise ValueError("exactly one intact warm_b9_hot symbol is required")
    start = starts[0]
    ends = [index for index in range(start + 1, len(lines))
            if re.match(rf"\s*\.size\s+{SYMBOL}\s*,", lines[index])]
    if not ends:
        raise ValueError("ELF assembly function size delimiter missing")
    hot_lines = lines[start:ends[0] + 1]
    (output / "hot-function.s").write_text("\n".join(hot_lines) + "\n")
    parsed = instructions(hot_lines)
    (output / "static-instruction-counts.json").write_text(json.dumps({
        "assembly_sha256": digest(path), "hot_assembly_sha256": digest(output / "hot-function.s"),
        "counts": dict(sorted(Counter(inst["mnemonic"] for inst in parsed).items())),
        "parsed_instructions": parsed}, indent=2) + "\n")
    labels = {}
    for index, line in enumerate(hot_lines):
        match = re.fullmatch(r"([.$A-Za-z_][.$A-Za-z0-9_]*):\s*(?:#.*)?", line.strip())
        if match:
            labels[match[1]] = index
    backwards = []
    for inst in parsed:
        if inst["mnemonic"].startswith("j") and inst["operands"] in labels:
            if labels[inst["operands"]] < inst["line"]:
                backwards.append(inst)
    failures = []
    if len(backwards) != 1 or backwards[0]["mnemonic"] not in ("jne", "jnz"):
        raise ValueError("expected exactly one fixed non-unrolled dot loop with a jne backedge")
    backedge = backwards[0]
    first, last = labels[backedge["operands"]], backedge["line"]
    body = [inst for inst in parsed if first <= inst["line"] <= last]
    prefix = [inst for inst in parsed if inst["line"] < first]
    induction = trip_count(body, prefix)
    if induction["trips"] != 25:
        failures.append("loop trip count is not25; K100 packing/padding mismatch")
    static = Counter(inst["mnemonic"] for inst in parsed)
    dynamic = Counter()
    for inst in parsed:
        dynamic[inst["mnemonic"]] += induction["trips"] if first <= inst["line"] <= last else 1
    dot = [inst for inst in parsed if inst["mnemonic"] == "vpdpbusd"]
    if len(dot) != 16 or any(not first <= inst["line"] <= last or "%zmm" not in inst["operands"] for inst in dot):
        failures.append("expected16 ZMM non-saturating dot instructions inside the25-step loop")
    if dynamic["vpdpbusd"] != 400:
        failures.append("dynamic dot count is not400")
    prohibited = [inst for inst in parsed if re.search(
        r"^(?:call|push|pop|enter|leave)|^v(?:f[nm]|gather|scatter)|^vpdpbusds", inst["mnemonic"])]
    if prohibited:
        failures.append("hot helper call, stack operation, fused arithmetic, gather/scatter or saturating dot")
    stack = [inst for inst in parsed if re.search(r"%(?:rsp|rbp)\b", inst["operands"])]
    if stack:
        failures.append("hot function references a stack/frame register")
    branches = [inst for inst in parsed if inst["mnemonic"].startswith("j")]
    if branches != [backedge]:
        failures.append("unaccounted hot branch/tail call")
    expected = {"vmulps": 48, "vaddps": 32, "vsubps": 16, "vcvtdq2ps": 16, "vpsubd": 16}
    for mnemonic, count in expected.items():
        if dynamic[mnemonic] != count:
            failures.append(f"{mnemonic} count {dynamic[mnemonic]} != {count}")
    double = []
    for inst in parsed:
        if inst["mnemonic"] == "vpslld" and re.match(r"\$1,", inst["operands"]):
            double.append(inst)
        if inst["mnemonic"] == "vpaddd":
            operands = [value.strip() for value in inst["operands"].split(",")]
            if len(operands) == 3 and operands[0] == operands[1]:
                double.append(inst)
    if len(double) != 16:
        failures.append("expected16 exact integer doublings")
    comparisons = [inst for inst in parsed if re.fullmatch(r"vcmp[a-z0-9_]*ps", inst["mnemonic"])]
    if len(comparisons) != 16 or any("%k" not in inst["operands"] for inst in comparisons):
        failures.append("expected16 lane comparisons producing masks")
    rounding = Counter()
    for inst in parsed:
        if inst["mnemonic"] in ("vmulps", "vaddps", "vsubps"):
            tag = re.search(r"\{(ru-sae|rd-sae)\}", inst["operands"])
            rounding[(inst["mnemonic"], tag[1] if tag else "missing")] += 1
    expected_rounding = {("vmulps", "ru-sae"): 48, ("vaddps", "ru-sae"): 16,
                         ("vaddps", "rd-sae"): 16, ("vsubps", "rd-sae"): 16}
    if dict(rounding) != expected_rounding:
        failures.append("embedded rounding counts differ from the frozen E36 arithmetic")
    stores = []
    for inst in parsed:
        match = re.fullmatch(r"%[a-z0-9]+,\s*(-?\d*)\((%[a-z0-9]+)\)", inst["operands"])
        if inst["mnemonic"] == "movl" and match:
            stores.append({"base": match[2], "offset": int(match[1] or 0), "line": inst["line"]})
    groups = {}
    for store in stores:
        groups.setdefault(store["base"], []).append(store["offset"])
    if len(stores) != 8 or not any(sorted(offsets) == list(range(0, 32, 4)) for offsets in groups.values()):
        failures.append("missing or unrecognized eight volatile per-query mask stores")
    zmm = sorted({int(value) for inst in parsed for value in re.findall(r"%zmm(\d+)", inst["operands"])})
    memory = [inst for inst in parsed if "(" in inst["operands"]]
    report = {
        "hypothesis": "H-B-9", "claim": "assembly shape/count gate only; no x86 execution or throughput",
        "assembly_sha256": digest(path), "hot_assembly_sha256": digest(output / "hot-function.s"),
        "symbol": SYMBOL, "source_assembly_line": start + 1, "loop_induction": induction,
        "static_instruction_counts": dict(sorted(static.items())),
        "dynamic_instructions_per_256_pair_tile": dict(sorted(dynamic.items())),
        "total_dynamic_instructions_per_tile": sum(dynamic.values()),
        "rounding_counts": {f"{mnemonic}:{mode}": count for (mnemonic, mode), count in sorted(rounding.items())},
        "output_mask_stores": stores, "ZMM_register_numbers": zmm,
        "register_note": "register names and stack references are counted; peak live ranges are not inferred",
        "all_memory_operands": memory, "prohibited_instructions": prohibited, "stack_references": stack,
        "full_parsed_hot_instructions": parsed, "failures": failures,
        "resource_decision": "unresolved: target-specific mixed ports, sustained frequency and cache behavior are not evidenced",
        "semantic_limit": "Counts/rounding syntax do not prove operand dataflow or candidate correctness; parent must inspect full assembly against C/E36. This gate cannot replace later independent native packed-arithmetic checks.",
        "wrapper_limit": "Full assembly includes validation, roots and candidate/fallback mask composition outside the hot function. Their work is not included in the hot instruction count and is not free.",
        "decision": "reject-fixed-assembly" if failures else "accept-assembly-shape-only",
    }
    (output / "assembly-audit.json").write_text(json.dumps(report, indent=2) + "\n")
    (output / "assembly-audit.log").write_text(json.dumps({
        "decision": report["decision"], "failures": failures,
        "dynamic_instructions_per_256_pair_tile": report["dynamic_instructions_per_256_pair_tile"],
        "resource_decision": report["resource_decision"]}, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    source = Path(__file__).resolve().parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise SystemExit("H-B-9 requires a fresh empty directory; preserve previous artifacts")
    started = time.monotonic()
    receipt = {
        "hypothesis": "H-B-9", "claim": "one compile-only fixed8x32 K100 gate; no x86 execution or performance timing",
        "seconds_cap": CAP_SECONDS, "source_directory": str(source),
        "platform": {"system": platform.system(), "machine": platform.machine(), "release": platform.release()},
        "python": {"executable": sys.executable, "version": sys.version},
        "flags": list(FLAGS), "commands": [], "decision": "not-run",
        "compilation_qualification": "-ffreestanding uses clang resource intrinsic/stdint headers without a Linux libc/sysroot or mm_malloc. No malloc stub, link, object execution or dependency installation is performed. Lack of cross-target support is a toolchain result, not an algorithm limit.",
        "validation_boundary": "ABI size/alignment/overlap, metadata ranges, masks and codes are checked outside the hot function. Certificate provenance and kth-threshold validity remain explicit controller preconditions; this is not a production serving proof.",
        "unmatched_host": "Owned M4 can compile but cannot execute VNNI. Core Ultra185H availability does not establish an equivalent IceLake Xeon target; no host connection is made.",
    }

    def save():
        temporary = output / "receipt.json.pending"
        temporary.write_text(json.dumps(receipt, indent=2) + "\n")
        temporary.replace(output / "receipt.json")

    def remaining():
        left = CAP_SECONDS - (time.monotonic() - started)
        if left <= 0:
            raise TimeoutError("180-second cross-assembly cap exceeded")
        return left

    def run(command, log_name):
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
                code = child.wait(timeout=remaining())
            except (subprocess.TimeoutExpired, TimeoutError) as error:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
                entry.update({"status": "timeout", "returncode": child.returncode})
                save()
                raise TimeoutError("180-second cross-assembly cap exceeded") from error
            except BaseException:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
                entry.update({"status": "interrupted", "returncode": child.returncode})
                save()
                raise
        entry.update({"status": "passed" if code == 0 else "failed", "returncode": code})
        save()
        if code != 0:
            raise RuntimeError(f"command failed with exit {code}; see {log_name}")

    save()
    try:
        receipt["source_sha256"] = {name: digest(source / name) for name in SOURCES}
        if receipt["source_sha256"]["native_interval.c"] != E36_REFERENCE_SHA256:
            raise RuntimeError("native arithmetic reference differs from the accepted E36 source")
        snapshot = output / "source"
        snapshot.mkdir()
        for name in SOURCES:
            shutil.copyfile(source / name, snapshot / name)
        if {name: digest(snapshot / name) for name in SOURCES} != receipt["source_sha256"]:
            raise RuntimeError("source changed while taking the frozen snapshot")
        model = json.loads((snapshot / "b9-cost.json").read_text())
        if (model["fixed_tile"]["queries"], model["fixed_tile"]["data_rows"],
                model["workload"]["dimension"]) != (8, 32, 100):
            raise RuntimeError("frozen B9 card/model configuration mismatch")
        compiler = shutil.which("clang")
        if compiler is None:
            raise RuntimeError("existing clang is required; no install or compiler fallback")
        receipt["compiler"] = compiler
        save()
        run([compiler, "--version"], "compiler-identity.log")
        run([compiler, "-print-resource-dir"], "compiler-resource-dir.log")
        run(["git", "rev-parse", "HEAD"], "source-head.log")
        run(["git", "status", "--short", "--untracked-files=all", "--", *SOURCES], "source-status.log")
        assembly = output / "packed_scan_tile.s"
        receipt["decision"] = "compiling"
        save()
        run([compiler, *FLAGS, snapshot / "packed_scan_tile.c", "-o", assembly], "compiler.log")
        remaining()
        receipt["decision"] = "auditing-assembly"
        save()
        audit = audit_assembly(assembly, output)
        remaining()
        if {name: digest(source / name) for name in SOURCES} != receipt["source_sha256"]:
            raise RuntimeError("source changed during the frozen cross-assembly gate")
        receipt["decision"] = audit["decision"]
        receipt["resource_decision"] = audit["resource_decision"]
        if audit["failures"]:
            receipt["failure"] = "; ".join(audit["failures"])
    except (Exception, KeyboardInterrupt) as error:
        phase = receipt["decision"]
        receipt.update({"decision": "failed-cross-assembly-gate", "failure_phase": phase,
                        "failure_type": type(error).__name__, "failure": str(error)})
        (output / "failure.log").write_text(f"{type(error).__name__}: {error}\n")
    finally:
        receipt["artifact_sha256"] = {
            str(path.relative_to(output)): digest(path) for path in sorted(output.rglob("*"))
            if path.is_file() and path.name not in ("receipt.json", "receipt.json.pending")}
        save()
        print(json.dumps({"hypothesis": "H-B-9", "decision": receipt["decision"],
                          "receipt": str(output / "receipt.json")}))
    if receipt["decision"] != "accept-assembly-shape-only":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
