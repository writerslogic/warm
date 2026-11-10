# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""H-B-8 fixed numerical counter with exact source refinement and independent audit."""

import argparse
import ctypes as ct
import hashlib
import json
import math
import struct
import time
from fractions import Fraction
from pathlib import Path

import numpy as np

from check_native_interval import Metadata, Score, configure, score_integer
from interval_reference import round_down, round_nearest, round_up, sqrt_up


SEEDS, BLOCK, QUERIES, AUDIT_CAP = 4096, 4096, 16, 65536
U32P = ct.POINTER(ct.c_uint32)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True, type=Path)
    parser.add_argument("--queries", required=True, type=Path)
    parser.add_argument("--native-receipt", required=True, type=Path)
    parser.add_argument("--distribution", required=True, choices=("easy", "hard"))
    parser.add_argument("--easy-result", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit("preserve existing evidence; output must be new")
    started = time.monotonic()
    result = {"hypothesis": "H-B-8", "rung": 2, "distribution": args.distribution,
              "claim": "necessary numerical count gate; no native serving or throughput claim"}
    stage = "input"
    counts = {"native_metadata_vectors": 0, "fraction_metadata_vectors": 0,
              "native_exact_refinements": 0, "fraction_exact_refinements": 0,
              "audit_fraction_source_distances": 0, "seed_square_roots": 0,
              "numerical_scan_square_roots": 0,
              "seed_dot_pairs": 0, "scan_dot_pairs": 0,
              "signed_correction_mismatches": 0, "bound_violations": 0,
              "seed_upper_violations": 0, "false_prunes": 0,
              "refinement_parity_mismatches": 0, "final_top10_mismatches": 0,
              "source_interval_violations": 0,
              "unresolved_final_seed_bounds": 0}

    def deadline():
        if time.monotonic() - started > 600:
            raise TimeoutError("600-second fixed counter cap exceeded")

    def digest(path):
        h = hashlib.sha256()
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                deadline()
                h.update(block)
        return h.hexdigest()

    def field(bits):
        return struct.unpack("=f", struct.pack("=I", bits))[0]

    def up32(values):
        with np.errstate(over="raise", invalid="raise"):
            return np.nextafter(np.asarray(values, dtype=np.float32), np.float32(np.inf)).astype(np.float64)

    def down32(values):
        with np.errstate(over="raise", invalid="raise"):
            return np.nextafter(np.asarray(values, dtype=np.float32), np.float32(-np.inf)).astype(np.float64)

    def numerical_quantize(rows):
        scales = np.maximum((np.max(np.abs(rows), axis=1) / 127).astype(np.float32), np.finfo(np.float32).tiny).astype(np.float64)
        codes = np.clip(np.rint(rows / scales[:, None]), -127, 127).astype(np.int8)
        rebuilt = codes * scales[:, None]
        errors = up32(np.sqrt(np.square(rows - rebuilt).sum(axis=1)))
        norms = np.square(rebuilt).sum(axis=1)
        return codes, scales, errors, norms, up32(norms)

    def exact_fraction_distance(row, query):
        return sum(((Fraction.from_float(float(left)) - Fraction.from_float(float(right))) ** 2
                    for left, right in zip(row, query)), Fraction(0))

    def ptr(row):
        if row.dtype != np.float32 or not row.flags.c_contiguous:
            raise ValueError("native input must be contiguous float32")
        return row.view(np.uint32).ctypes.data_as(U32P)

    def native_metadata(row):
        deadline()
        out = Metadata()
        status = lib.ni_quantize(ptr(row), len(row), ct.byref(out))
        if status == 0:
            counts["native_metadata_vectors"] += 1
            return (np.ctypeslib.as_array(out.codes)[:len(row)].copy(), field(out.scale_bits),
                    field(out.error_upper_bits), field(out.norm_lower_bits), field(out.norm_upper_bits))
        if status != 1:
            raise RuntimeError(f"native metadata failed with status {status}")
        counts["fraction_metadata_vectors"] += 1
        values = [Fraction.from_float(float(value)) for value in row]
        scale = max(round_nearest(max(abs(value) for value in values) / 127), Fraction(1, 1 << 126))
        code = [max(-127, min(127, round(value / scale))) for value in values]
        reconstructed = [scale * value for value in code]
        error2 = sum(((value - rebuilt) ** 2 for value, rebuilt in zip(values, reconstructed)), Fraction(0))
        norm2 = sum((value * value for value in reconstructed), Fraction(0))
        return (np.asarray(code, dtype=np.int8), float(scale), float(sqrt_up(error2)),
                float(round_down(norm2)), float(round_up(norm2)))

    def exact_refinement(row, query):
        deadline()
        out = Score()
        status = lib.ni_distance(ptr(row), len(row), ptr(query), len(query), ct.byref(out))
        if status == 0:
            counts["native_exact_refinements"] += 1
            return Fraction(score_integer(out), 1 << 78)
        if status != 1:
            raise RuntimeError(f"native source distance failed with status {status}")
        counts["fraction_exact_refinements"] += 1
        return exact_fraction_distance(row, query)

    def source_intervals(computed, dimension):
        unit = np.float64(2.0 ** -53)
        term = np.float64(dimension + 2) * unit
        gamma = np.nextafter(term / (1 - term), np.inf)
        plus = np.nextafter(1 + gamma, np.inf)
        minus = np.nextafter(1 - gamma, -np.inf)
        lower = np.maximum(0, np.nextafter(computed / plus, -np.inf))
        upper = np.nextafter(computed / minus, np.inf)
        return lower, upper

    def summary(values):
        return {"mean": float(np.mean(values)), "p99": float(np.quantile(values, .99)),
                "min": float(np.min(values)), "max": float(np.max(values))} if values else None

    try:
        directory = Path(__file__).resolve().parent
        identity_names = ("upper_seed_counter.py", "next-card-b8.md", "b8-cost.json",
                          "check_native_interval.py", "interval_reference.py", "native_interval.c")
        result["source_sha256"] = {name: digest(directory / name) for name in identity_names}
        if args.distribution == "hard":
            if args.easy_result is None:
                raise ValueError("hard transfer requires the accepted easy result")
            easy = json.loads(args.easy_result.read_text())
            if (easy.get("hypothesis") != "H-B-8" or easy.get("distribution") != "easy"
                    or easy.get("decision") != "accept-next-rung"
                    or easy.get("source_sha256") != result["source_sha256"]):
                raise ValueError("hard transfer requires unchanged sources and accepted easy gate")
            result["easy_result_sha256"] = digest(args.easy_result)
        receipt = json.loads(args.native_receipt.read_text())
        if (receipt.get("hypothesis") != "H-B-7" or receipt.get("decision") != "accept-native-arithmetic-only"
                or receipt.get("source_sha256", {}).get("native_interval.c") != result["source_sha256"]["native_interval.c"]):
            raise ValueError("requires accepted E36-compatible native source identity")
        library = Path(receipt["library"])
        if digest(library) != receipt["artifact_sha256"].get(library.name):
            raise ValueError("native shared-library hash differs from accepted receipt")
        lib = ct.CDLL(str(library.resolve()))
        configure(lib)
        if lib.ni_get_round() != 0 or lib.ni_metadata_size() != ct.sizeof(Metadata):
            raise ValueError("requires round-to-nearest caller and matching native ABI")
        precision = np.finfo(np.float64)
        if precision.nmant != 52 or precision.minexp != -1022 or precision.maxexp != 1024:
            raise ValueError("independent source interval proof requires IEEE binary64")
        result["numpy_version"] = np.__version__
        result["fixed_configuration"] = {"seeds": SEEDS, "block_rows": BLOCK, "query_prefix": QUERIES,
                                         "exact_or_ambiguous_row_guard": AUDIT_CAP}
        result["native_receipt"] = {"path": str(args.native_receipt.resolve()), "sha256": digest(args.native_receipt),
                                    "library_sha256": digest(library)}
        base = np.load(args.base, mmap_mode="r", allow_pickle=False)
        queries = np.load(args.queries, mmap_mode="r", allow_pickle=False)
        if (base.ndim != 2 or queries.ndim != 2 or base.shape[1] != queries.shape[1]
                or not SEEDS <= len(base) <= 1_000_000 or len(queries) < QUERIES
                or not 1 <= base.shape[1] <= 128 or base.dtype != np.float32 or queries.dtype != np.float32):
            raise ValueError("requires4096..1M contiguous float32 base rows,1..128D,and16 matching queries")
        n, d = base.shape
        result["base"] = {"path": str(args.base.resolve()), "sha256": digest(args.base), "rows": n, "dimension": d}
        result["queries"] = {"path": str(args.queries.resolve()), "sha256": digest(args.queries), "prefix_count": QUERIES}
        codes = np.empty((n, d), dtype=np.int8)
        scales, errors, norms, norm_uppers = (np.empty(n, dtype=np.float64) for _ in range(4))
        max_norm = 0.0
        stage = "numerical-full-source-metadata"
        for start in range(0, n, BLOCK):
            deadline()
            rows = np.asarray(base[start:start + BLOCK], dtype=np.float64)
            if not np.isfinite(rows).all():
                raise ValueError("nonfinite source coordinate")
            stop = start + len(rows)
            codes[start:stop], scales[start:stop], errors[start:stop], norms[start:stop], norm_uppers[start:stop] = numerical_quantize(rows)
            max_norm = max(max_norm, float(np.sqrt(np.square(rows).sum(axis=1)).max()))
        stage = "fixed-data-only-seed-selection"
        hashes = np.arange(n, dtype=np.uint64) + np.uint64(0x9E3779B97F4A7C15)
        hashes = (hashes ^ (hashes >> 30)) * np.uint64(0xBF58476D1CE4E5B9)
        hashes = (hashes ^ (hashes >> 27)) * np.uint64(0x94D049BB133111EB)
        hashes ^= hashes >> 31
        seed_ids = np.sort(np.argpartition(hashes, SEEDS - 1)[:SEEDS])
        del hashes
        seed_set = set(int(index) for index in seed_ids)
        seed_codes = codes[seed_ids].copy()
        seed_scales, seed_errors, seed_norm_upper = (np.empty(SEEDS, dtype=np.float64) for _ in range(3))
        for place, index in enumerate(seed_ids):
            native_code, scale, error, _, upper = native_metadata(np.asarray(base[index]))
            if not np.array_equal(native_code, seed_codes[place]) or scale != scales[index]:
                raise ValueError("numerical quantization differs from proved seed metadata")
            seed_scales[place], seed_errors[place], seed_norm_upper[place] = scale, error, upper
            norm_uppers[index] = upper
        refinements, seed_refinements, initial_final, seed_ratio_low, seed_ratio_high = [], [], [], [], []
        oracle_candidates, zero_thresholds = [], 0
        for raw_query in queries[:QUERIES]:
            stage = "upper-seed-and-exact-refinement"
            deadline()
            query = np.asarray(raw_query)
            if not np.isfinite(query).all():
                raise ValueError("nonfinite query coordinate")
            q64 = query.astype(np.float64)
            qc, qs, qe, _, qnu = native_metadata(query)
            approximate_query = numerical_quantize(q64[None, :])
            if not np.array_equal(qc, approximate_query[0][0]) or qs != approximate_query[1][0]:
                raise ValueError("numerical query quantization differs from proved metadata")
            qnorm = approximate_query[3][0]
            qcode = qc.astype(np.int64)
            correction = 128 * qcode.sum()
            seed_signed = seed_codes.astype(np.int64)
            seed_dot = (seed_signed + 128) @ qcode - correction
            counts["signed_correction_mismatches"] += int((seed_dot != seed_signed @ qcode).sum())
            product_lower = down32(seed_scales * down32(qs * (2 * seed_dot)))
            product_lower[seed_dot == 0] = 0
            reconstructed_upper = up32(up32(seed_norm_upper + qnu) - product_lower)
            seed_root = up32(np.sqrt(np.maximum(0, reconstructed_upper)))
            seed_radius = up32(up32(seed_root + seed_errors) + qe)
            seed_upper = up32(seed_radius * seed_radius)
            if not np.isfinite(seed_upper).all():
                raise ValueError("nonfinite seed upper bound")
            counts["seed_square_roots"] += SEEDS
            counts["seed_dot_pairs"] += SEEDS
            chosen = np.lexsort((seed_ids, seed_upper))[:10]
            best = [(Fraction.from_float(float(seed_upper[index])), int(seed_ids[index]), False) for index in chosen]
            initial_upper = best[-1][0]
            margin = 256 * np.finfo(np.float64).eps * (d + 1) * max(1.0, (max_norm + float(np.linalg.norm(q64))) ** 2)
            lower_all = np.empty(n, dtype=np.float64)
            pruned = np.zeros(n, dtype=bool)
            refined = {}
            for start in range(0, n, BLOCK):
                deadline()
                stop = min(start + BLOCK, n)
                signed = codes[start:stop].astype(np.int64)
                dot = (signed + 128) @ qcode - correction
                counts["signed_correction_mismatches"] += int((dot != signed @ qcode).sum())
                counts["scan_dot_pairs"] += stop - start
                counts["numerical_scan_square_roots"] += stop - start
                rebuilt2 = norms[start:stop] + qnorm - 2 * scales[start:stop] * qs * dot
                raw_lower = np.square(np.maximum(np.sqrt(np.maximum(rebuilt2, 0)) - errors[start:stop] - qe, 0))
                lower = np.maximum(0, raw_lower - margin)
                if not np.isfinite(lower).all():
                    raise ValueError("nonfinite numerical scan lower bound")
                lower_all[start:stop] = lower
                threshold_upper = math.nextafter(float(best[-1][0]), math.inf)
                keep = lower <= threshold_upper
                pruned[start:stop] = ~keep
                admitted = start + np.flatnonzero(keep)
                if len(refined) + len(admitted) > AUDIT_CAP:
                    result["count_rejection_lower_bound"] = {"one_query_exact_rows_exceeds": AUDIT_CAP,
                                                              "sample_p99_lower_bound": .85 * AUDIT_CAP}
                    raise RuntimeError("refinement resource guard exceeded;16-query p99 necessarily fails8192")
                new = []
                for index in admitted:
                    index = int(index)
                    exact = exact_refinement(np.asarray(base[index]), query)
                    refined[index] = exact
                    new.append((exact, index, True))
                admitted_set = set(int(index) for index in admitted)
                best = sorted([entry for entry in best if entry[1] not in admitted_set] + new,
                              key=lambda entry: (entry[0], entry[1]))[:10]
            counts["unresolved_final_seed_bounds"] += sum(not entry[2] for entry in best)
            if any(not entry[2] for entry in best):
                raise RuntimeError("unresolved upper-only seed remains after complete scan")
            stage = "independent-retrospective-oracle-and-audit"
            computed = np.empty(n, dtype=np.float64)
            for start in range(0, n, BLOCK):
                deadline()
                rows = np.asarray(base[start:start + BLOCK], dtype=np.float64)
                differences = rows - q64
                computed[start:start + len(rows)] = np.square(differences).sum(axis=1)
            if not np.isfinite(computed).all() or np.any(computed < 0):
                raise ValueError("invalid independent source scores")
            truth_lower, truth_upper = source_intervals(computed, d)
            cutoff = np.partition(truth_upper, 9)[9]
            candidates = np.flatnonzero(truth_lower <= cutoff)
            if len(candidates) > AUDIT_CAP:
                raise RuntimeError("independent exact oracle ambiguity exceeds fixed resource guard")
            audited = {}

            def audit(index):
                index = int(index)
                if index not in audited:
                    deadline()
                    audited[index] = exact_fraction_distance(base[index], query)
                    counts["audit_fraction_source_distances"] += 1
                    counts["source_interval_violations"] += int(not (
                        Fraction.from_float(float(truth_lower[index])) <= audited[index]
                        <= Fraction.from_float(float(truth_upper[index]))))
                return audited[index]

            ranked = sorted((audit(index), int(index)) for index in candidates)
            exact_top = ranked[:10]
            oracle_threshold = exact_top[-1][0]
            counts["final_top10_mismatches"] += int([(entry[0], entry[1]) for entry in best] != exact_top)
            counts["false_prunes"] += sum(bool(pruned[index]) and distance <= oracle_threshold for distance, index in ranked)
            for index, distance in refined.items():
                counts["refinement_parity_mismatches"] += int(distance != audit(index))
            suspect_lower = np.flatnonzero(lower_all > truth_lower)
            if len(suspect_lower) > AUDIT_CAP:
                raise RuntimeError("bound audit ambiguity exceeds fixed resource guard")
            for index in suspect_lower:
                counts["bound_violations"] += int(Fraction.from_float(float(lower_all[index])) > audit(index))
            for place in np.flatnonzero(seed_upper < truth_upper[seed_ids]):
                counts["seed_upper_violations"] += int(Fraction.from_float(float(seed_upper[place])) < audit(seed_ids[place]))
            seed_klo = np.partition(truth_lower[seed_ids], 9)[9]
            seed_khi = np.partition(truth_upper[seed_ids], 9)[9]
            if seed_klo > 0:
                seed_ratio_low.append(float(initial_upper) / seed_khi)
                seed_ratio_high.append(float(initial_upper) / seed_klo)
            if oracle_threshold:
                initial_final.append(float(initial_upper / oracle_threshold))
            else:
                zero_thresholds += 1
            refinements.append(len(refined))
            seed_refinements.append(sum(index in seed_set for index in refined))
            oracle_candidates.append(len(candidates))
        mean, p99 = float(np.mean(refinements)), float(np.quantile(refinements, .99))
        correctness_keys = ("signed_correction_mismatches", "bound_violations", "seed_upper_violations", "false_prunes",
                            "refinement_parity_mismatches", "final_top10_mismatches", "unresolved_final_seed_bounds",
                            "source_interval_violations")
        accepted = mean <= 2048 and p99 <= 8192 and all(counts[key] == 0 for key in correctness_keys)
        total_refined = sum(refinements)
        result.update({
            "decision": "accept-next-rung" if accepted else "reject-this-upper-seed-policy",
            "mean_additional_exact_rows": mean, "sample_p99_additional_exact_rows": p99,
            "additional_definition": "all actual source refinements after upper-only seeding, including admitted seeds",
            "refinement_counts": summary(refinements), "admitted_seed_counts": summary(seed_refinements),
            "initial_upper_to_final_exact_kth_ratio": summary(initial_final),
            "initial_upper_to_true_seed_kth_ratio_lower": summary(seed_ratio_low),
            "initial_upper_to_true_seed_kth_ratio_upper": summary(seed_ratio_high),
            "zero_exact_threshold_queries": zero_thresholds, "independent_exact_oracle_candidates": summary(oracle_candidates),
            "logical_source_bytes": {"setup_numerical_metadata": n * d * 4, "setup_proved_seed_metadata": SEEDS * d * 4,
                                     "query_metadata": QUERIES * d * 4,
                                     "serving_raw_seed_distances": 0, "serving_exact_refinements": total_refined * d * 4,
                                     "audit_full_source_scan": QUERIES * n * d * 4,
                                     "audit_fraction_distances": counts["audit_fraction_source_distances"] * d * 4},
            "source_byte_scope": "logical row requests; not measured physical DRAM/cache traffic; query loads and repeated native validation passes are not represented as raw-row bytes",
            "counter_row_storage_bytes": codes.nbytes + scales.nbytes + errors.nbytes + norms.nbytes + norm_uppers.nbytes,
            "counter_seed_cache_bytes": seed_codes.nbytes + seed_scales.nbytes + seed_errors.nbytes + seed_norm_upper.nbytes + seed_ids.nbytes,
            "native_extra_field_bytes_at_max_live": 10292043 * 4,
            "full_stream_seed_square_roots": 640 * 10000 * SEEDS,
            "projected_full_stream_actual_refinements": 640 * 10000 * mean,
            "projected_full_stream_logical_raw_refinement_bytes": 640 * 10000 * mean * d * 4,
            "projection_scope": "synthetic arithmetic counts only; no official-data or runtime prediction",
            "thresholds": {"mean_additional_max": 2048, "sample_p99_additional_max": 8192,
                           "correctness_failures_max": 0, "seconds_cap": 600},
            "numeric_limits": ["full scan uses H-B-5 numerical bound and explicit margin, not a formal native enclosure",
                               "seed upper vector arithmetic is separately audited on these inputs; E36 does not prove this different sequence",
                               "independent oracle intervals assume correctly rounded binary64 separate operations and gradual underflow; finite float32 source distances never under/overflow binary64",
                               "exact scores and final ties use source Fraction/native uint128; no float64 score is declared exact"],
            "unproved": ["native upper-seed implementation", "sustainable fused scan", "all seed/refinement/update costs",
                         "complete cgroup memory", "official GT parity", "untouched-data transfer"],
        })
        after = {name: digest(directory / name) for name in identity_names}
        if after != result["source_sha256"]:
            raise RuntimeError("counter sources changed during frozen run")
        if digest(args.base) != result["base"]["sha256"] or digest(args.queries) != result["queries"]["sha256"]:
            raise RuntimeError("counter inputs changed during frozen run")
    except Exception as error:
        result.update({"decision": "failed-or-rejected-no-complete-measurement", "failure_type": type(error).__name__,
                       "failure": str(error), "stage": stage})
        raise
    finally:
        result["counts"] = counts
        result["counter_wall_seconds_not_benchmark"] = time.monotonic() - started
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
        print(json.dumps({key: result[key] for key in ("decision", "mean_additional_exact_rows", "sample_p99_additional_exact_rows", "failure_type") if key in result}))


if __name__ == "__main__":
    main()
