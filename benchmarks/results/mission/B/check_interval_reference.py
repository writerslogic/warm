# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""Fixed independent rational checks for the H-B-6 scalar specification."""

import argparse
import hashlib
import json
import math
import struct
import time
from fractions import Fraction
from pathlib import Path

import interval_reference as reference


DIMENSIONS = (99, 100, 101, 127, 128, 129)
SECONDS_CAP = 180


def float_from_bits(bits):
    return struct.unpack(">f", struct.pack(">I", bits))[0]


def bits_from_float(value):
    return struct.unpack(">I", struct.pack(">f", float(value)))[0]


def oracle_value(value):
    return Fraction.from_float(float(value))


def oracle_distance(data, query):
    return sum(((oracle_value(left) - oracle_value(right)) ** 2
                for left, right in zip(data, query)), Fraction(0))


def neighbor(value, upward):
    binary64 = float(value)
    if binary64 == 0:
        return float_from_bits(1 if upward else 0x80000001)
    bits = bits_from_float(binary64)
    change = 1 if (binary64 > 0) == upward else -1
    return float_from_bits(bits + change)


def fixtures(dimension):
    zero = [0.0] * dimension
    one = [1.0] * dimension
    negative_one = [-1.0] * dimension
    signed = [((index % 9) - 4) * 0.125 for index in range(dimension)]
    small = [(-1 if index % 2 else 1) * 2.0 ** -16 for index in range(dimension)]
    large = [(-1 if index % 2 else 1) * 2.0 ** 16 for index in range(dimension)]
    almost_one = [float_from_bits(0x3F800000 + index % 4) for index in range(dimension)]
    return (
        ("zero", zero, [-0.0] * dimension, True),
        ("duplicate", signed, signed[:], True),
        ("positive-dot", [2.0] * dimension, one, True),
        ("negative-dot", one, negative_one, True),
        ("zero-dot", one, zero, True),
        ("norm-cancellation", almost_one, one, True),
        ("minimum-supported", small, zero, True),
        ("maximum-supported", large, small, True),
        ("below-supported", [float_from_bits(0x377FFFFF)] * dimension, zero, False),
        ("above-supported", [float_from_bits(0x47800001)] * dimension, zero, False),
        ("minimum-normal", [float_from_bits(0x00800000)] * dimension, zero, False),
        ("subnormal", [float_from_bits(1)] * dimension, zero, False),
        ("extreme-finite", [float_from_bits(0x7F7FFFFF)] * dimension,
         [-float_from_bits(0x7F7FFFFF)] * dimension, False),
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    started = time.monotonic()
    result = {
        "hypothesis": "H-B-6",
        "claim": "scalar exact-rational interval reference only; no native or performance claim",
        "dimensions": list(DIMENSIONS),
        "seconds_cap": SECONDS_CAP,
        "checks": 0,
        "failures": [],
        "decision_modes": {},
        "supported_prunes": 0,
        "positive_dots": 0,
        "negative_dots": 0,
        "threshold_checks": 0,
        "invalid_inputs_rejected": 0,
    }

    def check(condition, description):
        if time.monotonic() - started > SECONDS_CAP:
            raise TimeoutError("180-second fixed correctness-check cap exceeded")
        result["checks"] += 1
        if not condition:
            result["failures"].append(description)

    def rejects(callback, description, exception=ValueError):
        try:
            callback()
        except exception:
            result["invalid_inputs_rejected"] += 1
            check(True, description)
        else:
            check(False, description)

    def check_binary32(value, label):
        decoded = oracle_value(struct.unpack(">f", struct.pack(">f", float(value)))[0])
        check(value == decoded, label + ": endpoint exactly binary32")

    def check_metadata(values, label):
        quantized = reference.quantize(values)
        scale = oracle_value(float(quantized.scale))
        reconstructed = [scale * code for code in quantized.codes]
        true_norm = sum((value * value for value in reconstructed), Fraction(0))
        true_error = sum(((oracle_value(value) - rebuilt) ** 2
                          for value, rebuilt in zip(values, reconstructed)), Fraction(0))
        check(quantized.norm_squared.lower <= true_norm <= quantized.norm_squared.upper,
              label + ": reconstructed norm enclosure")
        check(quantized.error_upper >= 0 and quantized.error_upper ** 2 >= true_error,
              label + ": reconstruction error radius")
        check_binary32(quantized.scale, label + ": scale")
        check_binary32(quantized.error_upper, label + ": error radius")
        return quantized, reconstructed

    try:
        exact_values = [Fraction(0), Fraction(1, 1 << 200), Fraction(1, 1 << 150),
                        Fraction(1, 1 << 149), Fraction(1, 1 << 126),
                        Fraction(1) + Fraction(1, 1 << 24),
                        Fraction(1) + Fraction(3, 1 << 24),
                        Fraction(2) - Fraction(1, 1 << 25),
                        oracle_value(float_from_bits(0x7F7FFFFF))]
        for value in exact_values + [-value for value in exact_values]:
            low, high = reference.round_down(value), reference.round_up(value)
            check(low <= value <= high, "directed rounding enclosure")
            check_binary32(low, "downward rounding")
            check_binary32(high, "upward rounding")
            next_low, previous_high = neighbor(low, True), neighbor(high, False)
            check(not math.isfinite(next_low) or oracle_value(next_low) > value,
                  "downward rounding chooses greatest endpoint")
            check(not math.isfinite(previous_high) or oracle_value(previous_high) < value,
                  "upward rounding chooses least endpoint")
            nearest = reference.round_nearest(value)
            distances = (abs(value - low), abs(high - value))
            check(nearest in (low, high) and abs(nearest - value) == min(distances),
                  "nearest rounding chooses closest endpoint")
            if distances[0] == distances[1] and low != high:
                check(bits_from_float(nearest) % 2 == 0, "nearest midpoint chooses even encoding")
        for value in exact_values:
            upper = reference.sqrt_up(value)
            check(upper >= 0 and upper * upper >= value, "sqrt upper enclosure")
            previous = neighbor(upper, False)
            check(upper == 0 or oracle_value(previous) ** 2 < value, "sqrt upper is minimal endpoint")
        for left, right in ((-3, 5), (0, 0), (1, 2), (-8, -1)):
            interval = reference.Interval(Fraction(left), Fraction(right))
            for other_left, other_right in ((-7, -2), (-1, 4), (0, 0), (2, 9)):
                other = reference.Interval(Fraction(other_left), Fraction(other_right))
                product = interval.multiply(other)
                products = [a * b for a in (left, right) for b in (other_left, other_right)]
                check(product.lower <= min(products) and product.upper >= max(products),
                      "sign-aware interval product")
            square = interval.square()
            minimum = 0 if left <= 0 <= right else min(left * left, right * right)
            check(square.lower <= minimum and square.upper >= max(left * left, right * right),
                  "interval square crossing zero")

        delta = Fraction(1, 1 << 320)
        for dimension in DIMENSIONS:
            for name, data, query, range_supported in fixtures(dimension):
                label = f"{dimension}/{name}"
                exact = oracle_distance(data, query)
                in_domain = range_supported and dimension <= 128
                metadata = None
                if in_domain:
                    metadata = (check_metadata(data, label + "/data"),
                                check_metadata(query, label + "/query"))
                thresholds = [exact, exact + delta, exact / 4]
                if exact:
                    thresholds.append(exact - delta)
                else:
                    rejects(lambda: reference.decide(data, query, -delta), label + ": negative threshold")
                for threshold in thresholds:
                    decision = reference.decide(data, query, threshold)
                    modes = result["decision_modes"]
                    modes[decision.mode] = modes.get(decision.mode, 0) + 1
                    expected_interval = in_domain and threshold <= reference.MAX_THRESHOLD
                    check((decision.mode == "binary32-interval") == expected_interval,
                          label + ": explicit domain/fallback")
                    check(not decision.prune or exact > threshold, label + ": false prune")
                    if exact == threshold:
                        check(not decision.prune, label + ": exact tie retained")
                    if decision.mode != "binary32-interval":
                        check(decision.prune == (exact > threshold), label + ": fallback exact decision")
                        continue
                    (data_q, data_rebuilt), (query_q, query_rebuilt) = metadata
                    reconstructed_distance = sum(((x - q) ** 2 for x, q in zip(data_rebuilt, query_rebuilt)), Fraction(0))
                    check(0 <= decision.reconstructed_lower <= reconstructed_distance,
                          label + ": lower reconstructed-distance enclosure")
                    error_sum = data_q.error_upper + query_q.error_upper
                    remainder = decision.radius_squared_upper - threshold - error_sum ** 2
                    check(remainder >= 0 and remainder ** 2 >= 4 * threshold * error_sum ** 2,
                          label + ": sqrt-free RHS upper enclosure")
                    check(decision.threshold_upper >= threshold, label + ": threshold rounded upward")
                    signed_dot = sum(x * q for x, q in zip(data_q.codes, query_q.codes))
                    raw_dot = sum((x + 128) * q for x, q in zip(data_q.codes, query_q.codes))
                    check(decision.signed_integer_dot == signed_dot and decision.raw_integer_dot == raw_dot,
                          label + ": signed correction")
                    result["positive_dots"] += int(signed_dot > 0)
                    result["negative_dots"] += int(signed_dot < 0)
                    result["supported_prunes"] += int(decision.prune)

            zero = [0.0] * dimension
            candidate_sets = (
                ([zero, [1.0] * dimension, [1.0] * dimension, [-2.0] * dimension], zero, dimension <= 128),
                ([[float_from_bits(1)] * dimension, zero, zero], zero, False),
                ([[float_from_bits(0x7F7FFFFF)] * dimension, zero, zero], zero, False),
            )
            for candidates, query, in_domain in candidate_sets:
                distances = [oracle_distance(candidate, query) for candidate in candidates]
                for k in (1, 2, len(candidates)):
                    upper = reference.form_upper_threshold(candidates, query, k)
                    check(upper.value >= sorted(distances)[k - 1], "kth upper bound")
                    check(all(bound >= distance for bound, distance in zip(upper.candidate_upper_bounds, distances)),
                          "individual incumbent distances enclosed")
                    check((upper.mode == "binary32-interval") == in_domain, "threshold fallback mode")
                    for candidate, exact in zip(candidates, distances):
                        decision = reference.decide(candidate, query, upper.value)
                        check(not decision.prune or exact > sorted(distances)[k - 1],
                              "threshold formation composed with certificate")
                    result["threshold_checks"] += 1

        zero100 = [0.0] * 100
        for threshold in (Fraction(1, 1 << 400), reference.MAX_THRESHOLD, reference.MAX_THRESHOLD + 1):
            decision = reference.decide(zero100, zero100, threshold)
            check(not decision.prune, "threshold magnitude boundary retains zero")
            check((decision.mode == "binary32-interval") == (threshold <= reference.MAX_THRESHOLD),
                  "threshold magnitude explicit fallback")
        for dimension in (99, 100, 101, 127, 128):
            for sign in (-1, 1):
                data_codes, query_codes = [127] * dimension, [127 * sign] * dimension
                raw, dot = reference.signed_dot_from_unsigned_data(data_codes, query_codes)
                check(raw == dimension * 255 * 127 * sign and dot == dimension * 127 * 127 * sign,
                      "integer extremum and unsigned correction")
                if dimension == 100:
                    padded = reference.signed_dot_from_unsigned_data(data_codes + [0] * 28, query_codes + [0] * 28)
                    check(padded == (raw, dot), "zero-query padding preserves corrected dot")
        rejects(lambda: reference.decide([], [], 0), "empty vectors")
        rejects(lambda: reference.decide([0.0], [0.0, 0.0], 0), "dimension mismatch")
        for invalid in (float("nan"), float("inf"), -float("inf"), 0.1, True, "0"):
            rejects(lambda invalid=invalid: reference.decide([invalid], [0.0], 0), "invalid source coordinate")
        for invalid in (-1, float("nan"), float("inf"), True, "0"):
            rejects(lambda invalid=invalid: reference.decide([0.0], [0.0], invalid), "invalid threshold")
        for k in (0, 2, True):
            rejects(lambda k=k: reference.form_upper_threshold([[0.0]], [0.0], k), "invalid k")
        rejects(lambda: reference.form_upper_threshold([[0.0, 0.0]], [0.0], 1), "candidate dimension mismatch")
        rejects(lambda: reference.signed_dot_from_unsigned_data([128], [1]), "invalid integer code")
        rejects(lambda: reference.signed_dot_from_unsigned_data([1] * 129, [1] * 129), "integer dimension overflow")
        maximum = oracle_value(float_from_bits(0x7F7FFFFF))
        rejects(lambda: reference.round_up(maximum * 2), "rounding overflow", reference.IntervalOverflow)
        rejects(lambda: reference.sqrt_up(maximum * maximum * 2), "sqrt overflow", reference.IntervalOverflow)
        check(result["supported_prunes"] > 0, "at least one supported strict pruning case")
        check(result["positive_dots"] > 0 and result["negative_dots"] > 0, "both product signs exercised")
        check(result["decision_modes"].get("exact-fallback", 0) > 0, "fallback exercised")
        result["decision"] = "accept-reference-only" if not result["failures"] else "reject-reference"
    except Exception as error:
        result.update({"decision": "failed-check", "failure_type": type(error).__name__, "failure": str(error)})
        raise
    finally:
        result["check_wall_seconds_not_benchmark"] = time.monotonic() - started
        directory = Path(__file__).resolve().parent
        result["source_sha256"] = {
            name: hashlib.sha256((directory / name).read_bytes()).hexdigest()
            for name in ("interval_reference.py", "check_interval_reference.py", "next-card-b6.md")
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps({key: result[key] for key in ("decision", "checks", "supported_prunes", "failure_type") if key in result}))
    if result["failures"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
