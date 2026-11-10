# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Cauchy-only rejection certificate for the frozen4096-frontier family."""

import argparse
import csv
import fcntl
import hashlib
import json
import math
import os
import platform
import signal
import sys
from fractions import Fraction
from pathlib import Path


N = 256921
D = 128
QUERIES = 256
CURSOR_CAP = 4096
NORM_CHUNK_ROWS = 4096
CARD_SHA = "a8353f5ce09d375d6304002c1069193bf860c6b34ce79709654e9d2e2c665304"
RESEARCH_SHA = "9bb3468733a83fb01e98af82ebe8b7b15ad33ec31bcbc1314faabebf732c3d73"
INPUTS = {
    "base.f32": (N * D * 4, "1040eaaf8568ce7244572177e95fcaf78f1cba2e3e8c3cec8ea29918c675cf60"),
    "validation.f32": (QUERIES * D * 4, "c3394098fe3ba8302bfee1e7f307bb49b76e255cae43c404ecf2ccc5b1209c15"),
}
QUESTION = "Does the exact4096-frontier lower bound exceed a certified global Cauchy score ceiling?"
PREDICTION = "At least one of the256 exposed queries supplies a certified rejection; otherwise Cauchy is inconclusive."


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def rational(value):
    value = Fraction(value)
    return dict(numerator=str(value.numerator), denominator=str(value.denominator),
                approximate=float(value))


def f32_fraction(bits):
    require(type(bits) is int and 0 <= bits <= 0xFFFFFFFF, "invalid raw f32 bits")
    exponent = (bits >> 23) & 255
    require(exponent != 255, "nonfinite f32 input")
    significand = bits & 0x7FFFFF
    power = -149
    if exponent:
        significand |= 1 << 23
        power = exponent - 150
    if bits >> 31:
        significand = -significand
    return Fraction(significand << power) if power >= 0 else Fraction(significand, 1 << -power)


def ordered_key(bits):
    f32_fraction(bits)
    return (~bits & 0xFFFFFFFF) if bits >> 31 else bits ^ 0x80000000


def inverse_key(value):
    require(type(value) is int and 0 <= value <= 0xFFFFFFFF, "invalid ordered key")
    bits = value ^ 0x80000000 if value >> 31 else ~value & 0xFFFFFFFF
    f32_fraction(bits)
    return bits


def finite_matrix(bits):
    import numpy as np

    require(bits.ndim == 2 and bits.shape[1] == D and bits.shape[0] > 0
            and bits.dtype.kind == "u" and bits.dtype.itemsize == 4, "unsupported bit matrix")
    require(not bool(np.any(((bits >> 23) & 255) == 255)), "nonfinite f32 matrix")


def select_frontiers(bits):
    import numpy as np

    finite_matrix(bits)
    rows = bits.shape[0]
    require(rows > 2 * CURSOR_CAP + 1, "too few rows for fixed frontier ranks")
    keys = np.array(bits.T, dtype=np.uint32, order="C", copy=True)
    keys ^= np.where(keys >> 31, np.uint32(0xFFFFFFFF), np.uint32(0x80000000))
    keys.partition((CURSOR_CAP, rows - CURSOR_CAP - 1), axis=1)
    lower = [inverse_key(int(value)) for value in keys[:, CURSOR_CAP]]
    upper = [inverse_key(int(value)) for value in keys[:, rows - CURSOR_CAP - 1]]
    require(all(f32_fraction(lo) <= f32_fraction(hi) for lo, hi in zip(lower, upper)),
            "inverted selected coordinate bounds")
    return lower, upper


def gamma_bound(maximum_sum):
    require(type(maximum_sum) is float and math.isfinite(maximum_sum) and maximum_sum >= 0,
            "invalid computed squared norm")
    unit = Fraction(1, 1 << 52)
    gamma = (D - 1) * unit / (1 - (D - 1) * unit)
    return Fraction.from_float(maximum_sum) / (1 - gamma), gamma


def norm_upper_bound(bits, progress=None):
    import numpy as np

    require(sys.float_info.radix == 2 and sys.float_info.mant_dig == 53
            and sys.float_info.max_exp == 1024 and sys.float_info.min_exp == -1021
            and np.dtype(np.float64).itemsize == 8, "requires IEEE binary64")
    require(bits.ndim == 2 and bits.shape[1] == D and bits.shape[0] > 0
            and bits.dtype.kind == "u" and bits.dtype.itemsize == 4, "unsupported norm matrix")
    maximum = 0.0
    witness = 0
    with np.errstate(all="raise"):
        for start in range(0, len(bits), NORM_CHUNK_ROWS):
            if progress is not None:
                progress.update(phase="corpus_norm", row=start)
            chunk = bits[start:start + NORM_CHUNK_ROWS]
            finite_matrix(chunk)
            exponent = ((chunk >> 23) & 255).astype(np.int32)
            significand = (chunk & 0x7FFFFF) | np.where(exponent != 0, np.uint32(1 << 23), np.uint32(0))
            powers = np.where(exponent != 0, exponent - 150, -149)
            values = np.ldexp(significand.astype(np.float64), powers)
            squared = np.multiply(values, values)
            sums = squared[:, 0].copy()
            for coordinate in range(1, D):
                np.add(sums, squared[:, coordinate], out=sums)
            require(bool(np.isfinite(sums).all()) and bool((sums >= 0).all()), "invalid binary64 norm sum")
            local = int(np.argmax(sums))
            value = float(sums[local])
            if value > maximum:
                maximum = value
                witness = start + local
    upper, gamma = gamma_bound(maximum)
    witness_exact = sum((f32_fraction(int(value)) ** 2 for value in bits[witness]), Fraction())
    require(witness_exact <= upper, "computed maximum witness escapes certified bound")
    return upper, dict(maximum_computed_sum_hex=maximum.hex(), maximum_computed_sum=rational(Fraction(maximum)),
                      maximum_computed_row=witness, witness_exact_norm_squared=rational(witness_exact),
                      gamma=rational(gamma), unit_roundoff_bound=rational(Fraction(1, 1 << 52)),
                      norm_upper=rational(upper), rows=len(bits), dimension=D,
                      exact_binary64_squares=len(bits) * D, binary64_additions=len(bits) * (D - 1))


def exact_frontier(query_bits, lower_bits, upper_bits):
    require(len(query_bits) == len(lower_bits) == len(upper_bits) == D, "wrong frontier dimension")
    value = Fraction()
    squared_norm = Fraction()
    for raw_q, raw_low, raw_high in zip(query_bits, lower_bits, upper_bits):
        query = f32_fraction(int(raw_q))
        low = f32_fraction(int(raw_low))
        high = f32_fraction(int(raw_high))
        require(low <= high, "invalid coordinate bounds")
        value += query * (high if query >= 0 else low)
        squared_norm += query * query
    return value, squared_norm


def cauchy_decision(frontier, query_norm_squared, corpus_norm_upper):
    require(all(isinstance(value, Fraction) for value in (frontier, query_norm_squared, corpus_norm_upper)),
            "certificate operands must be exact fractions")
    require(query_norm_squared >= 0 and corpus_norm_upper >= 0, "negative squared norm")
    ceiling_squared = query_norm_squared * corpus_norm_upper
    margin = frontier * frontier - ceiling_squared
    rejected = frontier > 0 and margin > 0
    return dict(rejected=rejected, status="certified_reject" if rejected else "inconclusive",
                positive_frontier=frontier > 0, frontier=rational(frontier),
                query_norm_squared=rational(query_norm_squared), ceiling_squared=rational(ceiling_squared),
                squared_margin=rational(margin))


def verify_input(path, expected_bytes, expected_sha):
    require(path.is_file() and path.stat().st_size == expected_bytes, f"input byte count mismatch: {path.name}")
    require(sha256(path) == expected_sha, f"input hash mismatch: {path.name}")
    return dict(path=str(path.resolve()), bytes=expected_bytes, sha256=expected_sha)


def append_log(args, decision, outcome, failed=False):
    with args.ledger.open("a", newline="") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        csv.writer(stream, delimiter="\t").writerow([
            args.experiment, "experiment", "H-A-7", "R51-development", 2, QUESTION, PREDICTION,
            outcome, decision, "74bb9fa84b85a9bf1f58f89e5d35762f728ed17b",
            "fixed4096;Cauchy;exactFraction;gamma127;d128;all256;noGT", platform.node(),
            str(args.failure.resolve() if failed else args.output.resolve()),
        ])
        stream.flush()
        os.fsync(stream.fileno())


def run(args, progress):
    require(args.inputs.is_dir() and args.ledger.is_file(), "missing input directory or ledger")
    require(args.output.resolve() != args.failure.resolve(), "overlapping output paths")
    for path in (args.output, args.failure):
        require(not path.exists() and not path.is_symlink(), "output/failure path exists")
    own = Path(__file__).resolve().parent
    require(sha256(own / "next-card7.md") == CARD_SHA and sha256(own / "research7.json") == RESEARCH_SHA,
            "preregistered card/research changed")
    checked = json.loads(args.check_receipt.read_bytes())
    require(checked["status"] == "pass", "missing successful independent checker receipt")
    for name in ("frontier_bound_counter.py", "frontier_bound_check.py"):
        require(checked["source_sha256"][name] == sha256(own / name), "checker source mismatch")
    pins = {}
    progress["phase"] = "hash_inputs"
    for name, (size, digest) in INPUTS.items():
        pins[name] = verify_input(args.inputs / name, size, digest)
    import numpy as np

    base = np.memmap(args.inputs / "base.f32", dtype="<u4", mode="r", shape=(N, D))
    queries = np.memmap(args.inputs / "validation.f32", dtype="<u4", mode="r", shape=(QUERIES, D))
    finite_matrix(queries)
    norm_upper, norm_receipt = norm_upper_bound(base, progress)
    progress.update(phase="coordinate_order_statistics", row=None)
    lower, upper = select_frontiers(base)
    results = []
    for ordinal, query in enumerate(queries):
        progress.update(phase="query_certificates", query=ordinal)
        frontier, norm = exact_frontier(query, lower, upper)
        results.append(dict(query_ordinal=ordinal, **cauchy_decision(frontier, norm, norm_upper)))
    require(len(results) == QUERIES, "missing query certificates")
    progress.update(phase="recheck_inputs", query=None)
    for name, (size, digest) in INPUTS.items():
        verify_input(args.inputs / name, size, digest)
    rejected = sum(row["rejected"] for row in results)
    result = dict(status="complete", experiment=args.experiment, hypothesis="H-A-7", row="R51",
        claim="local", rung=2, question=QUESTION, prediction=PREDICTION,
        decision="reject" if rejected else "inconclusive", certified_rejection_queries=rejected,
        inconclusive_queries=QUERIES - rejected, feasible=None, performance_acceptance=False,
        score_claim=False, matched_gap_condition="UNPROVED", query_base_dot_products=0,
        gold_files_opened=[], official_test_read=False, max_seconds=args.max_seconds,
        rows=N, dimension=D, query_count=QUERIES, actual_cursor_cap=CURSOR_CAP,
        rank_selection=dict(ascending_zero_based=CURSOR_CAP, descending_zero_based=CURSOR_CAP,
                            high_in_ascending_order=N - CURSOR_CAP - 1),
        lower_frontier_f32_bits=[f"{value:08x}" for value in lower],
        upper_frontier_f32_bits=[f"{value:08x}" for value in upper],
        corpus_norm_certificate=norm_receipt, queries=results, inputs=pins,
        integrity_failures=0, card_sha256=CARD_SHA, research_sha256=RESEARCH_SHA,
        script_sha256=sha256(Path(__file__)), check_receipt=str(args.check_receipt.resolve()),
        check_receipt_sha256=sha256(args.check_receipt),
        environment=dict(python=sys.version, numpy=np.__version__, platform=platform.platform(),
                         float_radix=sys.float_info.radix, float_mantissa_bits=sys.float_info.mant_dig),
        limits="Exact rejection witness for the fixed4096 sorted-access threshold family only. Cauchy non-rejection is inconclusive and leaves the separately pinned-GT bound pending. No ANN search, full sorted index, scoring accuracy, timing speedup, fresh holdout or mission-row acceptance.")
    progress["phase"] = "write_result"
    write_json(args.output, result)
    summary = f"Cauchy certified_reject={rejected}/{QUERIES};inconclusive={QUERIES-rejected};0integrity_failures;noGT;no_performance_claim"
    append_log(args, "reject" if rejected else "compose", summary)
    print(f"E44 {result['decision']}: {summary}; {args.output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--check-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--failure", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--experiment", choices=("E44",), required=True)
    parser.add_argument("--max-seconds", type=int, default=180)
    args = parser.parse_args()
    progress = dict(phase="preflight", row=None, query=None)

    def interrupted(number, frame):
        if number == signal.SIGALRM:
            raise TimeoutError("E44 exceeded its registered counter cap")
        raise InterruptedError(f"E44 interrupted by signal {number}")

    try:
        require(1 <= args.max_seconds <= 180, "counter cap must be1..180seconds")
        signal.signal(signal.SIGALRM, interrupted)
        signal.signal(signal.SIGTERM, interrupted)
        signal.setitimer(signal.ITIMER_REAL, args.max_seconds)
        run(args, progress)
    except (Exception, KeyboardInterrupt) as error:
        signal.setitimer(signal.ITIMER_REAL, 0)
        failure = dict(status="failed", experiment="E44", hypothesis="H-A-7", decision=None,
            performance_acceptance=False, feasible=None, progress=progress,
            error_type=type(error).__name__, error=str(error), script_sha256=sha256(Path(__file__)),
            output_exists=args.output.exists(),
            limits="Execution/integrity failure is not a mechanism rejection or valid result; retain receipt and rerun only under a new registration.")
        try:
            write_json(args.failure, failure)
        except Exception as receipt_error:
            print(f"E44 failure receipt error: {type(receipt_error).__name__}: {receipt_error}", file=sys.stderr)
        try:
            if args.ledger.is_file():
                append_log(args, "reject", f"execution_failure,not mechanism rejection: {type(error).__name__}: {error};progress={json.dumps(progress, sort_keys=True)}", failed=True)
        except Exception as log_error:
            print(f"E44 failure ledger error: {type(log_error).__name__}: {log_error}", file=sys.stderr)
        print(f"E44 failed: {type(error).__name__}: {error}; {args.failure}", file=sys.stderr)
        return 1
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
