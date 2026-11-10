# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""Scalar specification of binary32 interval certificates, not a native kernel."""

import math
import struct
from dataclasses import dataclass
from fractions import Fraction
from functools import lru_cache


MAX_FINITE_BITS = 0x7F7FFFFF
MIN_NORMAL_BITS = 0x00800000
MIN_COORDINATE = Fraction(1, 1 << 16)
MAX_COORDINATE = Fraction(1 << 16)
MAX_THRESHOLD = Fraction(1 << 48)


class IntervalOverflow(ArithmeticError):
    pass


@lru_cache(maxsize=32768)
def fraction_from_bits(bits):
    if not isinstance(bits, int) or isinstance(bits, bool) or not 0 <= bits <= 0xFFFFFFFF:
        raise ValueError("requires a binary32 bit pattern")
    sign = -1 if bits >> 31 else 1
    exponent = (bits >> 23) & 255
    mantissa = bits & 0x7FFFFF
    if exponent == 255:
        raise ValueError("nonfinite binary32 source")
    if exponent == 0:
        return Fraction(sign * mantissa, 1 << 149)
    significand = (1 << 23) + mantissa
    power = exponent - 150
    if power >= 0:
        return Fraction(sign * significand * (1 << power))
    return Fraction(sign * significand, 1 << -power)


def source_bits(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("source coordinates must be Python int or float binary32 values")
    try:
        binary64 = float(value)
        if not math.isfinite(binary64):
            raise ValueError("nonfinite source coordinate")
        packed = struct.pack(">f", binary64)
        if struct.unpack(">f", packed)[0] != value:
            raise ValueError("source coordinate is not exactly binary32")
        return struct.unpack(">I", packed)[0]
    except (OverflowError, struct.error) as error:
        raise ValueError("source coordinate is outside finite binary32") from error


def source_vector(values):
    result = tuple(fraction_from_bits(source_bits(value)) for value in values)
    if not result:
        raise ValueError("empty vector")
    return result


def threshold_fraction(value):
    if isinstance(value, bool) or not isinstance(value, (int, float, Fraction)):
        raise ValueError("threshold must be a finite rational number")
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError("nonfinite threshold")
    value = Fraction(value)
    if value < 0:
        raise ValueError("negative squared-distance threshold")
    return value


def _positive_floor_bits(value):
    if value < 0:
        raise ValueError("negative unsigned rounding operand")
    if value > fraction_from_bits(MAX_FINITE_BITS):
        raise IntervalOverflow("binary32 endpoint overflow")
    low, high = 0, MAX_FINITE_BITS
    while low < high:
        middle = (low + high + 1) // 2
        if fraction_from_bits(middle) <= value:
            low = middle
        else:
            high = middle - 1
    return low


def round_down(value):
    value = Fraction(value)
    if value < 0:
        return -round_up(-value)
    return fraction_from_bits(_positive_floor_bits(value))


def round_up(value):
    value = Fraction(value)
    if value < 0:
        return -round_down(-value)
    bits = _positive_floor_bits(value)
    result = fraction_from_bits(bits)
    if result < value:
        if bits == MAX_FINITE_BITS:
            raise IntervalOverflow("binary32 endpoint overflow")
        result = fraction_from_bits(bits + 1)
    return result


def round_nearest(value):
    value = Fraction(value)
    if value < 0:
        return -round_nearest(-value)
    bits = _positive_floor_bits(value)
    lower = fraction_from_bits(bits)
    if lower == value:
        return lower
    upper = fraction_from_bits(bits + 1)
    left, right = value - lower, upper - value
    return lower if left < right or (left == right and bits % 2 == 0) else upper


def sqrt_up(value):
    value = Fraction(value)
    if value < 0:
        raise ValueError("negative square-root operand")
    maximum = fraction_from_bits(MAX_FINITE_BITS)
    if value > maximum * maximum:
        raise IntervalOverflow("binary32 square-root overflow")
    low, high = 0, MAX_FINITE_BITS
    while low < high:
        middle = (low + high) // 2
        candidate = fraction_from_bits(middle)
        if candidate * candidate >= value:
            high = middle
        else:
            low = middle + 1
    return fraction_from_bits(low)


@dataclass(frozen=True)
class Interval:
    lower: Fraction
    upper: Fraction

    @staticmethod
    def enclose(value):
        return Interval(round_down(value), round_up(value))

    def add(self, other):
        return Interval(round_down(self.lower + other.lower), round_up(self.upper + other.upper))

    def subtract(self, other):
        return Interval(round_down(self.lower - other.upper), round_up(self.upper - other.lower))

    def multiply(self, other):
        endpoints = (self.lower * other.lower, self.lower * other.upper,
                     self.upper * other.lower, self.upper * other.upper)
        return Interval(round_down(min(endpoints)), round_up(max(endpoints)))

    def square(self):
        high = max(self.lower * self.lower, self.upper * self.upper)
        low = 0 if self.lower <= 0 <= self.upper else min(self.lower * self.lower, self.upper * self.upper)
        return Interval(round_down(low), round_up(high))


@dataclass(frozen=True)
class Quantized:
    scale: Fraction
    codes: tuple
    error_upper: Fraction
    norm_squared: Interval


def supported(vector):
    return (1 <= len(vector) <= 128 and
            all(value == 0 or MIN_COORDINATE <= abs(value) <= MAX_COORDINATE for value in vector))


@lru_cache(maxsize=256)
def _quantize(vector):
    if not supported(vector):
        raise ValueError("quantization outside the reference certificate domain")
    maximum = max(abs(value) for value in vector)
    scale = round_nearest(maximum / 127) if maximum else fraction_from_bits(MIN_NORMAL_BITS)
    codes = tuple(max(-127, min(127, round(value / scale))) for value in vector)
    error_squared = sum(((value - scale * code) ** 2 for value, code in zip(vector, codes)), Fraction(0))
    sum_squares = sum(code * code for code in codes)
    norm = (Interval.enclose(scale).square().multiply(Interval.enclose(sum_squares))
            if sum_squares else Interval.enclose(0))
    return Quantized(scale, codes, sqrt_up(error_squared), norm)


def quantize(values):
    return _quantize(source_vector(values))


def signed_dot_from_unsigned_data(data_codes, query_codes):
    if not data_codes or len(data_codes) != len(query_codes) or len(data_codes) > 128:
        raise ValueError("requires matching 1..128 dimensional integer codes")
    if any(isinstance(value, bool) or not isinstance(value, int) or not -127 <= value <= 127
           for value in (*data_codes, *query_codes)):
        raise ValueError("code outside symmetric int8 domain")
    raw = sum((data + 128) * query for data, query in zip(data_codes, query_codes))
    corrected = raw - 128 * sum(query_codes)
    if abs(raw) > 128 * 255 * 127 or abs(corrected) >= 1 << 24:
        raise ArithmeticError("integer accumulation or exact conversion bound violated")
    return raw, corrected


def _exact_distance(data, query):
    return sum(((left - right) ** 2 for left, right in zip(data, query)), Fraction(0))


def _distance_interval(data, query):
    result = Interval.enclose(0)
    for left, right in zip(data, query):
        result = result.add(Interval.enclose(left).subtract(Interval.enclose(right)).square())
    return result


@dataclass(frozen=True)
class Decision:
    prune: bool
    mode: str
    reconstructed_lower: Fraction | None = None
    radius_squared_upper: Fraction | None = None
    threshold_upper: Fraction | None = None
    raw_integer_dot: int | None = None
    signed_integer_dot: int | None = None


def decide(data_values, query_values, threshold):
    data, query = source_vector(data_values), source_vector(query_values)
    if len(data) != len(query):
        raise ValueError("dimension mismatch")
    threshold = threshold_fraction(threshold)
    if not supported(data) or not supported(query) or threshold > MAX_THRESHOLD:
        return Decision(_exact_distance(data, query) > threshold, "exact-fallback")
    try:
        data_q, query_q = _quantize(data), _quantize(query)
        raw, dot = signed_dot_from_unsigned_data(data_q.codes, query_q.codes)
        product = Interval.enclose(0)
        if dot:
            product = (Interval.enclose(data_q.scale).multiply(Interval.enclose(query_q.scale))
                       .multiply(Interval.enclose(dot)).multiply(Interval.enclose(2)))
        norms = data_q.norm_squared.add(query_q.norm_squared)
        lower = max(Fraction(0), norms.subtract(product).lower)
        threshold_upper = round_up(threshold)
        radius_upper = round_up(round_up(sqrt_up(threshold_upper) + data_q.error_upper) + query_q.error_upper)
        radius_squared_upper = round_up(radius_upper * radius_upper)
        return Decision(lower > radius_squared_upper, "binary32-interval", lower,
                        radius_squared_upper, threshold_upper, raw, dot)
    except IntervalOverflow:
        return Decision(_exact_distance(data, query) > threshold, "overflow-fallback")


@dataclass(frozen=True)
class Threshold:
    value: Fraction
    mode: str
    candidate_upper_bounds: tuple


def form_upper_threshold(candidate_values, query_values, k):
    query = source_vector(query_values)
    candidates = tuple(source_vector(values) for values in candidate_values)
    if isinstance(k, bool) or not isinstance(k, int) or not 1 <= k <= len(candidates):
        raise ValueError("requires 1 <= k <= candidate count")
    if any(len(candidate) != len(query) for candidate in candidates):
        raise ValueError("dimension mismatch")
    if supported(query) and all(supported(candidate) for candidate in candidates):
        try:
            bounds = tuple(_distance_interval(candidate, query).upper for candidate in candidates)
            return Threshold(sorted(bounds)[k - 1], "binary32-interval", bounds)
        except IntervalOverflow:
            mode = "overflow-fallback"
    else:
        mode = "exact-fallback"
    bounds = tuple(_exact_distance(candidate, query) for candidate in candidates)
    return Threshold(sorted(bounds)[k - 1], mode, bounds)
