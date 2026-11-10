# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""Fixed induction/flag and mask-store boundary fixtures; no compiler or x86 execution."""

import argparse
import hashlib
import json
from pathlib import Path

from run_packed_scan_audit import instructions, mask_stores, trip_count


def enumerated_first_equal(initial, step, width, target):
    mask = (1 << width) - 1
    state = initial & mask
    for iteration in range(1, 129):
        state = (state + step) & mask
        if state == (target & mask):
            return iteration
    raise ValueError("fixture's independently enumerated state does not terminate within128updates")


def negative_case(trips, width=64):
    register, suffix = ("%rcx", "q") if width == 64 else ("%ecx", "l")
    prefix = [f"mov{suffix} $-{4 * trips}, {register}", "leaq 88(%rdi), %rax"]
    body = ["vpdpbusd %zmm9, %zmm7, %zmm21", "subq $-128, %rax",
            f"add{suffix} $4, {register}", "jne .Lloop"]
    return prefix, body


def zero_case(trips, width=64):
    register, suffix = ("%rcx", "q") if width == 64 else ("%ecx", "l")
    prefix = ["xorl %ecx, %ecx"]
    body = ["vpdpbusd %zmm9, %zmm7, %zmm21", f"add{suffix} $4, {register}",
            f"cmp{suffix} ${4 * trips}, {register}", "jne .Lloop"]
    return prefix, body


def store_lines(mnemonic="kmovd", source="%k0", offsets=tuple(range(0, 32, 4)), explicit_zero=False):
    return [f"{mnemonic} {source}, {offset if offset or explicit_zero else ''}(%rdx)"
            for offset in offsets]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    destination = args.output.resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".pending")
    if destination.exists() or temporary.exists():
        raise SystemExit("preserve earlier parser receipts; output must not exist")
    checks = []

    for mnemonic, operands in (
            ("vcmpgt_oqps", "%zmm26, %zmm21, %k0"),
            ("vcmpps", "$30, %zmm26, %zmm21, %k0"),
            ("vmulps", "{ru-sae}, %zmm21, %zmm23, %zmm21"),
            ("kmovd", "%k0, (%rdx)"), ("retq", "")):
        line = mnemonic + ("\t" + operands if operands else "")
        parsed = instructions([line])[0]
        checks.append({"name": "whole_opcode_" + mnemonic,
                       "passed": parsed["mnemonic"] == mnemonic and parsed["operands"] == operands})
    for line in ("vcmpgt_oqps%zmm26, %zmm21, %k0", "kmovd%k0, (%rdx)", "retq!"):
        try:
            instructions([line])
            passed = False
        except ValueError:
            passed = True
        checks.append({"name": "reject_missing_opcode_separator_" + line, "passed": passed})

    def valid_trip(name, prefix, body, initial, width, target, expected):
        try:
            independent = enumerated_first_equal(initial, 4, width, target)
            result = trip_count(instructions(body), instructions(prefix))
            passed = independent == result["trips"] == expected
            checks.append({"name": name, "passed": passed, "expected_trips": expected,
                           "enumerated_trips": independent, "induction": result,
                           "fixed25_gate_passes": result["trips"] == 25})
        except Exception as error:
            checks.append({"name": name, "passed": False, "error": f"{type(error).__name__}: {error}"})

    def rejected_trip(name, prefix, body):
        try:
            result = trip_count(instructions(body), instructions(prefix))
            checks.append({"name": name, "passed": False, "unexpected_acceptance": result})
        except ValueError as error:
            checks.append({"name": name, "passed": True, "rejection": str(error)})
        except Exception as error:
            checks.append({"name": name, "passed": False, "error": f"{type(error).__name__}: {error}"})

    def stores(name, lines, expect_valid):
        try:
            result = mask_stores(instructions(lines))
            checks.append({"name": name, "passed": expect_valid,
                           "stores": result, "expected_valid": expect_valid})
        except ValueError as error:
            checks.append({"name": name, "passed": not expect_valid,
                           "rejection": str(error), "expected_valid": expect_valid})
        except Exception as error:
            checks.append({"name": name, "passed": False, "error": f"{type(error).__name__}: {error}"})

    for width in (32, 64):
        for trips in (24, 25, 26):
            prefix, body = negative_case(trips, width)
            valid_trip(f"negative_{width}_trip{trips}", prefix, body, -4 * trips, width, 0, trips)
            prefix, body = zero_case(trips, width)
            valid_trip(f"zero_{width}_trip{trips}", prefix, body, 0, width, 4 * trips, trips)
    negative_prefix, negative_body = negative_case(25)
    zero_prefix, zero_body = zero_case(25)
    rejected_trip("negative_flags_replaced_by_test", negative_prefix,
                  negative_body[:-1] + ["testq %rax, %rax", negative_body[-1]])
    rejected_trip("negative_flags_replaced_by_pointer_subtract", negative_prefix,
                  negative_body[:-1] + ["subq $-128, %rax", negative_body[-1]])
    rejected_trip("negative_intervening_counter_write", negative_prefix,
                  negative_body[:2] + ["movq %rax, %rcx"] + negative_body[2:])
    rejected_trip("negative_second_counter_increment", negative_prefix,
                  negative_body[:2] + ["addq $4, %rcx"] + negative_body[2:])
    rejected_trip("negative_prefix_low_alias_write", negative_prefix + ["movl %eax, %ecx"], negative_body)
    rejected_trip("negative_exchange_counter_write", negative_prefix,
                  negative_body[:2] + ["xchgq %rcx, %rax"] + negative_body[2:])
    rejected_trip("negative_mixed_width_zero_extension", ["movl $-100, %ecx"], negative_body)
    rejected_trip("negative_nondivisible_initial", ["movq $-98, %rcx"], negative_body)
    rejected_trip("negative_positive_initial_would_require_wrap", ["movq $100, %rcx"], negative_body)
    rejected_trip("negative_zero_step", negative_prefix,
                  negative_body[:2] + ["addq $0, %rcx", "jne .Lloop"])
    rejected_trip("negative_branch_predicate_changed", negative_prefix,
                  negative_body[:-1] + ["je .Lloop"])
    rejected_trip("negative_prefix_branch_can_skip_initialization",
                  ["jne .Lloop"] + negative_prefix, negative_body)
    rejected_trip("zero_flags_replaced_by_test", zero_prefix,
                  zero_body[:-1] + ["testq %rax, %rax", zero_body[-1]])
    rejected_trip("zero_flags_replaced_by_add", zero_prefix,
                  zero_body[:-1] + ["addq $4, %rax", zero_body[-1]])
    rejected_trip("zero_intervening_counter_write", zero_prefix,
                  zero_body[:1] + ["movq %rax, %rcx"] + zero_body[1:])
    rejected_trip("zero_prefix_partial_register_write", zero_prefix + ["movl %eax, %ecx"], zero_body)
    rejected_trip("zero_mixed_compare_width", zero_prefix,
                  zero_body[:2] + ["cmpl $100, %ecx", "jne .Lloop"])
    rejected_trip("zero_suffix_operand_width_mismatch", zero_prefix,
                  ["addq $4, %ecx", "cmpq $100, %rcx", "jne .Lloop"])
    rejected_trip("zero_unknown_implicit_counter_effect", zero_prefix,
                  zero_body[:1] + ["syscall"] + zero_body[1:])

    stores("kmovd_all_offsets_zero_implicit", store_lines(), True)
    stores("kmovd_all_offsets_zero_explicit", store_lines(explicit_zero=True), True)
    stores("movl_existing_form", store_lines("movl", "%eax"), True)
    stores("mixed32bit_store_forms", store_lines()[:4] + store_lines("movl", "%eax")[4:], True)
    stores("missing_offset28", store_lines(offsets=tuple(range(0, 28, 4))), False)
    stores("duplicate24_missing28", store_lines(offsets=(0, 4, 8, 12, 16, 20, 24, 24)), False)
    stores("extra_offset32", store_lines(offsets=tuple(range(0, 36, 4))), False)
    stores("offset_limit_plus_one", store_lines(offsets=(0, 4, 8, 12, 16, 20, 24, 29)), False)
    stores("mixed_output_bases", store_lines()[:-1] + ["kmovd %k0, 28(%r8)"], False)
    stores("output_base_reassigned", ["movq %rdi, %rdx"] + store_lines(), False)
    stores("different_abi_output_base", [line.replace("%rdx", "%r8") for line in store_lines()], False)
    stores("wrong16bit_width", store_lines("kmovw"), False)
    stores("wrong64bit_width", store_lines("movq", "%rax"), False)
    stores("invalid_mask_register", store_lines(source="%k8"), False)
    stores("indexed_output_destination", store_lines()[:-1] + ["kmovd %k0, 28(%rdx,%rax,4)"], False)
    stores("extra_wrong_width_memory_write", store_lines() + ["movq %rax, (%rdx)"], False)

    source = Path(__file__).resolve().parent
    failures = [check for check in checks if not check["passed"]]
    receipt = {
        "hypothesis": "H-B-9", "claim": "fixed parser state/flag/store fixtures only; no hardware claim",
        "source_sha256": {name: hashlib.sha256((source / name).read_bytes()).hexdigest()
                          for name in ("run_packed_scan_audit.py", "check_packed_scan_parser.py")},
        "oracle": "independent bounded enumeration of modulo2^width integer updates and equality; no reuse of the parser's division formula",
        "checks": checks, "check_count": len(checks), "failures": failures,
        "decision": "accept-parser-boundaries" if not failures else "reject-parser-boundaries",
    }
    temporary.write_text(json.dumps(receipt, indent=2) + "\n")
    temporary.replace(destination)
    print(json.dumps({"decision": receipt["decision"], "checks": len(checks),
                      "failures": len(failures), "receipt": str(destination)}))
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
