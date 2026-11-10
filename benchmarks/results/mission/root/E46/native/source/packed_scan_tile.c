// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#include <float.h>
#include <immintrin.h>
#include <stddef.h>
#include <stdint.h>

#pragma STDC FENV_ACCESS ON
#pragma STDC FP_CONTRACT OFF

_Static_assert(sizeof(float) == 4 && FLT_RADIX == 2 && FLT_MANT_DIG == 24 &&
                   FLT_MAX_EXP == 128 && FLT_MIN_EXP == -125,
               "H-B-9 requires IEEE binary32");
_Static_assert(__BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__, "packed dwords require little endian");

enum { B9_OK = 0, B9_FALLBACK = 1, B9_INVALID = 2 };
enum { B9_DIM = 100, B9_ROWS = 32, B9_QUERIES = 8, B9_STEPS = 25 };
enum { B9_SCALE = 0, B9_NORM_LOWER = 1, B9_NORM_UPPER = 2, B9_ERROR_UPPER = 3 };
#define B9_ABI UINT32_C(0x42390001)
#define B9_RU (_MM_FROUND_TO_POS_INF | _MM_FROUND_NO_EXC)
#define B9_RD (_MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC)

/* Compile-only ABI, not an engine entry point. Before execution a caller must
 * establish compatibility with the entire icelake-server compilation target
 * (including CPU+OS AVX2/AVX512F/VL/VNNI support), and provide nonoverlapping, readable
 * immutable input and writable output objects of the declared sizes/alignment.
 * B9_INVALID forbids use of the output; invalid/overlapping output is not written.
 * B9_FALLBACK always supplies conservative candidate and fallback masks. Certified
 * bits assert provenance from the E36 exact metadata producer for these same
 * source vectors; numeric validation alone cannot establish that provenance or
 * that a threshold bounds the kth live source distance. The serving controller
 * owns both proofs. Uncertified/inactive entries must be canonical zero codes
 * and metadata. This gate validates every call; amortizing validation across
 * immutable encoded blocks is NOT implemented or credited as free work. */
typedef struct {
  uint32_t abi;
  uint32_t dimension;
  uint32_t live_rows;
  uint32_t certified_rows;
  uint32_t active_queries;
  uint32_t certified_queries;
  uint8_t data_codes[B9_STEPS][2][64];
  int8_t query_codes[B9_QUERIES][B9_DIM];
  uint32_t data_metadata[4][B9_ROWS];
  uint32_t query_metadata[4][B9_QUERIES];
  uint32_t threshold_upper[B9_QUERIES];
} B9_Input;

typedef struct {
  uint32_t candidate[B9_QUERIES];
  uint32_t fallback[B9_QUERIES];
  uint32_t pruned[B9_QUERIES];
} B9_Output;

typedef struct {
  float scale;
  float norm_lower;
  float radius_upper;
  int32_t correction;
} B9_Query;

_Static_assert(sizeof(B9_Input) == 4696, "B9 input ABI changed");
_Static_assert(sizeof(B9_Output) == 96 && sizeof(B9_Query) == 16, "B9 output/query ABI changed");

static __attribute__((always_inline)) inline float b9_float(uint32_t bits) {
  float value;
  __builtin_memcpy(&value, &bits, sizeof value);
  return value;
}

static __attribute__((always_inline)) inline __m512 b9_metadata(const uint32_t *bits) {
  return _mm512_castsi512_ps(_mm512_loadu_si512((const void *)bits));
}

static __attribute__((always_inline)) inline __m512i b9_query_word(const int8_t *codes) {
  int32_t word;
  __builtin_memcpy(&word, codes, sizeof word);
  return _mm512_set1_epi32(word);
}

static __attribute__((always_inline)) inline uint32_t b9_epilogue(
    __m512i raw, __m512 scale, __m512 norm, __m512 error, const B9_Query *query) {
  __m512i dot = _mm512_sub_epi32(raw, _mm512_set1_epi32(query->correction));
  __m512 df = _mm512_cvtepi32_ps(_mm512_slli_epi32(dot, 1));
  __m512 t = _mm512_mul_round_ps(_mm512_set1_ps(query->scale), df, B9_RU);
  __m512 product = _mm512_mul_round_ps(scale, t, B9_RU);
  __m512 norms = _mm512_add_round_ps(norm, _mm512_set1_ps(query->norm_lower), B9_RD);
  __m512 lower = _mm512_sub_round_ps(norms, product, B9_RD);
  __m512 radius = _mm512_add_round_ps(_mm512_set1_ps(query->radius_upper), error, B9_RU);
  __m512 right = _mm512_mul_round_ps(radius, radius, B9_RU);
  return (uint32_t)_mm512_cmp_ps_mask(lower, right, _CMP_GT_OQ);
}

/* Positive finite scales and exact integer conversion make the zero-dot
 * products positive zero, exactly matching E36's explicit zero branch. The
 * validated ranges keep every nonzero arithmetic intermediate normal and finite;
 * rounding is embedded, so this kernel does not alter the caller's MXCSR. */
static __attribute__((noinline, used)) void warm_b9_hot(
    const B9_Input *input, const B9_Query *queries, volatile uint32_t *pruned) {
#define B9_ACC(Q) __m512i a##Q##0 = _mm512_setzero_si512(), a##Q##1 = _mm512_setzero_si512()
  B9_ACC(0); B9_ACC(1); B9_ACC(2); B9_ACC(3);
  B9_ACC(4); B9_ACC(5); B9_ACC(6); B9_ACC(7);
#undef B9_ACC
#pragma clang loop unroll(disable) vectorize(disable) interleave(disable)
  for (size_t step = 0; step < B9_STEPS; ++step) {
    __m512i d0 = _mm512_loadu_si512((const void *)input->data_codes[step][0]);
    __m512i d1 = _mm512_loadu_si512((const void *)input->data_codes[step][1]);
#define B9_DOT(Q) do { \
    __m512i q = b9_query_word(input->query_codes[Q] + 4 * step); \
    a##Q##0 = _mm512_dpbusd_epi32(a##Q##0, d0, q); \
    a##Q##1 = _mm512_dpbusd_epi32(a##Q##1, d1, q); \
  } while (0)
    B9_DOT(0); B9_DOT(1); B9_DOT(2); B9_DOT(3);
    B9_DOT(4); B9_DOT(5); B9_DOT(6); B9_DOT(7);
#undef B9_DOT
  }
  __m512 s0 = b9_metadata(input->data_metadata[B9_SCALE]);
  __m512 s1 = b9_metadata(input->data_metadata[B9_SCALE] + 16);
  __m512 n0 = b9_metadata(input->data_metadata[B9_NORM_LOWER]);
  __m512 n1 = b9_metadata(input->data_metadata[B9_NORM_LOWER] + 16);
  __m512 e0 = b9_metadata(input->data_metadata[B9_ERROR_UPPER]);
  __m512 e1 = b9_metadata(input->data_metadata[B9_ERROR_UPPER] + 16);
#define B9_FINISH(Q) do { \
    uint32_t low = b9_epilogue(a##Q##0, s0, n0, e0, queries + Q); \
    uint32_t high = b9_epilogue(a##Q##1, s1, n1, e1, queries + Q); \
    pruned[Q] = low | (high << 16); \
  } while (0)
  B9_FINISH(0); B9_FINISH(1); B9_FINISH(2); B9_FINISH(3);
  B9_FINISH(4); B9_FINISH(5); B9_FINISH(6); B9_FINISH(7);
#undef B9_FINISH
}

static int b9_range(uint32_t bits, uint32_t lower, uint32_t upper, int zero_ok) {
  return (zero_ok && bits == 0) || (bits >= lower && bits <= upper);
}

static int b9_metadata_status(uint32_t scale, uint32_t lower, uint32_t upper,
                              uint32_t error, int zero_codes) {
  const uint32_t fields[4] = {scale, lower, upper, error};
  for (size_t field = 0; field < 4; ++field)
    if ((fields[field] & UINT32_C(0x7fffffff)) >= UINT32_C(0x7f800000))
      return B9_INVALID;
  if (zero_codes)
    return scale == UINT32_C(0x00800000) && lower == 0 && upper == 0 && error == 0
               ? B9_OK : B9_FALLBACK;
  if (!b9_range(scale, UINT32_C(0x33800000), UINT32_C(0x44800000), 0) ||
      !b9_range(lower, UINT32_C(0x27800000), UINT32_C(0x54000000), 0) ||
      !b9_range(upper, UINT32_C(0x27800000), UINT32_C(0x54000000), 0) ||
      !b9_range(error, UINT32_C(0x28800000), UINT32_C(0x49800000), 1) || lower > upper)
    return B9_FALLBACK;
  return B9_OK;
}

static void b9_all_candidates(B9_Output *output, uint32_t live, uint32_t active) {
  for (size_t query = 0; query < B9_QUERIES; ++query) {
    uint32_t mask = (active & (UINT32_C(1) << query)) ? live : 0;
    output->candidate[query] = mask;
    output->fallback[query] = mask;
    output->pruned[query] = 0;
  }
}

int warm_b9_scan_tile(const B9_Input *input, size_t input_bytes,
                      B9_Output *output, size_t output_bytes) {
  if (output == NULL || output_bytes != sizeof *output) return B9_INVALID;
  uintptr_t out_address = (uintptr_t)output, in_address = (uintptr_t)input;
  if (out_address % _Alignof(B9_Output) != 0 || out_address > UINTPTR_MAX - sizeof *output)
    return B9_INVALID;
  if (input == NULL || input_bytes != sizeof *input || in_address % _Alignof(B9_Input) != 0 ||
      in_address > UINTPTR_MAX - sizeof *input) {
    b9_all_candidates(output, UINT32_MAX, UINT32_C(255));
    return B9_INVALID;
  }
  if (in_address < out_address + sizeof *output && out_address < in_address + sizeof *input)
    return B9_INVALID;
  b9_all_candidates(output, UINT32_MAX, UINT32_C(255));
  if (input->abi != B9_ABI || input->dimension != B9_DIM ||
      (input->certified_rows & ~input->live_rows) != 0 ||
      (input->active_queries & ~UINT32_C(255)) != 0 ||
      (input->certified_queries & ~input->active_queries) != 0)
    return B9_INVALID;
  b9_all_candidates(output, input->live_rows, input->active_queries);
  for (size_t row = 0; row < B9_ROWS; ++row) {
    int zero_codes = 1;
    for (size_t dimension = 0; dimension < B9_DIM; ++dimension) {
      uint8_t code = input->data_codes[dimension / 4][row / 16][4 * (row % 16) + dimension % 4];
      if (code == 0) return B9_INVALID;
      zero_codes &= code == 128;
    }
    int status = b9_metadata_status(input->data_metadata[B9_SCALE][row],
        input->data_metadata[B9_NORM_LOWER][row], input->data_metadata[B9_NORM_UPPER][row],
        input->data_metadata[B9_ERROR_UPPER][row], zero_codes);
    if (status != B9_OK) return status;
    if ((input->certified_rows & (UINT32_C(1) << row)) == 0 && !zero_codes)
      return B9_INVALID;
  }
  B9_Query prepared[B9_QUERIES];
  for (size_t query = 0; query < B9_QUERIES; ++query) {
    int zero_codes = 1;
    int32_t sum = 0;
    for (size_t dimension = 0; dimension < B9_DIM; ++dimension) {
      int32_t code = input->query_codes[query][dimension];
      if (code < -127) return B9_INVALID;
      sum += code;
      zero_codes &= code == 0;
    }
    int status = b9_metadata_status(input->query_metadata[B9_SCALE][query],
        input->query_metadata[B9_NORM_LOWER][query], input->query_metadata[B9_NORM_UPPER][query],
        input->query_metadata[B9_ERROR_UPPER][query], zero_codes);
    if (status != B9_OK) return status;
    uint32_t threshold = input->threshold_upper[query];
    if ((threshold & UINT32_C(0x7fffffff)) >= UINT32_C(0x7f800000)) return B9_INVALID;
    if (!b9_range(threshold, UINT32_C(0x18800000), UINT32_C(0x54800000), 1))
      return B9_FALLBACK;
    if ((input->certified_queries & (UINT32_C(1) << query)) == 0 &&
        (!zero_codes || threshold != 0)) return B9_INVALID;
    prepared[query].scale = b9_float(input->query_metadata[B9_SCALE][query]);
    prepared[query].norm_lower = b9_float(input->query_metadata[B9_NORM_LOWER][query]);
    prepared[query].correction = 128 * sum;
    __m128 upper = _mm_set_ss(b9_float(threshold));
    __m128 root = _mm_sqrt_round_ss(upper, upper, B9_RU);
    __m128 error = _mm_set_ss(b9_float(input->query_metadata[B9_ERROR_UPPER][query]));
    prepared[query].radius_upper = _mm_cvtss_f32(_mm_add_round_ss(root, error, B9_RU));
  }
  uint32_t raw_pruned[B9_QUERIES];
  warm_b9_hot(input, prepared, raw_pruned);
  int status = B9_OK;
  for (size_t query = 0; query < B9_QUERIES; ++query) {
    uint32_t live = (input->active_queries & (UINT32_C(1) << query)) ? input->live_rows : 0;
    uint32_t certified = (input->certified_queries & (UINT32_C(1) << query))
                             ? input->certified_rows : 0;
    output->pruned[query] = raw_pruned[query] & certified & live;
    output->candidate[query] = live & ~output->pruned[query];
    output->fallback[query] = live & ~certified;
    if (output->fallback[query] != 0) status = B9_FALLBACK;
  }
  return status;
}
