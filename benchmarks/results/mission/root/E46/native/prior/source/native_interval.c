// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#include <fenv.h>
#include <float.h>
#include <math.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#pragma STDC FENV_ACCESS ON
#pragma STDC FP_CONTRACT OFF

#if !defined(__SIZEOF_INT128__)
#error H-B-7 requires checked unsigned 128-bit integer arithmetic
#endif

_Static_assert(sizeof(float) == 4 && FLT_RADIX == 2 && FLT_MANT_DIG == 24 &&
                   FLT_MAX_EXP == 128 && FLT_MIN_EXP == -125,
               "H-B-7 requires IEEE binary32 float");

typedef unsigned __int128 U128;
enum { NI_OK = 0, NI_FALLBACK = 1, NI_INVALID = 2, NI_ARITH = 3, NI_ENV = 4 };
enum { NI_MAX_DIM = 128, NI_INPUT_CAP = 4096, NI_MAX_ROWS = 64 };

typedef struct {
  uint64_t low;
  uint64_t high;
} NI_Score;

typedef struct {
  uint32_t scale_bits;
  uint32_t norm_lower_bits;
  uint32_t norm_upper_bits;
  uint32_t error_upper_bits;
  uint64_t error_squared_low;
  uint64_t error_squared_high;
  uint32_t code_squares;
  int32_t code_sum;
  int8_t codes[NI_MAX_DIM];
} NI_Metadata;

typedef struct {
  uint32_t prune;
  uint32_t lower_bits;
  uint32_t right_upper_bits;
  uint32_t threshold_upper_bits;
  int32_t raw_dot;
  int32_t signed_dot;
} NI_Decision;

static U128 unpack_score(NI_Score value) {
  return ((U128)value.high << 64) | value.low;
}

static NI_Score pack_score(U128 value) {
  return (NI_Score){(uint64_t)value, (uint64_t)(value >> 64)};
}

static uint32_t float_bits(float value) {
  uint32_t bits;
  memcpy(&bits, &value, sizeof bits);
  return bits;
}

static float bits_float(uint32_t bits) {
  float value;
  memcpy(&value, &bits, sizeof value);
  return value;
}

static void float_parts(uint32_t bits, U128 *significand, int *power) {
  uint32_t exponent = (bits >> 23) & 255;
  *significand = bits & UINT32_C(0x7fffff);
  *power = -149;
  if (exponent != 0) {
    *significand += UINT32_C(0x800000);
    *power = (int)exponent - 150;
  }
}

static int length128(U128 value) {
  uint64_t high = (uint64_t)(value >> 64);
  if (high != 0) return 128 - __builtin_clzll(high);
  uint64_t low = (uint64_t)value;
  return low == 0 ? 0 : 64 - __builtin_clzll(low);
}

static int compare_dyadic(U128 left, int lp, U128 right, int rp) {
  if (left == 0 || right == 0) return (left != 0) - (right != 0);
  int lt = length128(left) + lp, rt = length128(right) + rp;
  if (lt != rt) return lt < rt ? -1 : 1;
  // Equal leading exponents guarantee that aligning these integers fits128bits.
  if (lp > rp) left <<= (unsigned)(lp - rp);
  if (rp > lp) right <<= (unsigned)(rp - lp);
  return (left > right) - (left < right);
}

static int compare_float(uint32_t bits, U128 value, int power) {
  U128 significand;
  int fp;
  float_parts(bits, &significand, &fp);
  return compare_dyadic(significand, fp, value, power);
}

static int round_dyadic(U128 value, int power, int upward, uint32_t *out) {
  const uint32_t maximum = UINT32_C(0x7f7fffff);
  if (compare_float(maximum, value, power) < 0) return NI_ARITH;
  uint32_t low = 0, high = maximum;
  while (low < high) {
    uint32_t middle = low + (high - low + 1) / 2;
    if (compare_float(middle, value, power) <= 0) low = middle;
    else high = middle - 1;
  }
  if (upward && compare_float(low, value, power) < 0) ++low;
  *out = low;
  return NI_OK;
}

static int sqrt_dyadic_up(U128 value, int power, uint32_t *out) {
  uint32_t low = 0, high = UINT32_C(0x7f7fffff);
  U128 significand;
  int fp;
  float_parts(high, &significand, &fp);
  if (compare_dyadic(significand * significand, 2 * fp, value, power) < 0)
    return NI_ARITH;
  while (low < high) {
    uint32_t middle = low + (high - low) / 2;
    float_parts(middle, &significand, &fp);
    if (compare_dyadic(significand * significand, 2 * fp, value, power) >= 0)
      high = middle;
    else low = middle + 1;
  }
  *out = low;
  return NI_OK;
}

static int validate_vector(const uint32_t *bits, size_t count) {
  if (bits == NULL || count == 0 || count > NI_INPUT_CAP) return NI_INVALID;
  int supported = count <= NI_MAX_DIM;
  for (size_t index = 0; index < count; ++index) {
    uint32_t magnitude = bits[index] & UINT32_C(0x7fffffff);
    if (magnitude >= UINT32_C(0x7f800000)) return NI_INVALID;
    if (magnitude != 0 && (magnitude < UINT32_C(0x37800000) ||
                           magnitude > UINT32_C(0x47800000))) supported = 0;
  }
  return supported ? NI_OK : NI_FALLBACK;
}

static int grid_integer(uint32_t bits, int grid, int64_t *out) {
  U128 significand;
  int power;
  float_parts(bits, &significand, &power);
  if (significand == 0) {
    *out = 0;
    return NI_OK;
  }
  int shift = power + grid;
  if (shift < 0 || shift > 63 || length128(significand) + shift > 63)
    return NI_ARITH;
  uint64_t magnitude = (uint64_t)significand << (unsigned)shift;
  *out = (bits >> 31) != 0 ? -(int64_t)magnitude : (int64_t)magnitude;
  return NI_OK;
}

static int add_square(U128 *sum, int64_t difference) {
  uint64_t magnitude = difference < 0
      ? (uint64_t)(-(difference + 1)) + 1 : (uint64_t)difference;
  U128 square;
  if (__builtin_mul_overflow((U128)magnitude, (U128)magnitude, &square) ||
      __builtin_add_overflow(*sum, square, sum)) return NI_ARITH;
  return NI_OK;
}

static int exact_distance(const uint32_t *data, const uint32_t *query,
                          size_t count, U128 *out) {
  U128 sum = 0;
  for (size_t index = 0; index < count; ++index) {
    int64_t left, right, difference;
    if (grid_integer(data[index], 39, &left) != NI_OK ||
        grid_integer(query[index], 39, &right) != NI_OK ||
        __builtin_sub_overflow(left, right, &difference) ||
        add_square(&sum, difference) != NI_OK) return NI_ARITH;
  }
  if (sum > ((U128)1 << 119)) return NI_ARITH;
  *out = sum;
  return NI_OK;
}

static int scale_compare(uint32_t bits, U128 maximum) {
  U128 significand;
  int power;
  float_parts(bits, &significand, &power);
  return compare_dyadic(significand * 127, power, maximum, -39);
}

static int scale_nearest(U128 maximum, uint32_t *out) {
  uint32_t low = 0, high = UINT32_C(0x7f7fffff);
  while (low < high) {
    uint32_t middle = low + (high - low + 1) / 2;
    if (scale_compare(middle, maximum) <= 0) low = middle;
    else high = middle - 1;
  }
  if (scale_compare(low, maximum) == 0) {
    *out = low;
    return NI_OK;
  }
  if (low == UINT32_C(0x7f7fffff)) return NI_ARITH;
  U128 left, right;
  int lp, rp;
  float_parts(low, &left, &lp);
  float_parts(low + 1, &right, &rp);
  int common = lp < rp ? lp : rp;
  left <<= (unsigned)(lp - common);
  right <<= (unsigned)(rp - common);
  int midpoint = compare_dyadic((left + right) * 127, common, maximum * 2, -39);
  *out = midpoint > 0 || (midpoint == 0 && (low & 1) == 0) ? low : low + 1;
  return NI_OK;
}

static int metadata(const uint32_t *bits, size_t count, NI_Metadata *out) {
  memset(out, 0, sizeof *out);
  uint32_t maximum_bits = 0;
  for (size_t index = 0; index < count; ++index) {
    uint32_t magnitude = bits[index] & UINT32_C(0x7fffffff);
    if (magnitude > maximum_bits) maximum_bits = magnitude;
  }
  if (maximum_bits == 0) {
    out->scale_bits = UINT32_C(0x00800000);
    return NI_OK;
  }
  int64_t maximum, scale_grid;
  if (grid_integer(maximum_bits, 39, &maximum) != NI_OK ||
      scale_nearest((U128)maximum, &out->scale_bits) != NI_OK ||
      grid_integer(out->scale_bits, 46, &scale_grid) != NI_OK || scale_grid <= 0)
    return NI_ARITH;
  U128 error_squared = 0;
  for (size_t index = 0; index < count; ++index) {
    int64_t source, reconstructed, residual;
    if (grid_integer(bits[index], 46, &source) != NI_OK) return NI_ARITH;
    uint64_t magnitude = source < 0 ? (uint64_t)(-source) : (uint64_t)source;
    uint64_t divisor = (uint64_t)scale_grid;
    uint64_t quotient = magnitude / divisor, remainder = magnitude % divisor;
    if (remainder * 2 > divisor || (remainder * 2 == divisor && (quotient & 1)))
      ++quotient;
    if (quotient > 127) quotient = 127;
    int64_t code = source < 0 ? -(int64_t)quotient : (int64_t)quotient;
    if (__builtin_mul_overflow(scale_grid, code, &reconstructed) ||
        __builtin_sub_overflow(source, reconstructed, &residual) ||
        add_square(&error_squared, residual) != NI_OK) return NI_ARITH;
    out->codes[index] = (int8_t)code;
    out->code_sum += (int32_t)code;
    out->code_squares += (uint32_t)(code * code);
  }
  if (error_squared >= ((U128)1 << 119)) return NI_ARITH;
  NI_Score error = pack_score(error_squared);
  out->error_squared_low = error.low;
  out->error_squared_high = error.high;
  if (sqrt_dyadic_up(error_squared, -92, &out->error_upper_bits) != NI_OK)
    return NI_ARITH;
  U128 scale_mantissa, norm;
  int scale_power;
  float_parts(out->scale_bits, &scale_mantissa, &scale_power);
  if (__builtin_mul_overflow(scale_mantissa, scale_mantissa, &norm) ||
      __builtin_mul_overflow(norm, (U128)out->code_squares, &norm)) return NI_ARITH;
  if (round_dyadic(norm, 2 * scale_power, 0, &out->norm_lower_bits) != NI_OK ||
      round_dyadic(norm, 2 * scale_power, 1, &out->norm_upper_bits) != NI_OK)
    return NI_ARITH;
  return NI_OK;
}

int ni_quantize(const uint32_t *bits, size_t count, NI_Metadata *out) {
  if (out == NULL) return NI_INVALID;
  memset(out, 0, sizeof *out);
  int status = validate_vector(bits, count);
  return status == NI_OK ? metadata(bits, count, out) : status;
}

int ni_distance(const uint32_t *data, size_t count, const uint32_t *query,
                size_t query_count, NI_Score *out) {
  if (out == NULL) return NI_INVALID;
  *out = (NI_Score){0, 0};
  if (count != query_count) return NI_INVALID;
  int left = validate_vector(data, count), right = validate_vector(query, query_count);
  if (left == NI_INVALID || right == NI_INVALID) return NI_INVALID;
  if (left == NI_FALLBACK || right == NI_FALLBACK) return NI_FALLBACK;
  U128 exact;
  int status = exact_distance(data, query, count, &exact);
  if (status == NI_OK) *out = pack_score(exact);
  return status;
}

int ni_score_upper(NI_Score score, uint32_t *out) {
  if (out == NULL) return NI_INVALID;
  *out = 0;
  U128 exact = unpack_score(score);
  if (exact > ((U128)1 << 126)) return NI_FALLBACK;
  return round_dyadic(exact, -78, 1, out);
}

static int decide_impl(const uint32_t *data, size_t count, const uint32_t *query,
                       size_t query_count, NI_Score threshold, NI_Decision *out,
                       int failure_stage) {
  if (out == NULL) return NI_INVALID;
  memset(out, 0, sizeof *out);
  if (count != query_count) return NI_INVALID;
  int left = validate_vector(data, count), right = validate_vector(query, query_count);
  if (left == NI_INVALID || right == NI_INVALID) return NI_INVALID;
  if (left == NI_FALLBACK || right == NI_FALLBACK) return NI_FALLBACK;
  int status = ni_score_upper(threshold, &out->threshold_upper_bits);
  if (status != NI_OK) return status;
  NI_Metadata dm, qm;
  if ((status = metadata(data, count, &dm)) != NI_OK ||
      (status = metadata(query, count, &qm)) != NI_OK) return status;
  int32_t raw = 0;
  for (size_t index = 0; index < count; ++index)
    raw += ((int32_t)dm.codes[index] + 128) * (int32_t)qm.codes[index];
  int32_t dot = raw - 128 * qm.code_sum;
  if (raw < -4145280 || raw > 4145280 || dot < -2064512 || dot > 2064512)
    return NI_ARITH;
  out->raw_dot = raw;
  out->signed_dot = dot;
  fenv_t saved;
  if (feholdexcept(&saved) != 0) return NI_ENV;
  if (fesetenv(FE_DFL_ENV) != 0 || fesetround(FE_UPWARD) != 0) {
    status = NI_ENV;
    goto restore;
  }
  if (failure_stage == 1) {
    status = NI_INVALID;
    goto restore;
  }
  {
    volatile float df = (float)(2 * dot);
    volatile float sx = bits_float(dm.scale_bits), sq = bits_float(qm.scale_bits);
    volatile float t = dot == 0 ? 0.0f : sq * df;
    volatile float product = dot == 0 ? 0.0f : sx * t;
    volatile float upper = bits_float(out->threshold_upper_bits);
    volatile float root = sqrtf(upper);
    volatile float query_radius = root + bits_float(qm.error_upper_bits);
    volatile float radius = query_radius + bits_float(dm.error_upper_bits);
    volatile float rhs = radius * radius;
    if (fesetround(FE_DOWNWARD) != 0) {
      status = NI_ENV;
      goto restore;
    }
    if (failure_stage == 2) {
      status = NI_INVALID;
      goto restore;
    }
    volatile float norms = bits_float(dm.norm_lower_bits) + bits_float(qm.norm_lower_bits);
    volatile float lower = norms - product;
    uint32_t lb = float_bits(lower), rb = float_bits(rhs);
    if ((lb & UINT32_C(0x7fffffff)) >= UINT32_C(0x7f800000) ||
        (rb & UINT32_C(0x7fffffff)) >= UINT32_C(0x7f800000)) {
      status = NI_FALLBACK;
      goto restore;
    }
    out->lower_bits = lb;
    out->right_upper_bits = rb;
    out->prune = lower > rhs;
  }
restore:
  if (fesetenv(&saved) != 0) status = NI_ENV;
  if (status != NI_OK) out->prune = 0;
  return status;
}

int ni_decide(const uint32_t *data, size_t count, const uint32_t *query,
              size_t query_count, NI_Score threshold, NI_Decision *out) {
  return decide_impl(data, count, query, query_count, threshold, out, 0);
}

int ni_decide_error_probe(const uint32_t *data, size_t count, const uint32_t *query,
                          size_t query_count, NI_Score threshold, NI_Decision *out,
                          int failure_stage) {
  if (failure_stage != 1 && failure_stage != 2) return NI_INVALID;
  return decide_impl(data, count, query, query_count, threshold, out, failure_stage);
}

int ni_kth(const uint32_t *rows, size_t row_count, size_t dimension,
           const uint32_t *tags, const uint32_t *query, size_t query_count,
           size_t k, NI_Score *score, uint32_t *upper_bits, uint32_t *order) {
  if (rows == NULL || tags == NULL || score == NULL || upper_bits == NULL ||
      order == NULL || row_count == 0 || row_count > NI_MAX_ROWS ||
      k == 0 || k > row_count || dimension != query_count ||
      dimension == 0 || dimension > NI_INPUT_CAP) return NI_INVALID;
  *score = (NI_Score){0, 0};
  *upper_bits = 0;
  int status = validate_vector(query, query_count);
  if (status == NI_INVALID) return status;
  for (size_t row = 0; row < row_count; ++row) {
    int current = validate_vector(rows + row * dimension, dimension);
    if (current == NI_INVALID) return current;
    if (current == NI_FALLBACK) status = NI_FALLBACK;
    for (size_t other = 0; other < row; ++other)
      if (tags[other] == tags[row]) return NI_INVALID;
  }
  if (status != NI_OK) return status;
  U128 distances[NI_MAX_ROWS];
  uint32_t sorted_tags[NI_MAX_ROWS];
  for (size_t row = 0; row < row_count; ++row) {
    U128 distance;
    if (exact_distance(rows + row * dimension, query, dimension, &distance) != NI_OK)
      return NI_ARITH;
    size_t place = row;
    while (place > 0 && (distances[place - 1] > distance ||
           (distances[place - 1] == distance && sorted_tags[place - 1] > tags[row]))) {
      distances[place] = distances[place - 1];
      sorted_tags[place] = sorted_tags[place - 1];
      --place;
    }
    distances[place] = distance;
    sorted_tags[place] = tags[row];
  }
  memcpy(order, sorted_tags, row_count * sizeof *order);
  *score = pack_score(distances[k - 1]);
  return ni_score_upper(*score, upper_bits);
}

int ni_set_round(int mode) {
  const int modes[] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
  return mode < 0 || mode > 3 ? NI_INVALID : (fesetround(modes[mode]) == 0 ? NI_OK : NI_ENV);
}

int ni_get_round(void) {
  int mode = fegetround();
  if (mode == FE_TONEAREST) return 0;
  if (mode == FE_DOWNWARD) return 1;
  if (mode == FE_UPWARD) return 2;
  if (mode == FE_TOWARDZERO) return 3;
  return -1;
}

int ni_get_flags(void) { return fetestexcept(FE_ALL_EXCEPT); }
int ni_clear_flags(void) { return feclearexcept(FE_ALL_EXCEPT); }
int ni_raise_invalid(void) { return feraiseexcept(FE_INVALID); }

size_t ni_metadata_size(void) { return sizeof(NI_Metadata); }
size_t ni_decision_size(void) { return sizeof(NI_Decision); }
