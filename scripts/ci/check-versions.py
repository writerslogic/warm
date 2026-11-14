# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Fail unless the crate, wheel, npm packages and OpenAPI agree on one version.

usage: check-versions.py [--tag v0.7.0 | --tag py-v0.7.0]

Cargo.toml is the source of truth (maturin reads the wheel version from it). package.json,
package-lock.json, every optionalDependency and OpenAPI info.version must agree with it,
and a release tag must name it.
"""
import argparse
import json
import pathlib
import re
import sys

import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
NATIVE_NAME = "writerslogic-warm"
PLATFORMS = {
    "aarch64-apple-darwin": "darwin-arm64",
    "x86_64-apple-darwin": "darwin-x64",
    "aarch64-unknown-linux-gnu": "linux-arm64-gnu",
    "aarch64-unknown-linux-musl": "linux-arm64-musl",
    "x86_64-unknown-linux-gnu": "linux-x64-gnu",
    "x86_64-unknown-linux-musl": "linux-x64-musl",
    "aarch64-pc-windows-msvc": "win32-arm64-msvc",
    "x86_64-pc-windows-msvc": "win32-x64-msvc",
}


DOC_SKIP_DIRS = ("docs/research/", "docs/migrations/", "docs/integrations/frameworks/")
DOC_VERSION = re.compile(r"(?<![\w@/.-])(%s)(?:`)?(?:@|==|\s+)v?(\d+\.\d+\.\d+)(?![\w.-])")


def doc_files() -> list[pathlib.Path]:
    files = [ROOT / "README.md", ROOT / "CONTRIBUTING.md"]
    return files + sorted(
        path for path in (ROOT / "docs").rglob("*.md")
        if not str(path.relative_to(ROOT)).startswith(DOC_SKIP_DIRS)
        and path.name not in ("COMPARISON.md", "GAPS.md")
    )


def doc_versions(cargo: str) -> list[str]:
    """Current-version statements in README, CITATION.cff, CONTRIBUTING and docs/*.md must match the manifests."""
    sdk_js = json.loads((ROOT / "bindings/js/warm-sdk/package.json").read_text())["version"]
    sdk_py = tomllib.loads((ROOT / "bindings/python/warm-sdk/pyproject.toml").read_text())["project"]["version"]
    mcp_npm = json.loads((ROOT / "integrations/mcp/npm/package.json").read_text())["version"]
    mcp_py = tomllib.loads((ROOT / "integrations/mcp/pypi/pyproject.toml").read_text())["project"]["version"]
    models = tomllib.loads((ROOT / "bindings/embed-models/python/pyproject.toml").read_text())["project"]["version"]
    problems = []
    if sdk_js != sdk_py:
        problems.append(f"warm-sdk is {sdk_js} on npm and {sdk_py} on PyPI")
    if mcp_npm != mcp_py:
        problems.append(f"MCP launcher is {mcp_npm} on npm and {mcp_py} on PyPI")
    current = {
        "warm-rust": cargo,
        "warm-python": cargo,
        "writerslogic-warm": cargo,
        "warm-sdk": sdk_js,
        "writerslogic-warm-mcp": mcp_npm,
        "warm-mcp": mcp_py,
        "warm-models": models,
    }
    pattern = re.compile(DOC_VERSION.pattern % "|".join(sorted(map(re.escape, current), key=len, reverse=True)))
    citation = re.search(r"^version:\s*(\S+)", (ROOT / "CITATION.cff").read_text(), re.M)
    if not citation or citation.group(1) != cargo:
        problems.append(f"CITATION.cff is {citation and citation.group(1)}, Cargo.toml is {cargo}")
    rust = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["rust-version"]
    for name in ("README.md", "CONTRIBUTING.md"):
        if f"Rust {rust}" not in (ROOT / name).read_text():
            problems.append(f"{name} does not state Rust {rust}, the Cargo.toml rust-version")
    for path in doc_files():
        lines = path.read_text().splitlines()
        for number, line in enumerate(lines, 1):
            context = " ".join(lines[max(0, number - 3):number])
            if re.search(r"historical|earlier|previous|superseded|\bpartially published\b", context, re.I):
                continue
            for name, stated in pattern.findall(line):
                if stated != current[name]:
                    problems.append(f"{path.relative_to(ROOT)}:{number}: {name} {stated}, manifest says {current[name]}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag")
    args = parser.parse_args()

    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    package = json.loads((ROOT / "package.json").read_text())
    lock = json.loads((ROOT / "package-lock.json").read_text())
    openapi = json.loads((ROOT / "docs/server/openapi.json").read_text())
    native = tomllib.loads((ROOT / "pyproject.toml").read_text())["project"]
    models = tomllib.loads((ROOT / "bindings/embed-models/python/pyproject.toml").read_text())["project"]
    problems = []
    if native["name"] != "warm-python":
        problems.append(f"native Python distribution is {native['name']}")
    if models["name"] != "warm-models":
        problems.append(f"embedding model distribution is {models['name']}")
    if native.get("optional-dependencies", {}).get("embed") != [
        f"warm-models=={models['version']}"
    ]:
        problems.append("native Python embed extra disagrees with the model package")
    if package.get("name") != NATIVE_NAME:
        problems.append(f"native package.json must be named {NATIVE_NAME}")
    napi = package.get("napi", {})
    if not isinstance(napi, dict):
        napi = {}
    if napi.get("binaryName") != NATIVE_NAME:
        problems.append(f"native napi.binaryName must be {NATIVE_NAME}")
    targets = napi.get("targets", [])
    if not isinstance(targets, list) or not targets or any(
        not isinstance(target, str) or target not in PLATFORMS for target in targets
    ) or len(set(targets)) != len(targets):
        problems.append("native napi.targets must contain unique supported platform targets")
    else:
        expected = {f"{NATIVE_NAME}-{PLATFORMS[target]}" for target in targets}
        if set(package.get("optionalDependencies", {})) != expected:
            problems.append("native optionalDependencies disagree with the package family and napi.targets")
    if package["version"] != cargo:
        problems.append(f"package.json is {package['version']}, Cargo.toml is {cargo}")
    locked_packages = lock.get("packages", {})
    locked_root = locked_packages.get("", {})
    for label, entry in (("package-lock.json", lock), ("package-lock.json root package", locked_root)):
        if entry.get("name") != NATIVE_NAME:
            problems.append(f"{label} must be named {NATIVE_NAME}")
        if entry.get("version") != cargo:
            problems.append(f"{label} is {entry.get('version')}, Cargo.toml is {cargo}")
    if locked_root.get("optionalDependencies", {}) != package.get("optionalDependencies", {}):
        problems.append("package-lock.json root optionalDependencies disagree with package.json")
    if openapi["info"]["version"] != cargo:
        problems.append(f"docs/server/openapi.json is {openapi['info']['version']}, Cargo.toml is {cargo}")
    for name, version in package.get("optionalDependencies", {}).items():
        if version != cargo:
            problems.append(f"optionalDependency {name} is {version}, Cargo.toml is {cargo}")
        locked_version = locked_packages.get(f"node_modules/{name}", {}).get("version")
        if locked_version != cargo:
            problems.append(f"package-lock.json {name} is {locked_version}, Cargo.toml is {cargo}")
        locked_name = locked_packages.get(f"node_modules/{name}", {}).get("name", name)
        if locked_name != name:
            problems.append(f"package-lock.json {name} names a different native package")
    if args.tag:
        tag_version = args.tag.removeprefix("refs/tags/").removeprefix("py-").removeprefix("v")
        if tag_version != cargo:
            problems.append(f"tag {args.tag} does not name version {cargo}")
    problems += doc_versions(cargo)
    print(json.dumps({"version": cargo, "problems": len(problems)}))
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
