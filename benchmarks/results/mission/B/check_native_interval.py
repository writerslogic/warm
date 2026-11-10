# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""Fixed Fraction checks of the standalone H-B-7 native arithmetic ABI."""

import argparse
import ctypes as ct
import json
import struct
from fractions import Fraction
from pathlib import Path


OK, FALLBACK, INVALID = 0, 1, 2
DIMENSIONS = (1, 2, 99, 100, 101, 127, 128, 129)
U32P = ct.POINTER(ct.c_uint32)


class Score(ct.Structure):
    _fields_ = [("low", ct.c_uint64), ("high", ct.c_uint64)]


class Metadata(ct.Structure):
    _fields_ = [("scale_bits", ct.c_uint32), ("norm_lower_bits", ct.c_uint32),
                ("norm_upper_bits", ct.c_uint32), ("error_upper_bits", ct.c_uint32),
                ("error_squared_low", ct.c_uint64), ("error_squared_high", ct.c_uint64),
                ("code_squares", ct.c_uint32), ("code_sum", ct.c_int32),
                ("codes", ct.c_int8 * 128)]


class Decision(ct.Structure):
    _fields_ = [("prune", ct.c_uint32), ("lower_bits", ct.c_uint32),
                ("right_upper_bits", ct.c_uint32), ("threshold_upper_bits", ct.c_uint32),
                ("raw_dot", ct.c_int32), ("signed_dot", ct.c_int32)]


def score(value):
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value < 1 << 128:
        raise ValueError("requires an unsigned128 fixed-grid threshold")
    return Score(value & ((1 << 64) - 1), value >> 64)


def score_integer(value):
    return (value.high << 64) | value.low


def value(bits):
    return Fraction.from_float(struct.unpack(">f", struct.pack(">I", bits))[0])


def fbits(number):
    return struct.unpack(">I", struct.pack(">f", number))[0]


def bit_array(bits):
    if any(isinstance(bit, bool) or not isinstance(bit, int) or not 0 <= bit < 1 << 32 for bit in bits):
        raise ValueError("invalid binary32 bit encoding")
    return (ct.c_uint32 * len(bits))(*bits)


def exact_distance(data, query):
    if not data or len(data) != len(query):
        raise ValueError("invalid shape")
    return sum(((value(x) - value(q)) ** 2 for x, q in zip(data, query)), Fraction(0))


def in_domain(bits):
    return (0 < len(bits) <= 128 and all(
        (bit & 0x7FFFFFFF) == 0 or 0x37800000 <= (bit & 0x7FFFFFFF) <= 0x47800000
        for bit in bits))


def configure(library):
    library.ni_quantize.argtypes = [U32P, ct.c_size_t, ct.POINTER(Metadata)]
    library.ni_distance.argtypes = [U32P, ct.c_size_t, U32P, ct.c_size_t, ct.POINTER(Score)]
    library.ni_score_upper.argtypes = [Score, U32P]
    decide_args = [U32P, ct.c_size_t, U32P, ct.c_size_t, Score, ct.POINTER(Decision)]
    library.ni_decide.argtypes = decide_args
    library.ni_decide_error_probe.argtypes = decide_args + [ct.c_int]
    library.ni_kth.argtypes = [U32P, ct.c_size_t, ct.c_size_t, U32P, U32P, ct.c_size_t,
                              ct.c_size_t, ct.POINTER(Score), U32P, U32P]
    library.ni_set_round.argtypes = [ct.c_int]
    for name in ("ni_get_round", "ni_get_flags", "ni_clear_flags", "ni_raise_invalid",
                 "ni_metadata_size", "ni_decision_size"):
        getattr(library, name).argtypes = []
    library.ni_metadata_size.restype = ct.c_size_t
    library.ni_decision_size.restype = ct.c_size_t


def fixtures(dimension):
    zero = [0] * dimension
    one = [fbits(1.0)] * dimension
    signed = [fbits(((index % 9) - 4) * 0.125) for index in range(dimension)]
    endpoint_mix = [0x37800000, 0x37800001, 0x477FFFFF, 0x47800000]
    halfway = [fbits(number) for number in (127.0, 0.5, 1.5, 2.5, -0.5, -1.5, -2.5)]
    return (
        ("zero", zero, [0x80000000] * dimension),
        ("duplicate", signed, signed[:]),
        ("positive-dot", [fbits(2.0)] * dimension, one),
        ("negative-dot", one, [fbits(-1.0)] * dimension),
        ("norm-cancellation", [0x3F800000 + index % 4 for index in range(dimension)], one),
        ("source-and-sum-maximum", [0x47800000] * dimension, [0xC7800000] * dimension),
        ("scale-minimum", [0x37800000] * dimension, zero),
        ("quantizer-halfway", [halfway[index % len(halfway)] for index in range(dimension)], zero),
        ("endpoints", [endpoint_mix[index % 4] for index in range(dimension)], signed),
        ("below-minimum", [0x377FFFFF] * dimension, zero),
        ("above-maximum", [0x47800001] * dimension, zero),
        ("minimum-normal", [0x00800000] * dimension, zero),
        ("subnormal", [1] * dimension, zero),
        ("extreme-finite", [0x7F7FFFFF] * dimension, [0xFF7FFFFF] * dimension),
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--library", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    lib = ct.CDLL(str(args.library.resolve()))
    configure(lib)
    result = {"hypothesis": "H-B-7", "claim": "native arithmetic checks only; no performance result",
              "checks": 0, "failures": [], "native_decisions": 0, "native_prunes": 0,
              "exact_fallback_decisions": 0, "native_distances": 0, "exact_fallback_distances": 0,
              "metadata_vectors": 0, "kth_checks": 0, "rounding_modes": [],
              "environment_error_paths": 0, "positive_dots": 0, "negative_dots": 0,
              "uint128_maximum_seen": False, "invalid_rejections": 0,
              "abi_limits": {"input_dimensions": [1, 4096], "kth_rows": [1, 64],
                             "mathematical_dimensions": [1, 128],
                             "outside_resource_caps": "invalid; no fallback allocation"},
              "abi_dimension_boundary_cases": [], "abi_row_boundary_cases": []}

    def check(condition, label):
        result["checks"] += 1
        if not condition:
            result["failures"].append(label)

    def invalid(callback, label):
        check(callback() == INVALID, label)
        result["invalid_rejections"] += 1

    def meta(bits, label):
        native = Metadata()
        status = lib.ni_quantize(bit_array(bits), len(bits), ct.byref(native))
        check(status == OK, label + ": metadata status")
        if status != OK:
            raise RuntimeError(label + ": supported metadata failed")
        sources = [value(bit) for bit in bits]
        scale = value(native.scale_bits)
        maximum = max(abs(source) for source in sources)
        if maximum:
            wanted = maximum / 127
            for adjacent in (native.scale_bits - 1, native.scale_bits + 1):
                difference = abs(value(adjacent) - wanted)
                check(abs(scale - wanted) <= difference, label + ": nearest scale")
                if abs(scale - wanted) == difference:
                    check(native.scale_bits % 2 == 0, label + ": scale midpoint even")
        else:
            check(native.scale_bits == 0x00800000, label + ": zero scale")
        codes = list(native.codes)[:len(bits)]
        expected_codes = [max(-127, min(127, round(source / scale))) for source in sources]
        check(codes == expected_codes, label + ": exact quantized codes")
        check(native.code_squares == sum(code * code for code in codes), label + ": code square sum")
        check(native.code_sum == sum(codes), label + ": signed code sum")
        reconstructed = [scale * code for code in codes]
        error_squared = sum(((source - rebuilt) ** 2 for source, rebuilt in zip(sources, reconstructed)), Fraction(0))
        encoded_error = Fraction((native.error_squared_high << 64) | native.error_squared_low, 1 << 92)
        check(encoded_error == error_squared, label + ": exact uint128 reconstruction error")
        radius = value(native.error_upper_bits)
        check(radius * radius >= error_squared, label + ": error radius upper")
        if native.error_upper_bits:
            check(value(native.error_upper_bits - 1) ** 2 < error_squared, label + ": minimal error radius")
        norm = sum((entry * entry for entry in reconstructed), Fraction(0))
        lower, upper = value(native.norm_lower_bits), value(native.norm_upper_bits)
        check(lower <= norm <= upper, label + ": reconstructed norm enclosure")
        check(value(native.norm_lower_bits + 1) > norm, label + ": norm lower optimal")
        check(native.norm_upper_bits == 0 or value(native.norm_upper_bits - 1) < norm,
              label + ": norm upper optimal")
        result["metadata_vectors"] += 1
        return native, reconstructed

    def native_distance(data, query, label):
        out = Score()
        status = lib.ni_distance(bit_array(data), len(data), bit_array(query), len(query), ct.byref(out))
        oracle = exact_distance(data, query)
        if status == FALLBACK:
            check(not (in_domain(data) and in_domain(query)), label + ": explicit distance fallback")
            result["exact_fallback_distances"] += 1
            return oracle
        check(status == OK, label + ": distance status")
        exact = Fraction(score_integer(out), 1 << 78)
        check(exact == oracle, label + ": exact source distance")
        check(score_integer(out) <= 1 << 119, label + ": uint128 proved bound")
        result["uint128_maximum_seen"] |= score_integer(out) == 1 << 119
        result["native_distances"] += 1
        return exact

    def decide(data, query, threshold_integer, label, metadata_pair=None):
        threshold = Fraction(threshold_integer, 1 << 78)
        out = Decision()
        status = lib.ni_decide(bit_array(data), len(data), bit_array(query), len(query),
                               score(threshold_integer), ct.byref(out))
        exact = exact_distance(data, query)
        expected_native = in_domain(data) and in_domain(query) and threshold_integer <= 1 << 126
        if status == FALLBACK:
            check(not expected_native, label + ": explicit decision fallback")
            check(out.prune == 0, label + ": native fallback cannot prune")
            result["exact_fallback_decisions"] += 1
            return exact > threshold
        check(status == OK and expected_native, label + ": native decision status")
        if status != OK:
            raise RuntimeError(label + ": native decision failed")
        check(not out.prune or exact > threshold, label + ": no false prune")
        if exact == threshold:
            check(out.prune == 0, label + ": exact tie retained")
        dm, qm = metadata_pair if metadata_pair else (meta(data, label + "/data"), meta(query, label + "/query"))
        data_meta, data_rebuilt = dm
        query_meta, query_rebuilt = qm
        reconstructed = sum(((x - q) ** 2 for x, q in zip(data_rebuilt, query_rebuilt)), Fraction(0))
        check(value(out.lower_bits) <= reconstructed, label + ": reordered epilogue lower")
        radius_sum = value(data_meta.error_upper_bits) + value(query_meta.error_upper_bits)
        remainder = value(out.right_upper_bits) - threshold - radius_sum * radius_sum
        check(remainder >= 0 and remainder * remainder >= 4 * threshold * radius_sum * radius_sum,
              label + ": reordered epilogue RHS upper")
        check(value(out.threshold_upper_bits) >= threshold, label + ": actual threshold upper")
        dc, qc = list(data_meta.codes)[:len(data)], list(query_meta.codes)[:len(query)]
        raw = sum((x + 128) * q for x, q in zip(dc, qc))
        signed = sum(x * q for x, q in zip(dc, qc))
        check(out.raw_dot == raw and out.signed_dot == signed, label + ": exact signed correction")
        result["native_decisions"] += 1
        result["native_prunes"] += int(bool(out.prune))
        result["positive_dots"] += int(signed > 0)
        result["negative_dots"] += int(signed < 0)
        return bool(out.prune)

    try:
        check(lib.ni_metadata_size() == ct.sizeof(Metadata), "metadata ABI size")
        check(lib.ni_decision_size() == ct.sizeof(Decision), "decision ABI size")
        for dimension in DIMENSIONS:
            for name, data, query in fixtures(dimension):
                label = f"{dimension}/{name}"
                exact = native_distance(data, query, label)
                pair = None
                if in_domain(data) and in_domain(query):
                    pair = (meta(data, label + "/data"), meta(query, label + "/query"))
                grid_score = exact * (1 << 78)
                if grid_score.denominator == 1 and grid_score < (1 << 128) - 1:
                    number = int(grid_score)
                    thresholds = (number, number + 1, number // 4)
                    if number:
                        thresholds += (number - 1,)
                else:
                    thresholds = (0, 1, 1 << 126)
                for threshold in thresholds:
                    decide(data, query, threshold, label, pair)
                if not (in_domain(data) and in_domain(query)):
                    out = Decision()
                    status = lib.ni_decide(bit_array(data), len(data), bit_array(query), len(query),
                                           score(0), ct.byref(out))
                    check(status == FALLBACK and out.prune == 0, label + ": arbitrary-rational fallback handoff")
                    delta = Fraction(1, 1 << 320)
                    for threshold in (exact, exact + delta):
                        fallback_prune = exact_distance(data, query) > threshold
                        check(not fallback_prune, label + ": exact fallback tie/above")
                    if exact > 0:
                        fallback_prune = exact_distance(data, query) > exact - delta
                        check(fallback_prune, label + ": exact fallback just-below")

            zero = [0] * dimension
            candidates = [zero, [fbits(1.0)] * dimension, [fbits(1.0)] * dimension,
                          [fbits(-2.0)] * dimension]
            tags = [9, 7, 3, 1]
            distances = [exact_distance(candidate, zero) for candidate in candidates]
            wanted_order = [tag for _, tag in sorted(zip(distances, tags))]
            for k in (1, 2, len(candidates)):
                selected, upper, order = Score(), ct.c_uint32(), (ct.c_uint32 * len(tags))()
                status = lib.ni_kth(bit_array([bit for candidate in candidates for bit in candidate]),
                                    len(candidates), dimension, bit_array(tags), bit_array(zero), dimension,
                                    k, ct.byref(selected), ct.byref(upper), order)
                if dimension == 129:
                    check(status == FALLBACK, "kth unsupported dimension fallback")
                    fallback_order = sorted(zip(distances, tags))
                    check(fallback_order[k - 1][0] == sorted(distances)[k - 1], "exact Fraction kth fallback")
                    continue
                check(status == OK, "native kth status")
                check(list(order) == wanted_order, "exact source ranking and deterministic tag ties")
                exact_kth = sorted(distances)[k - 1]
                check(Fraction(score_integer(selected), 1 << 78) == exact_kth, "exact kth integer score")
                check(value(upper.value) >= exact_kth, "kth upward enclosure")
                for candidate, exact in zip(candidates, distances):
                    pruned = decide(candidate, zero, score_integer(selected), "kth-composition")
                    check(not pruned or exact > exact_kth, "kth threshold preserves candidates")
                result["kth_checks"] += 1

        for number in (0, 1, (1 << 119) - 1, 1 << 119, (1 << 119) + 1,
                       (1 << 126) - 1, 1 << 126, (1 << 126) + 1, (1 << 128) - 1):
            upper = ct.c_uint32()
            status = lib.ni_score_upper(score(number), ct.byref(upper))
            if number > 1 << 126:
                check(status == FALLBACK, "large finite threshold requests fallback")
            else:
                exact = Fraction(number, 1 << 78)
                check(status == OK and value(upper.value) >= exact, "uint128 threshold upper enclosure")
                check(upper.value == 0 or value(upper.value - 1) < exact, "minimal threshold upper")

        data, query = bit_array([fbits(2.0)] * 100), bit_array([fbits(1.0)] * 100)
        one = bit_array([0])
        for mode in range(4):
            for failure_stage in (0, 1, 2, 3):
                output = Decision()
                original = lib.ni_get_round()
                if lib.ni_clear_flags() != 0 or lib.ni_raise_invalid() != 0 or lib.ni_set_round(mode) != OK:
                    raise RuntimeError("cannot establish caller floating environment")
                before_mode, before_flags = lib.ni_get_round(), lib.ni_get_flags()
                try:
                    if failure_stage in (1, 2):
                        status = lib.ni_decide_error_probe(data, 100, query, 100, score(0), ct.byref(output), failure_stage)
                    elif failure_stage == 3:
                        status = lib.ni_decide(data, 100, one, 1, score(0), ct.byref(output))
                    else:
                        status = lib.ni_decide(data, 100, query, 100, score(0), ct.byref(output))
                    after_mode, after_flags = lib.ni_get_round(), lib.ni_get_flags()
                finally:
                    restored = lib.ni_set_round(original)
                    cleared = lib.ni_clear_flags()
                check(restored == OK and cleared == 0, "checker restores its caller environment")
                check(before_mode == mode and after_mode == mode, "native restores rounding state")
                check(before_flags != 0 and after_flags == before_flags, "native restores exception flags")
                check(status == (OK if failure_stage == 0 else INVALID), "environment probe status")
                if failure_stage:
                    check(output.prune == 0, "error cannot return a prune decision")
                    result["environment_error_paths"] += 1
            result["rounding_modes"].append(mode)

        metadata_out, distance_out, decision_out = Metadata(), Score(), Decision()
        invalid(lambda: lib.ni_quantize(None, 1, ct.byref(metadata_out)), "null input")
        invalid(lambda: lib.ni_quantize(one, 0, ct.byref(metadata_out)), "zero dimension")
        for dimension in (4095, 4096, 4097):
            data_buffer = bit_array([0] * dimension)
            query_buffer = bit_array([0] * dimension)
            upper, order = ct.c_uint32(), (ct.c_uint32 * 1)()
            expected = FALLBACK if dimension <= 4096 else INVALID
            statuses = (
                lib.ni_quantize(data_buffer, dimension, ct.byref(metadata_out)),
                lib.ni_distance(data_buffer, dimension, query_buffer, dimension, ct.byref(distance_out)),
                lib.ni_decide(data_buffer, dimension, query_buffer, dimension, score(0), ct.byref(decision_out)),
                lib.ni_kth(data_buffer, 1, dimension, bit_array([1]), query_buffer, dimension, 1,
                           ct.byref(distance_out), ct.byref(upper), order),
            )
            check(all(status == expected for status in statuses), f"allocated dimension boundary {dimension}")
            check(decision_out.prune == 0, f"dimension boundary {dimension} cannot prune")
            result["abi_dimension_boundary_cases"].append(dimension)
            if expected == INVALID:
                result["invalid_rejections"] += len(statuses)
        invalid(lambda: lib.ni_quantize(one, 1, None), "null output")
        for bad in (0x7F800000, 0xFF800000, 0x7FC00001, 0x7F800001):
            invalid(lambda bad=bad: lib.ni_quantize(bit_array([bad]), 1, ct.byref(metadata_out)), "nonfinite source")
            invalid(lambda bad=bad: lib.ni_decide(bit_array([bad]), 1, one, 1, score(0), ct.byref(decision_out)), "nonfinite decision input")
        invalid(lambda: lib.ni_distance(one, 1, query, 100, ct.byref(distance_out)), "distance dimension mismatch")
        order, upper = (ct.c_uint32 * 2)(), ct.c_uint32()
        for k in (0, 3):
            invalid(lambda k=k: lib.ni_kth(bit_array([0, 0]), 2, 1, bit_array([1, 2]), one, 1,
                                          k, ct.byref(distance_out), ct.byref(upper), order), "invalid kth index")
        invalid(lambda: lib.ni_kth(bit_array([0, 0]), 2, 1, bit_array([1, 1]), one, 1,
                                   1, ct.byref(distance_out), ct.byref(upper), order), "duplicate live tags")
        for row_count in (0, 1, 2, 63, 64, 65):
            row_bits = [fbits(float(index % 5)) for index in range(row_count)]
            row_tags = list(reversed(range(row_count)))
            rows_buffer, tags_buffer = bit_array(row_bits), bit_array(row_tags)
            order_buffer = (ct.c_uint32 * row_count)()
            status = lib.ni_kth(rows_buffer, row_count, 1, tags_buffer, one, 1,
                                max(1, row_count), ct.byref(distance_out), ct.byref(upper), order_buffer)
            if row_count in (0, 65):
                check(status == INVALID, f"allocated row boundary {row_count} invalid")
                result["invalid_rejections"] += 1
            else:
                ranked = sorted((value(bits) ** 2, tag) for bits, tag in zip(row_bits, row_tags))
                check(status == OK, f"allocated row boundary {row_count} valid")
                check(list(order_buffer) == [tag for _, tag in ranked], f"row boundary {row_count} order")
                check(Fraction(score_integer(distance_out), 1 << 78) == ranked[-1][0],
                      f"row boundary {row_count} exact kth")
                check(value(upper.value) >= ranked[-1][0], f"row boundary {row_count} threshold upper")
            result["abi_row_boundary_cases"].append(row_count)
        for bad in (-1, 1 << 128, True, "0"):
            try:
                score(bad)
            except ValueError:
                result["invalid_rejections"] += 1
            else:
                check(False, "invalid Python threshold")
        check(result["native_prunes"] > 0, "native strict pruning exercised")
        check(result["exact_fallback_decisions"] > 0 and result["exact_fallback_distances"] > 0, "exact fallback exercised")
        check(result["uint128_maximum_seen"], "exact128D maximum squared distance exercised")
        check(result["positive_dots"] > 0 and result["negative_dots"] > 0, "both signed-dot cases exercised")
        result["decision"] = "accept-native-arithmetic-only" if not result["failures"] else "reject-native-arithmetic"
    except Exception as error:
        result.update({"decision": "failed-check", "failure_type": type(error).__name__, "failure": str(error)})
        raise
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps({key: result[key] for key in ("decision", "checks", "native_prunes", "environment_error_paths", "failure_type") if key in result}))
    if result["failures"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
