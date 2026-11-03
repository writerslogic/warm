# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Development-only soft-cue tensor contraction with a guarded factorized path."""

import argparse
import hashlib
import json
from pathlib import Path
import platform
import resource
import signal
import time

import numpy as np


HERE = Path(__file__).resolve().parent
EPS = np.finfo(np.float64).eps / 2


def gamma(operations):
    return np.nextafter(operations * EPS / (1 - operations * EPS), np.inf)


def fit(tensor, rank):
    a, b, c = tensor.shape
    left, values, right = np.linalg.svd(tensor.reshape(a, b * c), full_matrices=False)
    r = min(rank, len(values))
    first = left[:, :r].copy()
    tail = (values[:r, None] * right[:r]).reshape(r * b, c)
    left, values, right = np.linalg.svd(tail, full_matrices=False)
    s = min(rank, len(values))
    return first, left[:, :s].reshape(r, b, s).copy(), (values[:s, None] * right[:s]).copy()


def reconstruct(cores):
    first, middle, last = cores
    return np.einsum("ia,ajb,bk->ijk", first, middle, last, optimize=True)


def exact(tensor, left, right):
    i, j = np.flatnonzero(left), np.flatnonzero(right)
    if len(i) * len(j) < tensor.shape[0] * tensor.shape[1] / 4:
        selected = tensor[np.ix_(i, j, np.arange(tensor.shape[2]))]
        return np.einsum("i,j,ijk->k", left[i], right[j], selected, optimize=True)
    return right @ (left @ tensor.reshape(tensor.shape[0], -1)).reshape(tensor.shape[1:])


class Memory:
    def __init__(self, tensor, rank=4, iterations=8, threshold=0.1, max_fraction=0.02):
        tensor = np.asarray(tensor, dtype=np.float64)
        if tensor.ndim != 3 or any(size < 1 or size > 256 for size in tensor.shape) or not np.isfinite(tensor).all() or np.max(np.abs(tensor)) > 1e6:
            raise ValueError("invalid observed tensor")
        if type(rank) is not int or not 1 <= rank <= 16 or type(iterations) is not int or not 1 <= iterations <= 16:
            raise ValueError("invalid fit budget")
        if not np.isfinite(threshold) or threshold <= 0 or not 0 <= max_fraction <= 1:
            raise ValueError("invalid correction policy")
        self.tensor = tensor.copy()
        self.tensor_max = float(np.max(np.abs(tensor)))
        correction = np.zeros_like(tensor)
        for _ in range(iterations):
            cores = fit(tensor - correction, rank)
            base = reconstruct(cores)
            residual = tensor - base
            correction = np.where(np.abs(residual) > threshold, residual, 0)
        self.positions = np.asarray(np.nonzero(correction), dtype=np.int32)
        self.values = correction[tuple(self.positions)].copy()
        self.cores = cores
        self.enabled = len(self.values) <= max_fraction * tensor.size
        envelope = reconstruct(tuple(np.abs(core) for core in cores))
        remaining = tensor - (base + correction)
        scale = float(np.max(envelope) + np.max(np.abs(correction)) + np.max(np.abs(tensor)) + 1)
        self.residual_bound = float(np.nextafter(np.max(np.abs(remaining)) + gamma(4 * rank + 16) * scale, np.inf))
        self.envelope_max = float(np.nextafter(np.max(envelope), np.inf))
        self.correction_l1 = float(np.nextafter(np.abs(self.values).sum(), np.inf))

    def query(self, left, right):
        left, right = np.asarray(left, dtype=np.float64), np.asarray(right, dtype=np.float64)
        if left.shape != (self.tensor.shape[0],) or right.shape != (self.tensor.shape[1],):
            raise ValueError("incorrect cue shape")
        if not np.isfinite(left).all() or not np.isfinite(right).all() or np.any(left < 0) or np.any(right < 0):
            raise ValueError("invalid cue values")
        if abs(float(left.sum()) - 1) > 1e-12 or abs(float(right.sum()) - 1) > 1e-12:
            raise ValueError("cue weights must sum to one")
        if not self.enabled or np.count_nonzero(left) * np.count_nonzero(right) < self.tensor.shape[0] * self.tensor.shape[1] / 4:
            return int(np.argmax(exact(self.tensor, left, right))), False, None
        first, middle, last = self.cores
        middle_query = np.einsum("j,ajb->ab", right, middle)
        scores = ((left @ first) @ middle_query) @ last
        i, j, k = self.positions
        if len(self.values):
            scores += np.bincount(k, weights=left[i] * right[j] * self.values, minlength=len(scores))
        weight = float(np.nextafter(left.sum() * right.sum() * (1 + gamma(2 * sum(self.tensor.shape))), np.inf))
        operations = 4 * sum(self.tensor.shape) + 8 * max(core.shape[0] for core in self.cores) + len(self.values) + 64
        rounding = gamma(operations) * weight * (self.envelope_max + self.correction_l1 + self.tensor_max + 1)
        bound = float(np.nextafter(weight * self.residual_bound + rounding, np.inf))
        best = int(np.argmax(scores))
        second = float(np.max(np.delete(scores, best))) if len(scores) > 1 else -np.inf
        if float(scores[best]) - second > 2 * bound:
            return best, True, bound
        return int(np.argmax(exact(self.tensor, left, right))), False, bound

    def state_hash(self):
        digest = hashlib.sha256()
        for value in [self.tensor, *self.cores, self.positions, self.values]:
            digest.update(value.tobytes())
        digest.update(np.float64(self.residual_bound).tobytes())
        return digest.hexdigest()

    def bytes(self):
        return {"retainedOriginalBytes": self.tensor.nbytes, "coreBytes": sum(core.nbytes for core in self.cores),
                "correctionBytes": self.positions.nbytes + self.values.nbytes,
                "note": "Numeric payload only; original tensor remains resident. Process peak RSS is separately reported."}


def make_tensor(plan, scenario, seed):
    rng = np.random.default_rng(seed)
    a, b, c = plan["shape"]
    if scenario == "independent":
        return rng.uniform(-1, 1, size=(a, b, c))
    rank = plan["rank"]
    tensor = reconstruct((rng.normal(size=(a, rank)), rng.normal(size=(rank, b, rank)), rng.normal(size=(rank, c))))
    tensor /= np.max(np.abs(tensor))
    if scenario == "rare_exceptions":
        chosen = rng.choice(tensor.size, int(tensor.size * plan["rareFraction"]), replace=False)
        tensor.flat[chosen] += rng.choice([-1, 1], size=len(chosen)) * plan["rareMagnitude"]
    return np.ascontiguousarray(tensor)


def cue(rng, size, support):
    result = np.zeros(size)
    ids = rng.choice(size, support, replace=False)
    result[ids] = rng.uniform(0.5, 1.5, size=support)
    return result / result.sum()


def measure(function):
    start = time.perf_counter_ns()
    value = function()
    return value, time.perf_counter_ns() - start


def run_case(plan, scenario, seed, stream):
    tensor = make_tensor(plan, scenario, seed)
    if not np.shares_memory(tensor, tensor.reshape(tensor.shape[0], -1)):
        raise AssertionError("exact control reshape must not copy input")
    build = lambda data: Memory(data, plan["rank"], plan["robustIterations"], plan["correctionThreshold"], plan["maxCorrectionFraction"])
    memory, build_ns = measure(lambda: build(tensor))
    rng = np.random.default_rng(seed + 10000)
    families = []
    for support in plan["cueSupports"]:
        control_times, candidate_times, certified, wrong = [], [], 0, 0
        for query in range(plan["queriesPerFamily"]):
            left, right = cue(rng, tensor.shape[0], support), cue(rng, tensor.shape[1], support)
            expected = int(np.argmax(exact(tensor, left, right)))
            memory.query(left, right)
            observations = []
            for repeat in range(3):
                control = lambda: int(np.argmax(exact(tensor, left, right)))
                candidate = lambda: memory.query(left, right)
                if repeat % 2:
                    answer, elapsed = measure(candidate)
                    truth, baseline = measure(control)
                else:
                    truth, baseline = measure(control)
                    answer, elapsed = measure(candidate)
                if truth != expected or answer[0] != expected:
                    wrong += 1
                control_times.append(baseline); candidate_times.append(elapsed)
                observations.append({"winner": answer[0], "certified": answer[1], "bound": answer[2], "controlNs": baseline, "candidateNs": elapsed})
            certified += int(observations[0]["certified"])
            stream.write(json.dumps({"scenario": scenario, "seed": seed, "support": support, "query": query,
                                    "left": left.tolist(), "right": right.tolist(),
                                    "expected": expected, "observations": observations}) + "\n")
        percentiles = lambda values: {"median": float(np.median(values)), "p95": float(np.quantile(values, .95, method="higher"))}
        control, candidate = percentiles(control_times), percentiles(candidate_times)
        families.append({"support": support, "wrong": wrong, "certified": certified, "queries": plan["queriesPerFamily"],
                         "controlNs": control, "candidateNs": candidate,
                         "passes": wrong == 0 and certified >= .9 * plan["queriesPerFamily"] and all(candidate[key] <= control[key] / 2 for key in control)})
    tensor[0, 0, 0] = 0.75
    tensor[1, 1, 1] = 0
    rebuilt, rebuild_ns = measure(lambda: build(tensor))
    reference = build(tensor)
    if rebuilt.state_hash() != reference.state_hash():
        raise AssertionError("rebuild differs from fresh surviving-state encoding")
    for _ in range(16):
        left, right = cue(rng, tensor.shape[0], 64), cue(rng, tensor.shape[1], 64)
        if rebuilt.query(left, right)[0] != int(np.argmax(exact(tensor, left, right))):
            raise AssertionError("wrong winner after overwrite/forget rebuild")
    return {"scenario": scenario, "seed": seed, "enabled": memory.enabled, "residualBound": memory.residual_bound,
            "inputSha256": hashlib.sha256(memory.tensor.tobytes()).hexdigest(),
            "inputLayout": {"cContiguous": tensor.flags.c_contiguous, "strides": tensor.strides, "controlReshapeSharesMemory": True},
            "updatedInputSha256": hashlib.sha256(tensor.tobytes()).hexdigest(),
            "bytes": memory.bytes(), "buildNs": build_ns, "rebuildNs": rebuild_ns, "freshRebuildMatched": True, "families": families}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir()
    plan = json.loads((HERE / "PLAN.json").read_text())
    hashes = {}
    for name in ["PLAN.json", "probe.py", "test_probe.py", "pyproject.toml", "uv.lock"]:
        raw = (HERE / name).read_bytes()
        (args.output / name).write_bytes(raw)
        hashes[name] = hashlib.sha256(raw).hexdigest()
    (args.output / "freeze.json").write_text(json.dumps({"plan": plan, "hashes": hashes, "numpy": np.__version__,
        "python": platform.python_version(), "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}, indent=2) + "\n")
    def timeout(_signum, _frame):
        raise RuntimeError("frozen research deadline exceeded")
    signal.signal(signal.SIGALRM, timeout)
    signal.alarm(plan["resources"]["seconds"])
    cases, failure = [], None
    try:
        with (args.output / "queries.ndjson").open("x", buffering=1) as stream:
            for scenario in plan["scenarios"]:
                for seed in plan["seeds"]:
                    case = run_case(plan, scenario, seed, stream)
                    cases.append(case)
                    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if platform.system() == "Darwin" else 1024)
                    if rss > plan["resources"]["rssBytes"]:
                        raise RuntimeError("frozen research RSS limit exceeded")
                    print(json.dumps({"scenario": scenario, "seed": seed, "families": case["families"]}), flush=True)
    except Exception as error:
        failure = {"type": type(error).__name__, "message": str(error)}
    finally:
        signal.alarm(0)
    for name, expected in hashes.items():
        if hashlib.sha256((HERE / name).read_bytes()).hexdigest() != expected:
            failure = {"type": "SourceChanged", "message": name}
    quality = all(family["wrong"] == 0 for case in cases for family in case["families"])
    qualified = failure is None and quality and len(cases) == 9 and all(family["passes"] for case in cases
        if case["scenario"] != "independent" for family in case["families"] if family["support"] != 4)
    result = {"complete": failure is None, "qualityChecksPassed": quality and len(cases) == 9, "promoteToNativeExperiment": qualified, "failure": failure, "cases": cases,
              "queriesSha256": hashlib.sha256((args.output / "queries.ndjson").read_bytes()).hexdigest(),
              "peakRssBytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if platform.system() == "Darwin" else 1024)}
    (args.output / "finished.json").write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
    if failure:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
