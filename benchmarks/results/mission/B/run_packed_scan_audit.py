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
E43_RECEIPT_SHA256 = "2213403d85feedaacb9c471d1a64e9e17bc8e796f32074e9a98d82051a3f8cee"
E43_ASSEMBLY_SHA256 = "642c997cf44ea888f98c36802ca6dc700cfd1c615d25006932312dba6117505c"
E43_KERNEL_SHA256 = "e4ce15da9e0aaf2164f89d749a86b68e3a107ab7185bed1937928d2ede655adf"
AUDITOR_SOURCES = SOURCES + ("check_packed_scan_parser.py",)


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
        match = re.fullmatch(r"([A-Za-z][A-Za-z0-9._]*)(?:\s+(.*))?", line)
        if match:
            result.append({"line": line_number, "mnemonic": match[1].lower(),
                           "operands": (match[2] or "").strip(), "text": line})
        else:
            raise ValueError(f"unparsed assembly line {line_number + 1}: {line}")
    return result


def writes_register(inst, register):
    if re.match(r"^(?:cmp|test|j|nop|prefetch)", inst["mnemonic"]):
        return False
    if re.fullmatch(r"(?:mul|div|idiv)[lq]", inst["mnemonic"]):
        return register in ("rax", "rdx")
    if inst["mnemonic"].startswith(("call", "syscall")):
        return True
    if inst["mnemonic"].startswith(("xchg", "xadd", "mulx")):
        return any(normalized_register(value) == register
                   for value in re.findall(r"%[a-z0-9]+", inst["operands"]))
    destination = inst["operands"].split(",")[-1].strip()
    return bool(re.fullmatch(r"%[a-z0-9]+", destination)
                and normalized_register(destination) == register)


def register_width(register):
    if re.fullmatch(r"%(?:r(?:ax|bx|cx|dx|si|di|bp|sp)|r(?:[89]|1[0-5]))", register):
        return 64
    if re.fullmatch(r"%(?:e(?:ax|bx|cx|dx|si|di|bp|sp)|r(?:[89]|1[0-5])d)", register):
        return 32
    raise ValueError("induction operands must be complete32-bit or64-bit GPRs")


def immediate_register(inst, stem):
    opcode = re.fullmatch(stem + r"([lq])", inst["mnemonic"])
    match = re.fullmatch(r"\$(-?\d+),\s*(%[a-z0-9]+)", inst["operands"])
    if not opcode or not match:
        raise ValueError(f"expected {stem} immediate/full-register form")
    width = register_width(match[2])
    if width != (64 if opcode[1] == "q" else 32):
        raise ValueError("instruction suffix and register width disagree")
    return int(match[1]), normalized_register(match[2]), width


def checked_induction_instructions(items):
    for inst in items:
        name = inst["mnemonic"]
        if (re.fullmatch(r"(?:lea|mov|xor|add|sub|inc|cmp|test)[lq]|nop[lw]?|jne|jnz", name)
                or re.match(r"^v(?:p|mov|broadcast|xor|add|sub|mul|cvt|cmp|zero|extract|insert|shuf|perm|blend|and|or|test)", name)):
            continue
        raise ValueError(f"unmodeled instruction in induction proof: {name}")


def trip_count(body, prefix):
    if len(body) < 2 or body[-1]["mnemonic"] not in ("jne", "jnz"):
        raise ValueError("loop must end with JNE/JNZ consuming the preceding producer's ZF")
    if any(inst["mnemonic"].startswith("j") for inst in prefix + body[:-1]):
        raise ValueError("induction proof requires straight-line initialization and one loop backedge")
    checked_induction_instructions(prefix + body)
    producer = body[-2]
    if producer["mnemonic"] in ("addq", "addl"):
        step, register, width = immediate_register(producer, "add")
        writes_inside = [inst for inst in body if writes_register(inst, register)]
        writes_before = [inst for inst in prefix if writes_register(inst, register)]
        if writes_inside != [producer] or not writes_before:
            raise ValueError("negative-to-zero induction has another counter write or no initialization")
        initial, init_register, init_width = immediate_register(writes_before[-1], "mov")
        if (init_register != register or init_width != width or not -(1 << (width - 1)) <= initial < 0
                or not 0 < step < (1 << (width - 1)) or (-initial) % step != 0):
            raise ValueError("negative-to-zero state/width/step does not prove an exact finite trip count")
        return {"form": "negative-to-zero-add-ZF", "register": register, "width_bits": width,
                "initial": initial, "step": step, "bound": 0, "trips": (-initial) // step,
                "flag_producer_line": producer["line"],
                "proof": "s_j=initial+j*step stays signed-negative before the final update and equals0 there; same-width ADD produces ZF and the immediately following JNE tests it. Unsigned wrap at the final addition is defined; no signed-range overflow occurs."}
    bound, register, width = immediate_register(producer, "cmp")
    writes_inside = [inst for inst in body if writes_register(inst, register)]
    writes_before = [inst for inst in prefix if writes_register(inst, register)]
    if len(writes_inside) != 1 or not writes_before:
        raise ValueError("zero-based induction requires one counter update and a known initializer")
    update = writes_inside[0]
    if update["mnemonic"] in ("incl", "incq"):
        update_width = register_width(update["operands"])
        if update_width != (64 if update["mnemonic"] == "incq" else 32):
            raise ValueError("increment suffix/register width mismatch")
        step, update_register = 1, normalized_register(update["operands"])
    else:
        step, update_register, update_width = immediate_register(update, "add")
    initializer = writes_before[-1]
    zeroed = False
    if initializer["mnemonic"] in ("movl", "movq"):
        value, init_register, init_width = immediate_register(initializer, "mov")
        zeroed = value == 0 and init_register == register and init_width in (32, width)
    elif initializer["mnemonic"] in ("xorl", "xorq"):
        match = re.fullmatch(r"(%[a-z0-9]+),\s*(%[a-z0-9]+)", initializer["operands"])
        if match:
            init_width = register_width(match[1])
            zeroed = (match[1] == match[2] and normalized_register(match[1]) == register
                      and init_width == (64 if initializer["mnemonic"] == "xorq" else 32)
                      and init_width in (32, width))
    if (not zeroed or update_register != register or update_width != width
            or not 0 < step <= bound < (1 << (width - 1)) or bound % step != 0):
        raise ValueError("zero-based state/width/step does not prove an exact finite trip count")
    return {"form": "zero-to-bound-compare-ZF", "register": register, "width_bits": width,
            "initial": 0, "step": step, "bound": bound, "trips": bound // step,
            "flag_producer_line": producer["line"],
            "proof": "s_j=j*step reaches bound for the first time without signed or unsigned wrap; the adjacent same-width CMP defines ZF consumed by JNE."}


def mask_stores(parsed):
    stores = []
    for inst in parsed:
        destination = re.search(r"(?:^|,)\s*(-?\d*)\((%[a-z0-9]+)([^)]*)\)$", inst["operands"])
        if not destination:
            continue
        source = inst["operands"][:destination.start()].strip().rstrip(",").strip()
        if destination[3] or register_width(destination[2]) != 64:
            raise ValueError("unmodeled indexed/non64-bit output store")
        if inst["mnemonic"] == "kmovd":
            valid = re.fullmatch(r"%k[0-7]", source) is not None
        elif inst["mnemonic"] == "movl":
            valid = register_width(source) == 32
        else:
            valid = False
        if not valid:
            raise ValueError("unmodeled memory destination or wrong-width mask store")
        stores.append({"base": destination[2], "offset": int(destination[1] or 0),
                       "line": inst["line"], "mnemonic": inst["mnemonic"], "source": source})
    if (len(stores) != 8 or len({store["base"] for store in stores}) != 1
            or sorted(store["offset"] for store in stores) != list(range(0, 32, 4))):
        raise ValueError("expected exactly eight32-bit output stores at offsets0,4,...,28 on one base")
    if stores[0]["base"] != "%rdx" or any(
            writes_register(inst, "rdx") for inst in parsed if inst["line"] <= stores[-1]["line"]):
        raise ValueError("mask stores must retain the unchanged third-argument RDX output base")
    return stores


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
    try:
        stores = mask_stores(parsed)
    except ValueError as error:
        stores = []
        failures.append(str(error))
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


def replay_existing_receipt(path, source, output):
    path = path.resolve(strict=True)
    if not path.is_file() or path.stat().st_size > 1048576:
        raise ValueError("existing receipt must be a regular file within1MiB")
    raw = path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != E43_RECEIPT_SHA256:
        raise ValueError("existing receipt is not the pinned E43 receipt")
    prior = json.loads(raw)
    if (prior.get("hypothesis") != "H-B-9" or prior.get("failure_phase") != "auditing-assembly"
            or prior.get("flags") != list(FLAGS) or prior.get("decision") != "failed-cross-assembly-gate"):
        raise ValueError("E43 provenance or fixed compiler flags mismatch")
    commands = [entry for entry in prior.get("commands", []) if entry.get("log") == "compiler.log"]
    if len(commands) != 1 or commands[0].get("status") != "passed" or commands[0].get("returncode") != 0:
        raise ValueError("E43 must contain exactly one successful compilation")
    argv = commands[0].get("argv", [])
    if (len(argv) != len(FLAGS) + 4 or argv[0] != prior.get("compiler")
            or argv[1:1 + len(FLAGS)] != list(FLAGS) or argv[-2] != "-o"
            or Path(argv[-3]).parts[-2:] != ("source", "packed_scan_tile.c")
            or Path(argv[-1]).name != "packed_scan_tile.s"):
        raise ValueError("actual E43 compiler command does not match the fixed strict target")
    hashes = prior.get("artifact_sha256")
    frozen_sources = prior.get("source_sha256")
    if not isinstance(hashes, dict) or not isinstance(frozen_sources, dict) or set(frozen_sources) != set(SOURCES):
        raise ValueError("E43 source/artifact hash manifest is malformed")
    if (hashes.get("packed_scan_tile.s") != E43_ASSEMBLY_SHA256
            or frozen_sources.get("packed_scan_tile.c") != E43_KERNEL_SHA256
            or frozen_sources.get("native_interval.c") != E36_REFERENCE_SHA256):
        raise ValueError("E43 assembly, kernel or E36 reference identity mismatch")
    for name in SOURCES:
        if hashes.get("source/" + name) != frozen_sources[name]:
            raise ValueError(f"E43 source/snapshot manifest mismatch: {name}")
    if (digest(source / "packed_scan_tile.c") != E43_KERNEL_SHA256
            or digest(source / "native_interval.c") != E36_REFERENCE_SHA256):
        raise ValueError("current kernel/reference differs from the exact E43 compiled sources")
    archive = output / "prior"
    archive.mkdir()
    (archive / "receipt.json").write_bytes(raw)
    root = path.parent
    for name, expected in hashes.items():
        if (not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", name)
                or any(part in (".", "..") for part in name.split("/"))
                or not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected)):
            raise ValueError("unsafe E43 artifact path or malformed hash")
        artifact = root
        for part in name.split("/"):
            artifact = artifact / part
            if artifact.is_symlink():
                raise ValueError(f"symlink is not allowed in E43 artifacts: {name}")
        if not artifact.is_file() or artifact.stat().st_size > 16777216 or digest(artifact) != expected:
            raise ValueError(f"E43 artifact missing, oversized or changed: {name}")
        target = archive / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(artifact, target)
        if digest(target) != expected:
            raise ValueError(f"E43 artifact changed while copying: {name}")
    assembly = output / "packed_scan_tile.s"
    shutil.copyfile(archive / "packed_scan_tile.s", assembly)
    if digest(assembly) != E43_ASSEMBLY_SHA256:
        raise ValueError("copied E43 assembly identity mismatch")
    return assembly, {
        "receipt_path": str(path), "receipt_sha256": E43_RECEIPT_SHA256,
        "assembly_sha256": E43_ASSEMBLY_SHA256, "kernel_sha256": E43_KERNEL_SHA256,
        "frozen_source_sha256": frozen_sources, "verified_artifact_sha256": hashes,
        "compiler": prior["compiler"], "compiler_flags": prior["flags"],
        "compiler_command": commands[0], "new_compilations": 0,
        "identity_note": "The old compiler/source/flags/assembly receipt is preserved intact under prior/. Only the separately hashed parser changes; no prior script or x86 code is executed."}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--existing-receipt", type=Path)
    args = parser.parse_args()
    source = Path(__file__).resolve().parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise SystemExit("H-B-9 requires a fresh empty directory; preserve previous artifacts")
    started = time.monotonic()
    receipt = {
        "hypothesis": "H-B-9",
        "claim": ("audit exact existing E43 assembly only; no compilation, x86 execution or timing"
                  if args.existing_receipt else "one compile-only fixed8x32 K100 gate; no x86 execution or timing"),
        "mode": "existing-assembly" if args.existing_receipt else "cross-compile",
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
        receipt["source_sha256"] = {name: digest(source / name) for name in AUDITOR_SOURCES}
        if receipt["source_sha256"]["native_interval.c"] != E36_REFERENCE_SHA256:
            raise RuntimeError("native arithmetic reference differs from the accepted E36 source")
        snapshot = output / "source"
        snapshot.mkdir()
        for name in AUDITOR_SOURCES:
            shutil.copyfile(source / name, snapshot / name)
        if {name: digest(snapshot / name) for name in AUDITOR_SOURCES} != receipt["source_sha256"]:
            raise RuntimeError("source changed while taking the frozen snapshot")
        model = json.loads((snapshot / "b9-cost.json").read_text())
        if (model["fixed_tile"]["queries"], model["fixed_tile"]["data_rows"],
                model["workload"]["dimension"]) != (8, 32, 100):
            raise RuntimeError("frozen B9 card/model configuration mismatch")
        if args.existing_receipt:
            receipt["decision"] = "verifying-existing-assembly"
            save()
            assembly, provenance = replay_existing_receipt(args.existing_receipt, source, output)
            receipt["existing_assembly"] = provenance
            receipt["compiler"] = provenance["compiler"]
        else:
            compiler = shutil.which("clang")
            if compiler is None:
                raise RuntimeError("existing clang is required; no install or compiler fallback")
            receipt["compiler"] = compiler
            save()
            run([compiler, "--version"], "compiler-identity.log")
            run([compiler, "-print-resource-dir"], "compiler-resource-dir.log")
            run(["git", "rev-parse", "HEAD"], "source-head.log")
            run(["git", "status", "--short", "--untracked-files=all", "--", *AUDITOR_SOURCES], "source-status.log")
            assembly = output / "packed_scan_tile.s"
            receipt["decision"] = "compiling"
            save()
            run([compiler, *FLAGS, snapshot / "packed_scan_tile.c", "-o", assembly], "compiler.log")
        remaining()
        receipt["decision"] = "auditing-assembly"
        save()
        audit = audit_assembly(assembly, output)
        remaining()
        if {name: digest(source / name) for name in AUDITOR_SOURCES} != receipt["source_sha256"]:
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
