# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Hardware class, statistics and subprocess measurement shared by the runner, workers and table."""

from contextlib import suppress
import contextlib
import fcntl
import json
import math
import os
import platform
import random
import re
import resource
import select
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RESULTS = Path(os.environ.get("WARM_BENCH_RESULTS", ROOT / "benchmarks/results/competitive"))
CACHE = Path(os.environ.get("WARM_BENCH_DATA", Path.home() / ".cache" / "warm-bench"))


def _sysctl(name):
    try:
        return subprocess.run(["sysctl", "-n", name], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return ""


def cpu_model():
    if platform.system() == "Darwin":
        return _sysctl("machdep.cpu.brand_string")
    with suppress(OSError):
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    # arm64 Linux /proc/cpuinfo has no "model name"; lscpu reads the core name (for example Neoverse-N1) from the MIDR.
    with suppress(OSError, subprocess.SubprocessError):
        for line in subprocess.run(["lscpu"], capture_output=True, text=True, check=True, env={**os.environ, "LC_ALL": "C"}).stdout.splitlines():
            if line.startswith("Model name:") and line.split(":", 1)[1].strip():
                return line.split(":", 1)[1].strip()
    return platform.processor() or platform.machine()


def memory_bytes():
    if platform.system() == "Darwin":
        return int(_sysctl("hw.memsize") or 0)
    with suppress(OSError):
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal"):
                return int(line.split()[1]) * 1024
    return 0


def hardware():
    """Hardware description. `class` is the only key rows may be compared across."""
    cpu, cores, mem = cpu_model(), os.cpu_count() or 0, memory_bytes()
    slug = "".join(c if c.isalnum() else "-" for c in cpu.lower())
    slug = "-".join(part for part in slug.split("-") if part and part not in ("r", "tm", "cpu", "processor", "with", "core"))
    derived = f"{platform.system().lower()}-{platform.machine()}-{slug}-{cores}c-{round(mem / 2**30)}g"
    return {
        "class": os.environ.get("WARM_BENCH_HW_CLASS", derived),
        "cpu": cpu, "logical_cores": cores, "memory_bytes": mem,
        "os": f"{platform.system()} {platform.release()}", "machine": platform.machine(),
        "hosted_runner": bool(os.environ.get("GITHUB_ACTIONS")),
        "load_1m": os.getloadavg()[0],
    }


FULL_COMMIT = re.compile(r"[0-9a-f]{40}")


ENCRYPTION_CHOICES = ("on", "off")


def encryption():
    """Encryption at rest of the WARM store for this run, from the WARM_ENCRYPTION switch.

    `on` builds WARM with the `security` feature and encrypts (AES-256-GCM, master key);
    `off`, the default of the harness, stores plaintext. The engine reads the same variable,
    so a binary built with `security` also honors `off`.
    """
    switch = os.environ.get("WARM_ENCRYPTION", "off").strip().lower() or "off"
    if switch not in ENCRYPTION_CHOICES:
        raise SystemExit(f"WARM_ENCRYPTION must be one of {ENCRYPTION_CHOICES}, got {switch!r}")
    return {"switch": switch, "effective": "aes-256-gcm" if switch == "on" else "plaintext"}


def commit():
    """Full 40-character source revision of the measured tree. The table treats anything shorter as unpinned and draws
    no verdict from it, so an abbreviated WARM_BENCH_COMMIT is expanded through git and refused when it cannot be."""
    given = os.environ.get("WARM_BENCH_COMMIT")  # a shipped source tree has no .git
    revision = given or "HEAD"
    try:
        full = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--verify", "--quiet", revision + "^{commit}"],
                              capture_output=True, text=True, check=True).stdout.strip().lower()
    except (OSError, subprocess.CalledProcessError):
        full = (given or "").lower()
    if given and not FULL_COMMIT.fullmatch(full):
        raise SystemExit(f"WARM_BENCH_COMMIT={given!r} is not a full 40-character commit id and git cannot expand it here; "
                         "pass the full id (git rev-parse HEAD in the measured tree)")
    return full if FULL_COMMIT.fullmatch(full) else "unknown"


def _git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)


def source_record(suite, revision=None):
    """Where the measured tree came from, refusing a publishable result that nobody can rebuild. A non-smoke result needs
    a clean tracked tree (untracked files ignored) at a commit reachable from origin/main; otherwise WARM_BENCH_DRAFT=1
    must be set, and the file is marked a draft (the table draws no verdict from it)."""
    revision = revision or commit()
    draft = bool(os.environ.get("WARM_BENCH_DRAFT"))
    if suite == "smoke":
        return {"commit": revision, "draft": True, "checked": "smoke run, never published"}
    if not (ROOT / ".git").exists():
        # A shipped tree (modal_run.py) carries the launcher's checks in WARM_BENCH_SOURCE.
        shipped = json.loads(os.environ.get("WARM_BENCH_SOURCE") or "{}")
        dirty, reachable = shipped.get("dirty_tree"), shipped.get("reachable_from_origin_main")
        reasons = ([] if shipped else ["no git metadata in the measured tree and no launcher record"]) + (["tracked files differed from the commit at launch"] if dirty else []) + \
            ([f"{revision} is not reachable from origin/main"] if shipped and not reachable else [])
    else:
        _git("fetch", "--quiet", "origin", "main")
        dirty = bool(_git("status", "--porcelain", "--untracked-files=no").stdout.strip())
        reachable = _git("merge-base", "--is-ancestor", revision, "origin/main").returncode == 0
        reasons = (["tracked files differ from the commit"] if dirty else []) + ([] if reachable else [f"{revision} is not reachable from origin/main"])
    if reasons and not draft:
        raise SystemExit("refusing to write a publishable result: " + "; ".join(reasons) + ". Run from a clean checkout of main, or set WARM_BENCH_DRAFT=1 to mark the file a draft")
    return {"commit": revision, "dirty_tree": dirty, "reachable_from_origin_main": reachable, "draft": bool(reasons) or draft, "draft_reasons": reasons}


def quantile(values, q):
    values = sorted(values)
    at = (len(values) - 1) * q
    low = int(at)
    high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (at - low)


def pass_summary(ns):
    """Compact record of one timed pass over the queries (per-query nanoseconds in): what the
    table's QPS and latency intervals are computed from, with the same order statistics."""
    srt = sorted(ns)
    at = lambda q: srt[int((len(srt) - 1) * q)]  # noqa: E731
    return {"queries": len(srt), "total_ns": sum(srt), "p50_ns": srt[(len(srt) - 1) // 2], "p90_ns": at(0.90), "p99_ns": at(0.99), "max_ns": srt[-1]}


def bootstrap_ci(values, statistic=statistics.median, rounds=2000, seed=24731, level=0.95):
    """Percentile bootstrap interval of `statistic` over repeats; one repeat is degenerate."""
    values = list(values)
    if len(values) < 2:
        return [values[0], values[0]] if values else [math.nan, math.nan]
    rng = random.Random(seed)
    draws = sorted(statistic([rng.choice(values) for _ in values]) for _ in range(rounds))
    tail = (1 - level) / 2
    return [draws[int(tail * rounds)], draws[min(rounds - 1, int((1 - tail) * rounds))]]


def ratio_ci(numerator, denominator, rounds=2000, seed=24731, level=0.95):
    """Point ratio and bootstrap interval for median(numerator) / median(denominator)."""
    rng = random.Random(seed)
    num, den = list(numerator), list(denominator)
    point = statistics.median(num) / statistics.median(den)
    if len(num) < 2 and len(den) < 2:
        return point, [point, point]
    draws = sorted(
        statistics.median([rng.choice(num) for _ in num]) / statistics.median([rng.choice(den) for _ in den])
        for _ in range(rounds)
    )
    tail = (1 - level) / 2
    return point, [draws[int(tail * rounds)], draws[min(rounds - 1, int((1 - tail) * rounds))]]


def round_ratio_ci(numerator, denominator, paired, rounds=4000, seed=24731, level=0.95):
    """Percentile bootstrap interval of median(numerator) / median(denominator) over per-round observations. `paired`:
    the two lists are the same rounds (index i of each ran in round i), so round indices are resampled with one draw
    for both; otherwise each list is resampled on its own."""
    import numpy as np
    num, den = np.asarray(numerator, dtype=float), np.asarray(denominator, dtype=float)
    if paired and len(num) != len(den):
        raise ValueError(f"paired rounds need the same count: {len(num)} against {len(den)}")
    rng = np.random.default_rng(seed)
    pick = rng.integers(0, len(num), size=(rounds, len(num)))
    other = pick if paired else rng.integers(0, len(den), size=(rounds, len(den)))
    draws = np.sort(np.median(num[pick], axis=1) / np.median(den[other], axis=1))
    tail = (1 - level) / 2
    return [float(draws[int(tail * rounds)]), float(draws[min(rounds - 1, int((1 - tail) * rounds))])]


def paired_bootstrap(first, second, rounds=2000, seed=24731, level=0.95):
    """Paired per-query bootstrap of two systems scored on the same queries: queries are resampled with replacement, the
    same draw for both systems. Returns the means, mean(first) - mean(second) with its percentile interval, and
    mean(first) / mean(second) with its interval (None when a resampled mean of `second` is not positive)."""
    first, second = list(first), list(second)
    if len(first) != len(second):
        raise ValueError(f"paired scores need the same queries: {len(first)} against {len(second)}")
    n = len(first)
    if n == 0:
        raise ValueError("paired scores need at least one query")
    rng = random.Random(seed)
    population = range(n)
    differences, ratios = [], []
    for _ in range(rounds):
        draw = rng.choices(population, k=n)
        a, b = sum(map(first.__getitem__, draw)), sum(map(second.__getitem__, draw))
        differences.append((a - b) / n)
        ratios.append(a / b if b > 0 else math.nan)
    tail = (1 - level) / 2
    lo, hi = int(tail * rounds), min(rounds - 1, int((1 - tail) * rounds))
    differences.sort()
    ratio_interval = None if any(r != r for r in ratios) else [sorted(ratios)[lo], sorted(ratios)[hi]]
    mean_first, mean_second = sum(first) / n, sum(second) / n
    return {"means": [mean_first, mean_second], "difference": mean_first - mean_second, "difference_ci": [differences[lo], differences[hi]],
            "ratio": mean_first / mean_second if mean_second > 0 else None, "ratio_ci": ratio_interval, "queries": n}


def ndcg_at(found, relevant, k=10):
    gain = sum(relevant.get(key, 0) / math.log2(rank + 2) for rank, key in enumerate(found[:k]))
    ideal = sum(g / math.log2(rank + 2) for rank, g in enumerate(sorted(relevant.values(), reverse=True)[:k]))
    return gain / ideal if ideal else 0.0


def recall_at(found, relevant, k):
    positive = {key for key, grade in relevant.items() if grade > 0}
    return len(set(found[:k]) & positive) / len(positive) if positive else 0.0


def max_rss_bytes(ru):
    """`ru_maxrss` is bytes on macOS and kilobytes on Linux."""
    return ru.ru_maxrss if platform.system() == "Darwin" else ru.ru_maxrss * 1024


def self_peak_rss_bytes():
    return max_rss_bytes(resource.getrusage(resource.RUSAGE_SELF))


def current_rss_bytes(pid=None):
    pid = pid or os.getpid()
    try:
        if platform.system() == "Linux":
            return int(Path(f"/proc/{pid}/statm").read_text().split()[1]) * os.sysconf("SC_PAGE_SIZE")
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True, check=True).stdout
        return int(out.strip()) * 1024
    except (OSError, ValueError, subprocess.CalledProcessError):
        return 0


class _Spawner:
    """Client of `_spawner.py`: runs measured commands from a small process, so each child's
    `ru_maxrss` is its own rather than at least the orchestrator's RSS (see that module)."""

    def __init__(self):
        self.proc = subprocess.Popen([sys.executable, "-I", "-S", str(Path(__file__).with_name("_spawner.py"))],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.pending = b""

    def _line(self, deadline=None):
        fd = self.proc.stdout.fileno()
        while b"\n" not in self.pending:
            wait = None if deadline is None else max(0.0, deadline - time.monotonic())
            if not select.select([fd], [], [], wait)[0]:
                return None
            chunk = os.read(fd, 4096)
            if not chunk:
                raise RuntimeError("measurement helper exited")
            self.pending += chunk
        line, _, self.pending = self.pending.partition(b"\n")
        return line.decode()

    def run(self, job, timeout):
        deadline = time.monotonic() + timeout
        self.proc.stdin.write((json.dumps(job) + "\n").encode())
        self.proc.stdin.flush()
        pid = int(self._line().split()[1])
        line = self._line(deadline)
        if line is None:
            os.kill(pid, signal.SIGKILL)
            self._line()
            raise TimeoutError(f"{job['argv'][0]} exceeded {timeout}s")
        _, code, peak, cpu = line.split()
        global LAST_CPU_S
        LAST_CPU_S = float(cpu)
        return int(code), int(peak)
_SPAWNER = None
LAST_CPU_S = None
LAST_LOAD = None  # CPU accounting of the last measured command: see _load_record


def _cpu_snapshot():
    try:
        cores = {}
        for line in Path("/proc/stat").read_text().splitlines():
            if line.startswith("cpu"):
                name, *v = line.split()
                v = [int(x) for x in v]
                cores[name] = (v[0] + v[1] + v[2] + v[5] + v[6], v[7] if len(v) > 7 else 0)  # user nice system irq softirq; steal
        return {"t": time.perf_counter(), "cores": cores, "loadavg": Path("/proc/loadavg").read_text().split()[:3]}
    except OSError:
        return None


def _load_record(pre, post, cpu_s):
    """What ran on the machine besides the measured child during its window, from /proc/stat deltas.
    Busy time = user + nice + system + irq + softirq; the child's own CPU (wait4 rusage, descendants included) is subtracted."""
    hz = os.sysconf("SC_CLK_TCK")
    window = post["t"] - pre["t"]
    ncpu = len(pre["cores"]) - 1
    busy = {c: (post["cores"][c][0] - pre["cores"][c][0]) / hz for c in pre["cores"]}
    steal = (post["cores"]["cpu"][1] - pre["cores"]["cpu"][1]) / hz
    return {"window_s": window, "child_cpu_s": cpu_s, "loadavg_before": pre["loadavg"], "loadavg_after": post["loadavg"],
            "foreign_machine_pct_of_capacity": max(0.0, busy["cpu"] - cpu_s) / (window * ncpu) * 100,
            "foreign_machine_pct_of_one_core": max(0.0, busy["cpu"] - cpu_s) / window * 100,
            "core_busy_s": {c: v for c, v in busy.items() if c != "cpu"}, "steal_s": steal}


# Cold-start clocks start when the child is about to be spawned: after the timing lock and load gate are
# held. A command element containing T0_PLACEHOLDER gets that instant (ns since the epoch) substituted;
# the same instant is left in LAST_T0_NS for callers that compute a start-to-event interval themselves.
T0_PLACEHOLDER = "@T0_NS@"
LAST_T0_NS = None


def run_measured(command, env=None, timeout=3600, cwd=None):
    """Run `command`; return (stdout, stderr, wall_s, peak_rss_bytes) of that child alone: its
    own wait4 rusage, from a small helper on Linux (see `_Spawner`) and directly elsewhere.
    Raises on a non-zero exit with the stderr tail."""
    global _SPAWNER
    with timing_lock(), tempfile.TemporaryDirectory() as tmp:
        out_path, err_path = Path(tmp) / "out", Path(tmp) / "err"
        full_env = {**os.environ, **(env or {})}
        global LAST_T0_NS
        LAST_T0_NS = time.time_ns()
        command = [str(c).replace(T0_PLACEHOLDER, str(LAST_T0_NS)) for c in command]
        global LAST_LOAD
        pre = _cpu_snapshot()
        start = time.perf_counter()
        if platform.system() == "Linux":
            if _SPAWNER is None or _SPAWNER.proc.poll() is not None:
                _SPAWNER = _Spawner()
            out_path.touch()
            err_path.touch()
            code, peak = _SPAWNER.run({"argv": [str(c) for c in command], "env": full_env, "cwd": str(cwd) if cwd else "",
                                       "stdout": str(out_path), "stderr": str(err_path)}, timeout)
        else:
            with out_path.open("w") as out, err_path.open("w") as err:
                proc = subprocess.Popen(command, stdout=out, stderr=err, env=full_env, cwd=cwd)
                deadline = time.monotonic() + timeout
                while True:
                    pid, status, ru = os.wait4(proc.pid, os.WNOHANG)
                    if pid:
                        break
                    if time.monotonic() > deadline:
                        proc.kill()
                        os.wait4(proc.pid, 0)
                        raise TimeoutError(f"{command[0]} exceeded {timeout}s")
                    time.sleep(0.05)
            code, peak = os.waitstatus_to_exitcode(status), max_rss_bytes(ru)
        wall = time.perf_counter() - start
        post = _cpu_snapshot()
        LAST_LOAD = _load_record(pre, post, LAST_CPU_S) if pre and post and platform.system() == "Linux" and LAST_CPU_S is not None else None
        stdout, stderr = out_path.read_text(errors="replace"), err_path.read_text(errors="replace")
    if code:
        raise RuntimeError(f"{command[0]} exited {code}: {stderr[-2000:]}")
    return stdout, stderr, wall, peak


@contextlib.contextmanager
def timing_lock():
    """Exclusive timing lock shared with the other harnesses on this host (one phase at a time,
    never held across a suite, so each hold stays far below the 14 minute rule). A no-op on CI
    and wherever the lock directory does not exist."""
    path = Path(os.environ.get("WARM_BENCH_LOCK", "./.warm-target/timing.lock"))
    if os.environ.get("GITHUB_ACTIONS") or not path.parent.exists():
        yield
        return
    with path.open("a") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        try:
            deadline = time.monotonic() + float(os.environ.get("WARM_BENCH_LOAD_WAIT_S", "0"))
            while os.getloadavg()[0] >= 2 and time.monotonic() < deadline:
                time.sleep(5)
            yield
        finally:
            fcntl.flock(handle, fcntl.LOCK_UN)


_BLOCK = 4096


def _data_bytes(path):
    """Bytes in 4 KiB blocks that hold a non-zero byte: what a sparse-capable filesystem would allocate."""
    zero = bytes(_BLOCK)
    total = 0
    with open(path, "rb") as handle:
        while chunk := handle.read(1 << 20):
            total += sum(len(chunk[i:i + _BLOCK]) for i in range(0, len(chunk), _BLOCK) if chunk[i:i + _BLOCK] != zero)
    return total


_SPARSE_MISREPORTED = {}


def _misreports_sparse(directory):
    """True when the filesystem of `directory` reports a freshly truncated (hole-only) file as fully allocated."""
    dev = os.stat(directory).st_dev
    if dev not in _SPARSE_MISREPORTED:
        try:
            with tempfile.TemporaryFile(dir=directory) as probe:
                probe.truncate(1 << 20)
                probe.flush()
                st = os.fstat(probe.fileno())
                _SPARSE_MISREPORTED[dev] = st.st_blocks * 512 >= st.st_size
        except OSError:
            _SPARSE_MISREPORTED[dev] = False
    return _SPARSE_MISREPORTED[dev]


def dir_bytes(path):
    """Allocated bytes (du semantics): sparse or preallocated files count what they occupy.
    Some sandboxes (gVisor) report every file as fully allocated, sparse ones included; on such
    a filesystem only the non-zero 4 KiB blocks of a file of 1 MiB or more are counted."""
    path = Path(path)
    files = [path] if path.is_file() else [p for p in path.rglob("*") if p.is_file()]
    scan = bool(files) and _misreports_sparse(path.parent if path.is_file() else path)
    total = 0
    for p in files:
        st = p.stat()
        allocated = min(st.st_size, st.st_blocks * 512)
        if scan and allocated >= st.st_size >= 1 << 20:
            allocated = _data_bytes(p)
        total += allocated
    return total


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")
    tmp.replace(path)


def log(message):
    print(message, file=sys.stderr, flush=True)
