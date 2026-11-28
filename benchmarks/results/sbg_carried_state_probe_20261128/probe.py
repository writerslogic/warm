# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Carried-state probe for search-benchmark-game engines.

An engine carries work between queries when a query repeated back to back is much cheaper than the same kind of query seen
for the first time (result caches, memoised postings or block decodes, warmed per-query state). The probe drives each
engine's own `make serve` process over the benchmark protocol (one `COMMAND<TAB>query` line in, one answer line out) and
times every round trip, in fresh processes, for three workloads of equal length:

  hot     a fixed small set of queries repeated N times
  mixed   N distinct queries drawn from the query file
  twice   the same N distinct queries sent twice in one process; pass 2 over pass 1 holds query cost fixed, so it isolates
          what the engine remembers from what the queries cost

Every process first runs a warmup of distinct queries outside all three sets (JIT, page cache). Workload order alternates
between repeats. The per-query figure is the median round trip in microseconds, so pipe latency is included and is
the same for every engine; ratios of two workloads on one engine cancel it only partly, absolute microseconds are printed.

Flag: `twice` ratio below --flag (default 0.7) means pass 2 is faster than pass 1 by more than 30 percent on identical
queries. `hot/mixed` is reported next to it (it also reflects the cost mix of the chosen queries, so it is not used to flag).

usage: probe.py --fork DIR --engines infino-0.11.26 infino-main lucene-10.5.0 lucene-10.5.0-bp --out DIR
       probe.py --selftest
Engines must be built and indexed (`make compile index` in the fork); the probe does not build.
"""

import argparse
import json
import random
import statistics
import subprocess
import sys
import time
from pathlib import Path

DEFAULT_HOT = ["+william +wallace", "montgomery college", "john klein"]


def load_queries(path):
    queries = []
    with open(path) as handle:
        for line in handle:
            if line.strip():
                queries.append(json.loads(line)["query"])
    return queries


def split_sets(queries, n, hot_size, seed, hot=None):
    """(hot set, mixed list, warmup list), pairwise disjoint; n distinct mixed queries."""
    rng = random.Random(seed)
    pool = list(dict.fromkeys(queries))
    hot = [q for q in (hot or DEFAULT_HOT) if q in pool][:hot_size]
    rest = [q for q in pool if q not in hot]
    rng.shuffle(rest)
    while len(hot) < hot_size and rest:
        hot.append(rest.pop())
    warm = min(200, len(rest) - n)
    if warm < 0:
        raise SystemExit(f"{len(pool)} distinct queries cannot supply {n} mixed + {hot_size} hot + a warmup; lower --n")
    return hot, rest[:n], rest[n:n + warm]


class Engine:
    def __init__(self, cmd, cwd):
        self.proc = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)

    def ask(self, command, query):
        """Round-trip time in nanoseconds, or None when the engine answers UNSUPPORTED."""
        start = time.perf_counter_ns()
        self.proc.stdin.write(f"{command}\t{query}\n".encode())
        self.proc.stdin.flush()
        answer = self.proc.stdout.readline()
        elapsed = time.perf_counter_ns() - start
        if not answer:
            raise RuntimeError("engine closed its output")
        return None if answer.strip() == b"UNSUPPORTED" else elapsed

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=60)


def run_workload(cmd, cwd, command, warmup, workload):
    """Per-query round trips (ns) of `workload`, a list of queries, in a fresh process after the warmup."""
    engine = Engine(cmd, cwd)
    try:
        for q in warmup:
            if engine.ask(command, q) is None:
                return None
        times = []
        for q in workload:
            t = engine.ask(command, q)
            if t is None:
                return None
            times.append(t)
        return times
    finally:
        engine.close()


def probe_engine(cmd, cwd, command, hot, mixed, warmup, repeats):
    n = len(mixed)
    hot_load = [hot[i % len(hot)] for i in range(n)]
    rows = {"hot": [], "mixed": [], "pass1": [], "pass2": []}
    for r in range(repeats):
        order = ["hot", "mixed", "twice"] if r % 2 == 0 else ["twice", "mixed", "hot"]
        for name in order:
            if name == "hot":
                t = run_workload(cmd, cwd, command, warmup, hot_load)
                if t is None:
                    return None
                rows["hot"].append(statistics.median(t) / 1000)
            elif name == "mixed":
                t = run_workload(cmd, cwd, command, warmup, mixed)
                if t is None:
                    return None
                rows["mixed"].append(statistics.median(t) / 1000)
            else:
                t = run_workload(cmd, cwd, command, warmup, mixed + mixed)
                if t is None:
                    return None
                rows["pass1"].append(statistics.median(t[:n]) / 1000)
                rows["pass2"].append(statistics.median(t[n:]) / 1000)
    med = {k: statistics.median(v) for k, v in rows.items()}
    return {"per_repeat_median_us": rows, "median_us": med, "hot_over_mixed": med["hot"] / med["mixed"], "twice": med["pass2"] / med["pass1"],
            "twice_per_repeat": [b / a for a, b in zip(rows["pass1"], rows["pass2"])]}


def engine_command(fork, name):
    cwd = Path(fork) / "engines" / name
    if not (cwd / "idx").is_dir():
        raise SystemExit(f"{cwd}/idx is missing: build and index the engine first")
    return ["make", "-s", "serve"], cwd


def report(results, flag):
    lines = [f"{'engine':<22}{'command':<11}{'hot us':>9}{'mixed us':>10}{'pass1 us':>10}{'pass2 us':>10}{'hot/mixed':>11}{'pass2/pass1':>13}  verdict"]
    for engine, per_cmd in results.items():
        for command, r in per_cmd.items():
            if r is None:
                lines.append(f"{engine:<22}{command:<11}{'UNSUPPORTED':>9}")
                continue
            m = r["median_us"]
            verdict = "CARRIES STATE" if r["twice"] < flag else "no carry detected"
            lines.append(f"{engine:<22}{command:<11}{m['hot']:>9.1f}{m['mixed']:>10.1f}{m['pass1']:>10.1f}{m['pass2']:>10.1f}{r['hot_over_mixed']:>11.2f}{r['twice']:>13.2f}  {verdict}")
    return "\n".join(lines)


STUB = r'''
import sys
cache = {}
memo = sys.argv[1] == "memo"
for line in sys.stdin:
    q = line.rstrip("\n").split("\t", 1)[1]
    if not (memo and q in cache):
        s = 0
        for _ in range(20000):
            s += hash(q) & 7
        cache[q] = s
    print(cache[q], flush=True)
'''


def selftest(flag):
    """A memoising stub must be flagged and a stateless stub must not."""
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        stub = Path(d) / "stub.py"
        with open(stub, "w") as handle:
            handle.write(STUB)
        queries = [f"q{i} w{i % 7}" for i in range(400)]
        hot, mixed, warm = split_sets(queries, 100, 3, 1, hot=[])
        verdicts = {}
        for kind in ("memo", "plain"):
            r = probe_engine([sys.executable, str(stub), kind], d, "COUNT", hot, mixed, warm, 3)
            verdicts[kind] = r["twice"] < flag
            print(kind, f"pass2/pass1 {r['twice']:.2f} hot/mixed {r['hot_over_mixed']:.2f}")
        if verdicts != {"memo": True, "plain": False}:
            raise SystemExit(f"selftest failed: {verdicts}")
    print("selftest ok")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--fork", help="search-benchmark-game checkout with built and indexed engines")
    p.add_argument("--engines", nargs="+", default=["infino-0.11.26", "infino-main", "lucene-10.5.0", "lucene-10.5.0-bp"])
    p.add_argument("--queries", default=None, help="JSON-lines query file (default FORK/queries-full.txt)")
    p.add_argument("--commands", nargs="+", default=["COUNT", "TOP_10"])
    p.add_argument("--n", type=int, default=500, help="queries per workload")
    p.add_argument("--hot-size", type=int, default=3)
    p.add_argument("--repeats", type=int, default=6)
    p.add_argument("--seed", type=int, default=7)
    p.add_argument("--flag", type=float, default=0.7)
    p.add_argument("--out", help="directory for probe.json and probe.txt")
    p.add_argument("--selftest", action="store_true")
    a = p.parse_args()
    if a.selftest:
        selftest(a.flag)
        return
    if not a.fork or not a.out:
        p.error("--fork and --out are required")
    queries = load_queries(a.queries or Path(a.fork) / "queries-full.txt")
    hot, mixed, warm = split_sets(queries, a.n, a.hot_size, a.seed)
    results = {}
    for engine in a.engines:
        cmd, cwd = engine_command(a.fork, engine)
        results[engine] = {c: probe_engine(cmd, cwd, c, hot, mixed, warm, a.repeats) for c in a.commands}
        print(f"done {engine}", file=sys.stderr)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    text = report(results, a.flag)
    with open(out / "probe.json", "w") as handle:
        json.dump({"args": vars(a) | {"fork": Path(a.fork).name, "out": Path(a.out).name}, "hot": hot, "n_distinct_queries": len(mixed), "warmup": len(warm), "results": results}, handle, indent=1)
    with open(out / "probe.txt", "w") as handle:
        handle.write(text + "\n")
    print(text)


if __name__ == "__main__":
    main()
