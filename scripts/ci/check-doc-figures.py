# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Fail when README or benchmark prose quotes a figure no committed result contains.

Prose in README.md and docs/benchmarks/*.md (the generated COMPARISON.md, GAPS.md and any text
between <!-- BENCH:START --> and <!-- BENCH:END --> excluded) is scanned for benchmark figures:
a number written next to QPS, MiB, GiB, a ratio suffix (2.93x), or after recall / nDCG / MRR.
Each must equal, at the precision it is written with, a number in the committed generated data
(docs/benchmarks/comparison.cells.json, COMPARISON.md, GAPS.md) or in a file that the same table
row or paragraph links to (a result file, or every file under a linked directory). Numbers with
no unit, years and versions are not figures.

README.md, EVALUATION.md and ANN-BENCHMARKS.md are held to this fully. The older research pages
in docs/benchmarks quote many derived ratios; the ones that were unsupported when this check was
added are listed in doc-figures-baseline.txt. A new unsupported figure fails, and so does a
baseline entry that has become supported or disappeared, so the list can only shrink.

usage: check-doc-figures.py [--root DIR] [--write-baseline]
"""
import argparse
import bisect
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
NUM = r"\d[\d,]*(?:\.\d+)?"
TOKEN = re.compile(r"(?<![\w.])-?" + NUM)
UNIT_AFTER = re.compile(
    r"(?<![\w.@])(" + NUM + r")(?:\s?(?:x|QPS|qps|MiB|GiB)(?![A-Za-z0-9])|\s(?:QPS|MiB|GiB)\b)"
)
UNIT_BEFORE = re.compile(
    r"(?:recall(?:@\d+)?|nDCG(?:@\d+)?|MRR(?:@\d+)?)\s*(?:of|is|was|to|=|:)?\s*\**(" + r"\d?\.\d+" + r")"
)
GENERATED_DOCS = {"COMPARISON.md", "GAPS.md"}
STRICT = {"README.md", "docs/benchmarks/EVALUATION.md", "docs/benchmarks/ANN-BENCHMARKS.md"}
BASELINE = "scripts/ci/doc-figures-baseline.txt"
CORPUS_SUFFIXES = {".json", ".jsonl", ".txt", ".csv", ".md", ".log"}
LINK = re.compile(r"\]\(([^)#\s]+)(?:#[^)]*)?\)")
MAX_FILE = 64 * 1024 * 1024


def strip_generated(text: str) -> str:
    return re.sub(r"<!-- BENCH:START -->.*?<!-- BENCH:END -->", "", text, flags=re.S)


def strip_noise(text: str) -> str:
    text = re.sub(r"```.*?```", "", text, flags=re.S)
    text = re.sub(r"\]\([^)]*\)", "]", text)
    text = re.sub(r"https?://\S+", "", text)
    return re.sub(r"`[^`]*`", lambda m: m.group(0) if re.search(NUM, m.group(0)) and re.search("QPS|recall", m.group(0)) else "", text)


def numbers(text: str):
    for match in TOKEN.finditer(text):
        raw = match.group(0).replace(",", "")
        try:
            yield float(raw)
        except ValueError:
            continue


def tracked(directory: pathlib.Path) -> list:
    """Files under a directory that git tracks, so a local run sees what CI sees."""
    try:
        listing = subprocess.run(
            ["git", "ls-files", "-z", "--", "."], cwd=directory, capture_output=True, check=True
        ).stdout.decode()
    except (OSError, subprocess.CalledProcessError):
        return [p for p in directory.rglob("*") if p.is_file()]
    return [directory / name for name in listing.split("\0") if name]


def file_numbers(path: pathlib.Path, cache: dict) -> list:
    if path not in cache:
        values = []
        files = [path] if path.is_file() else tracked(path) if path.is_dir() else []
        for item in files[:5000]:
            try:
                if item.suffix in CORPUS_SUFFIXES and item.stat().st_size <= MAX_FILE:
                    values.extend(numbers(item.read_text(errors="replace")))
            except OSError:
                continue
        cache[path] = values
    return cache[path]


def generated_corpus(root: pathlib.Path, cache: dict) -> list:
    values = []
    for name in ("comparison.cells.json", *GENERATED_DOCS):
        values.extend(file_numbers(root / "docs/benchmarks" / name, cache))
    return values


def blocks(text: str):
    """Yield (first line number, text) for each table row and each paragraph."""
    paragraph, start = [], 0
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("|"):
            if paragraph:
                yield start, "\n".join(paragraph)
                paragraph = []
            yield number, line
        elif line.strip():
            if not paragraph:
                start = number
            paragraph.append(line)
        elif paragraph:
            yield start, "\n".join(paragraph)
            paragraph = []
    if paragraph:
        yield start, "\n".join(paragraph)


SCALE = {"MiB": 2**20, "GiB": 2**30}


def present(values: list, written: str, percent: bool, unit: str = "") -> bool:
    raw = written.replace(",", "")
    decimals = len(raw.split(".")[1]) if "." in raw else 0
    target = float(raw)
    tolerance = 0.5 * 10 ** -decimals + 1e-9
    candidates = [(target, tolerance)]
    if percent:
        candidates.append((target / 100, tolerance / 100))
    if unit in SCALE:
        candidates.append((target * SCALE[unit], tolerance * SCALE[unit]))
    for goal, tol in candidates:
        low = bisect.bisect_left(values, goal - tol)
        if low < len(values) and values[low] <= goal + tol:
            return True
    return False


def figures(text: str):
    for number in UNIT_AFTER.finditer(text):
        yield number.group(1), number.group(0).rstrip().endswith("%"), number.group(0)
    for number in UNIT_BEFORE.finditer(text):
        yield number.group(1), False, number.group(0)


def prose_files(root: pathlib.Path):
    yield root / "README.md"
    for path in sorted((root / "docs/benchmarks").glob("*.md")):
        if path.name not in GENERATED_DOCS:
            yield path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=ROOT)
    parser.add_argument("--write-baseline", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    cache: dict = {}
    generated = generated_corpus(root, cache)
    unsupported = []
    checked = 0
    for path in prose_files(root):
        relative = str(path.relative_to(root))
        source = strip_generated(path.read_text())
        for number, block in blocks(source):
            values = list(generated)
            for target in LINK.findall(block):
                if "://" in target:
                    continue
                values.extend(file_numbers((path.parent / target).resolve(), cache))
            values.sort()
            for written, percent, context in figures(strip_noise(block)):
                checked += 1
                if not present(values, written, percent, next((u for u in SCALE if context.rstrip().endswith(u)), "")):
                    unsupported.append((relative, number, " ".join(context.split())))
    baseline_path = root / BASELINE
    if args.write_baseline:
        entries = sorted({f"{f}|{c}" for f, _, c in unsupported if f not in STRICT})
        baseline_path.write_text("\n".join(entries) + "\n")
        print(f"wrote {len(entries)} baseline entries")
        return 0
    baseline = set(baseline_path.read_text().splitlines()) if baseline_path.exists() else set()
    seen = set()
    problems = []
    for relative, number, context in unsupported:
        key = f"{relative}|{context}"
        seen.add(key)
        if relative in STRICT or key not in baseline:
            problems.append(
                f"{relative}:{number}: {context!r} is in no generated table and no file linked from that row or paragraph"
            )
    problems += [f"{BASELINE}: stale entry {key!r}" for key in sorted(baseline - seen)]
    print(f"checked {checked} figures; {len(problems)} problems; {len(baseline & seen)} baselined")
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
