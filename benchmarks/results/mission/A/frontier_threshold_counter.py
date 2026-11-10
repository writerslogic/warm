# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Certify the unchanged frontier against independent rounded-score observations."""

import argparse
import csv
import fcntl
import hashlib
import json
import os
import platform
import re
import signal
import sys
from fractions import Fraction
from pathlib import Path

from frontier_bound_counter import (
    CURSOR_CAP, D, INPUTS, N, QUERIES, finite_matrix, rational, require,
    sha256, verify_input, write_json,
)


PLAN_SHA = "4d3ee8ea252e661f936e6db6f6387389ff73e465c5f3d4f77009e2c8c60829ff"
E44_SOURCE_SHA = "6c2ed0e9575c555a0c1ed8e2c13c18c5966bd6415aad481b80623b641d7d8f7a"
QUERY_TILE = 8
CORPUS_TILE = 4096
OUTPUT_CAP_BYTES = 32 * 1024 * 1024
K = 100
QUESTION = "Does the unchanged4096 frontier exceed a certified true100th-IP upper bound?"
PREDICTION = "At least one strict rejection witness; retain all256 margins without early stopping."


def fraction_from_record(record):
    require(type(record) is dict, "invalid rational record")
    numerator = record["numerator"]
    denominator = record["denominator"]
    require(type(numerator) is str and type(denominator) is str
            and len(numerator) <= 4096 and len(denominator) <= 4096
            and re.fullmatch(r"-?(0|[1-9][0-9]*)", numerator) is not None
            and re.fullmatch(r"[1-9][0-9]*", denominator) is not None, "invalid rational encoding")
    value = Fraction(int(numerator), int(denominator))
    require(str(value.numerator) == numerator and str(value.denominator) == denominator,
            "noncanonical rational encoding")
    return value


def check_binary64():
    import numpy as np

    require(sys.float_info.radix == 2 and sys.float_info.mant_dig == 53
            and sys.float_info.max_exp == 1024 and sys.float_info.min_exp == -1021
            and np.dtype(np.float64).itemsize == 8, "requires IEEE binary64")


def promote_signed(bits):
    import numpy as np

    finite_matrix(bits)
    exponent = ((bits >> 23) & 255).astype(np.int32)
    significand = (bits & 0x7FFFFF) | np.where(exponent != 0, np.uint32(1 << 23), np.uint32(0))
    powers = np.where(exponent != 0, exponent - 150, -149)
    with np.errstate(all="raise"):
        values = np.ldexp(significand.astype(np.float64), powers)
        np.negative(values, out=values, where=(bits >> 31) != 0)
    require(bool(np.isfinite(values).all()), "invalid exact promotion")
    return values


def promote_base_columns(bits, progress):
    import numpy as np

    require(bits.shape == (N, D), "wrong base shape")
    columns = np.empty((D, N), dtype=np.float64)
    for start in range(0, N, CORPUS_TILE):
        progress.update(phase="promote_base", corpus_row=start, coordinate=None)
        stop = min(N, start + CORPUS_TILE)
        columns[:, start:stop] = promote_signed(bits[start:stop]).T
    return columns


def sequential_observations(base_columns, query_rows, progress=None):
    import numpy as np

    require(base_columns.ndim == 2 and base_columns.shape[0] == D
            and base_columns.dtype == np.float64 and 0 < base_columns.shape[1] <= N,
            "invalid promoted base")
    require(query_rows.ndim == 2 and query_rows.shape[1] == D
            and query_rows.dtype == np.float64 and 0 < len(query_rows) <= QUERY_TILE,
            "invalid promoted query tile")
    rows = base_columns.shape[1]
    score_bytes = len(query_rows) * rows * 8
    product_bytes = len(query_rows) * min(CORPUS_TILE, rows) * 8
    require(score_bytes + product_bytes <= OUTPUT_CAP_BYTES, "score output cap exceeded")
    observed = np.empty((len(query_rows), rows), dtype=np.float64)
    temporary = np.empty((len(query_rows), min(CORPUS_TILE, rows)), dtype=np.float64)
    with np.errstate(all="raise"):
        for start in range(0, rows, CORPUS_TILE):
            stop = min(rows, start + CORPUS_TILE)
            if progress is not None:
                progress.update(phase="observe_scores", corpus_row=start, coordinate=0)
            output = observed[:, start:stop]
            product = temporary[:, :stop - start]
            np.multiply(query_rows[:, 0, None], base_columns[0, None, start:stop], out=output)
            for coordinate in range(1, D):
                if progress is not None:
                    progress["coordinate"] = coordinate
                np.multiply(query_rows[:, coordinate, None], base_columns[coordinate, None, start:stop], out=product)
                np.add(output, product, out=output)
    require(bool(np.isfinite(observed).all()), "nonfinite rounded score observation")
    return observed


def observed_kths(observed, k):
    import numpy as np

    require(observed.ndim == 2 and observed.dtype == np.float64 and type(k) is int
            and 1 <= k <= observed.shape[1] and bool(np.isfinite(observed).all()), "invalid kth input")
    digests = [hashlib.sha256(memoryview(row).cast("B")).hexdigest() for row in observed]
    rank = observed.shape[1] - k
    observed.partition(rank, axis=1)
    result = []
    for row, digest in zip(observed, digests):
        value = float(row[rank])
        greater = int(np.count_nonzero(row > value))
        at_least = int(np.count_nonzero(row >= value))
        require(greater < k <= at_least, "computed kth order-statistic invariant failed")
        result.append(dict(value=value, observed_kth_hex=value.hex(), observed_kth=rational(Fraction(value)),
                           scores_strictly_greater=greater, scores_at_least=at_least,
                           ordered_observations_sha256=digest))
    return result


def threshold_decision(frontier, observed_kth, query_norm_squared, corpus_norm_upper, gamma):
    operands = (frontier, observed_kth, query_norm_squared, corpus_norm_upper, gamma)
    require(all(isinstance(value, Fraction) for value in operands), "threshold operands must be exact Fractions")
    require(query_norm_squared >= 0 and corpus_norm_upper >= 0 and 0 <= gamma < 1,
            "invalid norm/error bound")
    error_squared = gamma * gamma * query_norm_squared * corpus_norm_upper
    delta = frontier - observed_kth
    margin = delta * delta - error_squared
    rejected = delta > 0 and margin > 0
    return dict(rejected=rejected, status="certified_reject" if rejected else "inconclusive",
                frontier=rational(frontier), query_norm_squared=rational(query_norm_squared),
                error_squared=rational(error_squared), delta=rational(delta), delta_positive=delta > 0,
                exact_squared_comparison_margin=rational(margin),
                true_kth_upper="observed_kth + sqrt(error_squared)")


def load_prior(path, plan, own):
    pins = plan["reused_E44"]
    require(sha256(path) == pins["result_sha256"], "E44 result hash mismatch")
    require(sha256(own / "frontier_bound_counter.py") == pins["counter_sha256"] == E44_SOURCE_SHA,
            "E44 helper source changed")
    prior = json.loads(path.read_bytes())
    require(prior["status"] == "complete" and prior["experiment"] == "E44"
            and prior["decision"] == "inconclusive" and prior["integrity_failures"] == 0
            and prior["certified_rejection_queries"] == 0 and prior["inconclusive_queries"] == QUERIES,
            "invalid E44 completion receipt")
    require(prior["rows"] == N and prior["dimension"] == D and prior["query_count"] == QUERIES
            and prior["actual_cursor_cap"] == CURSOR_CAP, "E44 family mismatch")
    require(prior["script_sha256"] == pins["counter_sha256"]
            and prior["check_receipt_sha256"] == pins["checker_result_sha256"]
            and prior["card_sha256"] == pins["card_sha256"]
            and prior["research_sha256"] == pins["research_sha256"], "E44 provenance mismatch")
    require(len(prior["lower_frontier_f32_bits"]) == len(prior["upper_frontier_f32_bits"]) == D,
            "missing E44 frontier receipt")
    for name, (size, digest) in INPUTS.items():
        require(prior["inputs"][name]["bytes"] == size and prior["inputs"][name]["sha256"] == digest,
                "E44 input pin mismatch")
    norm = prior["corpus_norm_certificate"]
    require(norm["rows"] == N and norm["dimension"] == D, "E44 norm dimensions changed")
    maximum = fraction_from_record(norm["norm_upper"])
    gamma = fraction_from_record(norm["gamma"])
    require(maximum >= 0 and gamma == Fraction(D - 1, (1 << 52) - (D - 1)), "invalid E44 norm/gamma")
    rows = prior["queries"]
    require(len(rows) == QUERIES and [row["query_ordinal"] for row in rows] == list(range(QUERIES)),
            "E44 query identity mismatch")
    exact = []
    for row in rows:
        frontier = fraction_from_record(row["frontier"])
        query_norm = fraction_from_record(row["query_norm_squared"])
        require(query_norm >= 0, "negative E44 query norm")
        exact.append((frontier, query_norm))
    return prior, maximum, gamma, exact


def append_log(args, decision, outcome, failed=False):
    with args.ledger.open("a", newline="") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        csv.writer(stream, delimiter="\t").writerow([
            args.experiment, "experiment", "H-A-7", "R51-development", 2, QUESTION, PREDICTION,
            outcome, decision, "74bb9fa84b85a9bf1f58f89e5d35762f728ed17b",
            "fixed4096;128sequentialcoords;8queries/4096rows;exactMargin;noGT", platform.node(),
            str(args.failure.resolve() if failed else args.output.resolve()),
        ])
        stream.flush()
        os.fsync(stream.fileno())


def record_line(stream, value):
    stream.write(json.dumps(value, separators=(",", ":"), allow_nan=False) + "\n")


def run(args, progress):
    require(args.inputs.is_dir() and args.ledger.is_file(), "missing inputs/ledger")
    paths = (args.output, args.partial, args.failure)
    require(len({path.resolve() for path in paths}) == len(paths), "overlapping artifact paths")
    for path in paths:
        require(not path.exists() and not path.is_symlink(), "existing output/partial/failure artifact")
    own = Path(__file__).resolve().parent
    require(sha256(own / "e45-threshold-plan.json") == PLAN_SHA, "frozen E45 plan changed")
    plan = json.loads((own / "e45-threshold-plan.json").read_bytes())
    checked = json.loads(args.check_receipt.read_bytes())
    require(checked["status"] == "pass" and checked["decision"] == "accept-threshold-integrity-checks",
            "missing successful threshold checker")
    for name in ("frontier_threshold_counter.py", "frontier_threshold_check.py"):
        require(checked["source_sha256"][name] == sha256(own / name), "threshold checker source changed")
    require(checked["reused_E44_counter_sha256"] == E44_SOURCE_SHA, "checker used a different E44 dependency")
    prior, norm_upper, gamma, exact = load_prior(args.prior, plan, own)
    progress["phase"] = "hash_inputs"
    inputs = {name: verify_input(args.inputs / name, size, digest) for name, (size, digest) in INPUTS.items()}
    import numpy as np

    check_binary64()
    base_bits = np.memmap(args.inputs / "base.f32", dtype="<u4", mode="r", shape=(N, D))
    query_bits = np.memmap(args.inputs / "validation.f32", dtype="<u4", mode="r", shape=(QUERIES, D))
    args.partial.parent.mkdir(parents=True, exist_ok=True)
    results = []
    with args.partial.open("x") as retained:
        record_line(retained, dict(type="manifest", experiment="E45", plan_sha256=PLAN_SHA,
            prior_sha256=plan["reused_E44"]["result_sha256"], inputs=inputs,
            script_sha256=sha256(Path(__file__)), max_seconds=args.max_seconds,
            observation_contract="Exact original-IP scores are the underlying state; sequential binary64 sums are rounded observations. Pointwise errors<=gamma*sqrt(Q*M) imply true_kth<=observed_kth+gamma*sqrt(Q*M)."))
        retained.flush()
        os.fsync(retained.fileno())
        base = promote_base_columns(base_bits, progress)
        queries = promote_signed(query_bits)
        for begin in range(0, QUERIES, QUERY_TILE):
            stop = min(QUERIES, begin + QUERY_TILE)
            progress.update(query_tile_begin=begin, completed_queries=len(results))
            observed = sequential_observations(base, queries[begin:stop], progress)
            progress.update(phase="observed_kth", corpus_row=None, coordinate=None)
            thresholds = observed_kths(observed, K)
            for offset, observation in enumerate(thresholds):
                ordinal = begin + offset
                frontier, query_norm = exact[ordinal]
                observed_kth = Fraction.from_float(observation.pop("value"))
                witness = dict(query_ordinal=ordinal, **observation,
                               **threshold_decision(frontier, observed_kth, query_norm, norm_upper, gamma))
                results.append(witness)
                record_line(retained, dict(type="query_witness", **witness))
            retained.flush()
            os.fsync(retained.fileno())
            progress["completed_queries"] = len(results)
            del observed
        require(len(results) == QUERIES and [row["query_ordinal"] for row in results] == list(range(QUERIES)),
                "incomplete query witnesses")
        progress["phase"] = "recheck_inputs"
        for name, (size, digest) in INPUTS.items():
            verify_input(args.inputs / name, size, digest)
        record_line(retained, dict(type="complete", query_count=len(results), integrity_failures=0))
        retained.flush()
        os.fsync(retained.fileno())
    rejected = sum(row["rejected"] for row in results)
    result = dict(status="complete", experiment="E45", hypothesis="H-A-7", row="R51", claim="local", rung=2,
        question=QUESTION, prediction=PREDICTION, decision="reject" if rejected else "inconclusive",
        certified_rejection_queries=rejected, inconclusive_queries=QUERIES - rejected,
        feasible=None, performance_acceptance=False, score_claim=False, matched_gap_condition="UNPROVED",
        cursor_cap=CURSOR_CAP, rows=N, dimension=D, query_count=QUERIES, k=K, max_seconds=args.max_seconds,
        query_base_pairs=N * QUERIES, exact_coordinate_products=N * QUERIES * D,
        rounded_binary64_additions=N * QUERIES * (D - 1), query_tile=QUERY_TILE, corpus_tile=CORPUS_TILE,
        score_output_bytes=QUERY_TILE * N * 8, product_output_bytes=QUERY_TILE * CORPUS_TILE * 8,
        outputs_cap_bytes=OUTPUT_CAP_BYTES, queries=results, gamma=rational(gamma), norm_upper=rational(norm_upper),
        state_observation_contract="Exact original-IP scores are unobserved real states; freshly computed binary64 scores are rounded observations. The pointwise global error enclosure propagates monotonically to an upper bound on the true100th score. Neither stored nor new rounded rankings are asserted to equal the exact ranking.",
        upper_transition_invariant="true_score_i<=observed_score_i+sqrt(error_squared) for every row implies true100th<=observed100th+sqrt(error_squared)",
        exact_rejection_predicate="delta=L-observed100th; delta>0 AND delta²>error_squared",
        inputs=inputs, prior=str(args.prior.resolve()), prior_sha256=plan["reused_E44"]["result_sha256"],
        plan_sha256=PLAN_SHA, script_sha256=sha256(Path(__file__)), reused_E44_counter_sha256=E44_SOURCE_SHA,
        check_receipt=str(args.check_receipt.resolve()), check_receipt_sha256=sha256(args.check_receipt),
        retained_witnesses=str(args.partial.resolve()), retained_witnesses_sha256=sha256(args.partial),
        integrity_failures=0, gold_files_opened=[], official_test_read=False,
        environment=dict(python=sys.version, numpy=np.__version__, platform=platform.platform()),
        limits="Only the fixed4096 threshold-certificate family is tested. Equality/non-rejection is inconclusive. No returned-neighbor accuracy score, ANN search, full sorted index, BLAS, parameter sweep, timing gain, fresh holdout or MATRIX-row acceptance.")
    progress["phase"] = "write_result"
    write_json(args.output, result)
    summary = f"certified_reject={rejected}/{QUERIES};inconclusive={QUERIES-rejected};0integrity_failures;independent_sequential_threshold;noGT"
    append_log(args, "reject" if rejected else "compose", summary)
    print(f"E45 {result['decision']}: {summary}; {args.output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--prior", type=Path, required=True)
    parser.add_argument("--check-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--partial", type=Path, required=True)
    parser.add_argument("--failure", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--experiment", choices=("E45",), required=True)
    parser.add_argument("--max-seconds", type=int, default=180)
    args = parser.parse_args()
    progress = dict(phase="preflight", completed_queries=0, query_tile_begin=None, corpus_row=None, coordinate=None)

    def interrupted(number, frame):
        if number == signal.SIGALRM:
            raise TimeoutError("E45 exceeded its registered180s ceiling")
        raise InterruptedError(f"E45 interrupted by signal {number}")

    try:
        require(1 <= args.max_seconds <= 180, "counter cap must be1..180seconds")
        signal.signal(signal.SIGALRM, interrupted)
        signal.signal(signal.SIGTERM, interrupted)
        signal.setitimer(signal.ITIMER_REAL, args.max_seconds)
        run(args, progress)
    except (Exception, KeyboardInterrupt) as error:
        signal.setitimer(signal.ITIMER_REAL, 0)
        partial = None
        if args.partial.is_file() and not args.partial.is_symlink():
            partial = dict(path=str(args.partial.resolve()), bytes=args.partial.stat().st_size, sha256=sha256(args.partial))
        failure = dict(status="failed", experiment="E45", hypothesis="H-A-7", decision=None, feasible=None,
            performance_acceptance=False, progress=progress, error_type=type(error).__name__, error=str(error),
            script_sha256=sha256(Path(__file__)), partial_artifact=partial, output_exists=args.output.exists(),
            limits="Partial observations are retained but incomplete execution is not a mechanism result; every256 margins is required. No automatic retry or changed backend/budget.")
        try:
            write_json(args.failure, failure)
        except Exception as receipt_error:
            print(f"E45 failure receipt error: {type(receipt_error).__name__}: {receipt_error}", file=sys.stderr)
        try:
            if args.ledger.is_file():
                append_log(args, "reject", f"execution_failure,not mechanism rejection: {type(error).__name__}: {error};progress={json.dumps(progress, sort_keys=True)}", failed=True)
        except Exception as log_error:
            print(f"E45 failure ledger error: {type(log_error).__name__}: {log_error}", file=sys.stderr)
        print(f"E45 failed: {type(error).__name__}: {error}; {args.failure}", file=sys.stderr)
        return 1
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
