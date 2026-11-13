# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check the portable JavaScript/Python SDK release independently of the engine.

usage: check-sdk-versions.py [--tag sdk-v0.2.0]
"""
import argparse
import json
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
JS = "bindings/js/warm-sdk"
PYTHON = "bindings/python/warm-sdk/pyproject.toml"
NUMBER = r"(?:0|[1-9][0-9]*)"
PRERELEASE = rf"(?:{NUMBER}|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
SEMVER = re.compile(
    rf"{NUMBER}\.{NUMBER}\.{NUMBER}(?:-{PRERELEASE}(?:\.{PRERELEASE})*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def read_manifest(relative: str, problems: list[str]) -> dict:
    try:
        text = (ROOT / relative).read_text(encoding="utf-8")
        value = tomllib.loads(text) if relative.endswith(".toml") else json.loads(text)
        if not isinstance(value, dict):
            raise ValueError
        return value
    except (OSError, UnicodeError, ValueError):
        problems.append(f"{relative} must be a readable manifest object")
        return {}


def object_field(value: dict, field: str) -> dict:
    result = value.get(field)
    return result if isinstance(result, dict) else {}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag")
    args = parser.parse_args()
    problems: list[str] = []
    package = read_manifest(f"{JS}/package.json", problems)
    lock = read_manifest(f"{JS}/package-lock.json", problems)
    locked_root = object_field(object_field(lock, "packages"), "")
    python = object_field(read_manifest(PYTHON, problems), "project")
    version = package.get("version")
    if not isinstance(version, str) or len(version) > 128 or not SEMVER.fullmatch(version):
        problems.append(f"{JS}/package.json must declare a semantic version")
        version = None
    entries = (
        (f"{JS}/package.json", package),
        (f"{JS}/package-lock.json", lock),
        (f"{JS}/package-lock.json root package", locked_root),
        (PYTHON, python),
    )
    for label, entry in entries:
        if entry.get("name") != "warm-sdk":
            problems.append(f"{label} must be named warm-sdk")
        if not isinstance(entry.get("version"), str) or entry.get("version") != version:
            problems.append(f"{label} version disagrees with the portable JavaScript SDK")
    for label, entry in entries[:1] + entries[2:3]:
        for field in ("dependencies", "optionalDependencies", "peerDependencies"):
            dependencies = entry.get(field, {})
            if not isinstance(dependencies, dict) or dependencies:
                problems.append(f"{label} must not declare {field}")
        for field in ("bundleDependencies", "bundledDependencies"):
            if field in entry and entry[field] is not False and entry[field] != []:
                problems.append(f"{label} must not bundle dependencies")
        scripts = entry.get("scripts", {})
        if not isinstance(scripts, dict):
            problems.append(f"{label} scripts must be an object")
        elif any(hook in scripts for hook in ("preinstall", "install", "postinstall")):
            problems.append(f"{label} must not define install lifecycle hooks")
        if entry.get("hasInstallScript", False) is not False:
            problems.append(f"{label} must not require an install script")
    if args.tag is not None and args.tag.removeprefix("refs/tags/") != f"sdk-v{version}":
        problems.append("tag does not name the portable SDK version with the sdk-v prefix")
    print(json.dumps({"version": version, "problems": len(problems)}))
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
